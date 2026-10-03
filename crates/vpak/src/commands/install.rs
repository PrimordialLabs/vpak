//! `vpak install` and `vpak resume`: the install driver. One runner turn per
//! phase, read back the phase the runner recorded, stop on `needs_human`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use is_terminal::IsTerminal;
use serde::Serialize;
use vpak_core::install::{audit, journal, questions, InitOptions, Mode, Phase, Status, Workdir};
use vpak_core::runner::{self, Config, RunRequest, Runner};
use vpak_core::{bootstrap, util};

use crate::cli::InstallArgs;
use crate::commands::ask;
use crate::out::{self, EXIT_NEEDS_HUMAN, EXIT_OK};

pub fn run(a: InstallArgs, workdir: Option<&Path>, json: bool) -> Result<i32> {
    let file = util::canonical(&a.file).with_context(|| format!("archive {}", a.file.display()))?;
    let digest = format!("sha256:{}", util::sha256_file(&file)?);
    let config = Config::load()?;

    // Resume if the workdir (explicit or discovered) already holds this install.
    if let Some(wd) = workdir {
        if wd.join(vpak_core::install::INSTALL_FILE).is_file() {
            let wd = Workdir::open(wd)?;
            out::note(&format!(
                "resuming install {} in {}",
                wd.state()?.id,
                wd.root().display()
            ));
            return if a.print_bootstrap {
                print_bootstrap(&wd)
            } else {
                drive(&wd, a.max_phases, json)
            };
        }
    } else {
        let found = Workdir::find_resumable(&std::env::current_dir()?, &digest)?;
        if found.len() == 1 {
            let wd = Workdir::open(&found[0])?;
            out::note(&format!(
                "resuming install {} in {}",
                wd.state()?.id,
                wd.root().display()
            ));
            return if a.print_bootstrap {
                print_bootstrap(&wd)
            } else {
                drive(&wd, a.max_phases, json)
            };
        } else if found.len() > 1 {
            bail!("several unfinished installs of this archive under .vpak/; pass --workdir to pick one:\n  {}", found.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n  "));
        }
    }

    let (manifest, _) = vpak_core::archive::list(&file)?;
    let id = util::new_id("vp");
    let wd_path = workdir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".vpak").join(&id));
    let dest = a
        .dest
        .clone()
        .unwrap_or_else(|| PathBuf::from(&manifest.name));
    let mode = match &a.mode {
        Some(m) => m.parse::<Mode>()?,
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                Mode::Interactive
            } else {
                Mode::Auto
            }
        }
    };
    let runner_name = a
        .runner
        .clone()
        .or(config.default_runner.clone())
        .unwrap_or_else(|| "claude".into());
    let target_text = match &a.target {
        Some(t) => Some(util::resolve_text_arg(t)?),
        None => None,
    };
    let constraints_text = match &a.constraints {
        Some(c) => Some(util::resolve_text_arg(c)?),
        None => None,
    };

    let (wd, manifest) = Workdir::init(InitOptions {
        id: Some(id),
        vpak_path: file,
        workdir: wd_path,
        dest,
        runner: runner_name,
        mode,
        target_text,
        constraints_text,
    })?;
    out::note(&format!(
        "install {} of {} {} in {}",
        wd.state()?.id,
        manifest.name,
        manifest.version,
        wd.root().display()
    ));

    if a.print_bootstrap {
        return print_bootstrap(&wd);
    }
    drive(&wd, a.max_phases, json)
}

pub fn resume(workdir: &Path, max_phases: u32, json: bool) -> Result<i32> {
    let wd = Workdir::open(workdir)?;
    drive(&wd, max_phases, json)
}

fn print_bootstrap(wd: &Workdir) -> Result<i32> {
    let text = bootstrap::compile(wd)?;
    println!("{text}");
    eprintln!("\nvpak: workdir {}", wd.root().display());
    eprintln!(
        "vpak: drive the install with the primitives above; set VPAK_WORKDIR or pass --workdir."
    );
    Ok(EXIT_OK)
}

#[derive(Serialize)]
struct DriveReport {
    id: String,
    phase: Phase,
    status: Status,
    turns: u32,
    open_questions: usize,
    workdir: PathBuf,
}

/// Loop: one runner turn per phase until done, needs_human, failure, or the
/// turn budget. Interactive mode answers open questions between turns.
fn drive(wd: &Workdir, max_phases: u32, json: bool) -> Result<i32> {
    let config = Config::load()?;
    let st0 = wd.state()?;
    let runner: Box<dyn Runner> = runner::by_name(&st0.runner, &config)?;
    match runner.available()? {
        runner::Availability::Available { detail } => {
            out::note(&format!("runner {}: {detail}", runner.name()))
        }
        runner::Availability::Unavailable { reason } => {
            eprintln!("vpak: runner '{}' is not available: {reason}", st0.runner);
            for (n, av) in runner::survey(&config) {
                if let runner::Availability::Available { detail } = av {
                    eprintln!("  available: {n} ({detail})");
                }
            }
            bail!("pick a runner with --runner or install one");
        }
    }

    let mut turns = 0u32;
    let code = loop {
        let mut st = wd.state()?;
        if st.status == Status::Done || st.phase.is_terminal() {
            out::note(&format!("install {} complete", st.id));
            break EXIT_OK;
        }
        if st.status == Status::Failed {
            out::note(&format!(
                "install {} failed; see {}",
                st.id,
                wd.audit_file().display()
            ));
            break 1;
        }

        // Open questions: answer them here in interactive mode, stop in auto mode.
        let open = questions::open(wd)?;
        if !open.is_empty() {
            if st.mode == Mode::Interactive && ask::console_available() {
                for q in &open {
                    let answer = ask::prompt_console(q)?;
                    questions::answer(wd, &q.id, &answer, &util::identity())?;
                    audit::record(
                        wd,
                        &util::identity(),
                        "question.answer",
                        serde_json::json!({"id": q.id}),
                        "ok",
                    )?;
                }
                wd.set_status(Status::Pending, "vpak", "questions answered")?;
                st = wd.state()?;
            } else {
                wd.set_status(
                    Status::NeedsHuman,
                    "vpak",
                    &format!("{} open question(s)", open.len()),
                )?;
                print_questions(&open, wd);
                break EXIT_NEEDS_HUMAN;
            }
        }

        if turns >= max_phases {
            out::note(&format!(
                "stopping after {turns} runner turn(s); resume with: vpak resume {}",
                wd.root().display()
            ));
            break EXIT_OK;
        }
        turns += 1;

        let phase = st.phase;
        let system = bootstrap::compile(wd)?;
        let _ = system;
        let prompt = bootstrap::phase_prompt(wd, phase)?;
        let policy = wd.policy()?;
        let req = RunRequest {
            system_prompt_file: wd.compiled_file(),
            prompt,
            cwd: wd.root().to_path_buf(),
            extra_dirs: vec![st.dest.clone()],
            budget: policy.budget.clone(),
            policy,
            session: st.session.clone(),
            env: {
                let mut env = runner::self_env();
                env.extend([
                    ("VPAK_WORKDIR".into(), wd.root().display().to_string()),
                    ("VPAK_MODE".into(), st.mode.as_str().to_string()),
                    ("VPAK_PHASE".into(), phase.as_str().to_string()),
                    ("VPAK_INSTALL_ID".into(), st.id.clone()),
                    ("VPAK_ACTOR".into(), format!("runner:{}", runner.name())),
                ]);
                env
            },
        };
        std::fs::create_dir_all(&st.dest).ok();
        wd.set_status(
            Status::Running,
            "vpak",
            &format!("runner turn {turns} for {phase}"),
        )?;
        audit::record(
            wd,
            "vpak",
            "runner.start",
            serde_json::json!({"runner": runner.name(), "phase": phase, "turn": turns}),
            "ok",
        )?;
        out::note(&format!(
            "turn {turns}: phase {phase} via {}",
            runner.name()
        ));

        let actor = format!("runner:{}", runner.name());
        let outcome = runner.run(&req, &mut |ev| {
            if !json {
                eprintln!("  [{}] {}", ev.kind, ev.summary);
            }
            let _ = audit::record(
                wd,
                &actor,
                &format!("runner.{}", ev.kind),
                serde_json::json!({"summary": ev.summary}),
                "",
            );
        })?;
        audit::record(
            wd,
            "vpak",
            "runner.end",
            serde_json::json!({"exit": outcome.exit_code, "timed_out": outcome.timed_out, "turns": outcome.num_turns, "cost_usd": outcome.cost_usd, "denials": outcome.denials.len(), "is_error": outcome.is_error, "stderr": util::truncate(&outcome.stderr_tail, 400)}),
            if outcome.is_error { "error" } else { "ok" },
        )?;

        let mut after = wd.state()?;
        if let Some(s) = &outcome.session_id {
            after.session = Some(s.clone());
        }
        for d in &outcome.denials {
            let q = questions::ask(wd, &format!("The runner policy blocked `{}`: {}. Allow it, or tell the bootstrap how to proceed without it.", d.tool, d.detail), &[], questions::ApplyAs::Note, None, "vpak")?;
            audit::record(
                wd,
                "vpak",
                "question.ask",
                serde_json::json!({"id": q.id, "from": "denial"}),
                "ok",
            )?;
        }
        if outcome.timed_out {
            let mut j = journal::Journal::load(wd)?;
            j.append(wd, phase, "runner timed out", &format!("The {} runner hit the wall-clock budget ({}) during phase {phase} and was stopped.", runner.name(), req.budget.wall_clock), "vpak")?;
        }

        if after.phase == phase && after.status != Status::Done {
            after.stalls += 1;
            wd.save_state(&after)?;
            if after.stalls >= 2 && questions::open(wd)?.is_empty() {
                let q = questions::ask(wd, &format!("The runner ended {} turn(s) in phase {phase} without advancing it or asking a question. Should the install continue in this phase, move on, or stop?", after.stalls), &["continue".into(), "move on".into(), "stop".into()], questions::ApplyAs::Note, None, "vpak")?;
                audit::record(
                    wd,
                    "vpak",
                    "question.ask",
                    serde_json::json!({"id": q.id, "from": "stall"}),
                    "ok",
                )?;
            }
        } else {
            after.stalls = 0;
            wd.save_state(&after)?;
        }
        if after.status == Status::Running {
            wd.set_status(Status::Pending, "vpak", "turn ended")?;
        }
    };

    let st = wd.state()?;
    let rep = DriveReport {
        id: st.id.clone(),
        phase: st.phase,
        status: st.status,
        turns,
        open_questions: questions::open(wd)?.len(),
        workdir: wd.root().to_path_buf(),
    };
    out::emit(json, &rep, || {
        format!(
            "{} phase {} status {} after {} turn(s)",
            rep.id, rep.phase, rep.status, rep.turns
        )
    });
    Ok(code)
}

fn print_questions(open: &[questions::Question], wd: &Workdir) {
    eprintln!("vpak: a human must answer {} question(s):", open.len());
    for q in open {
        eprintln!("  {}: {}", q.id, q.question);
        if !q.options.is_empty() {
            eprintln!("      options: {}", q.options.join(", "));
        }
    }
    eprintln!("vpak: answer with: vpak --workdir \"{}\" question answer <id> --text \"...\"  then: vpak resume \"{}\"", wd.root().display(), wd.root().display());
}
