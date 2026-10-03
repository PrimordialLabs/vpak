//! `vpak add` and `vpak inspect`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use vpak_core::archive::{self, PackOptions, SecretsFound};
use vpak_core::policy::Policy;
use vpak_core::runner::{self, Config, RunRequest};
use vpak_core::{templates, util};

use crate::cli::AddArgs;
use crate::out::{self, EXIT_OK};

pub fn run(a: AddArgs, json: bool) -> Result<i32> {
    let directives = match &a.directives {
        Some(d) => Some(util::resolve_text_arg(d)?),
        None => None,
    };
    let source = util::canonical(&a.dir)?;
    let probe_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| util::slug(s, 64))
        .unwrap_or_else(|| "vpak".into());
    let out_path = a
        .out
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(format!("{probe_name}.vpak")));
    let out_abs = if out_path.is_absolute() {
        out_path.clone()
    } else {
        std::env::current_dir()?.join(&out_path)
    };

    let opts = PackOptions {
        source: source.clone(),
        directives: directives.clone(),
        ignore: a.ignore.clone(),
        allow_secret: a.allow_secret.clone(),
        packer: a.packer.clone(),
        exclude_abs: vec![out_abs.clone()],
    };

    let mut st = match archive::stage(&opts) {
        Ok(s) => s,
        Err(e) => {
            if let Some(found) = e.downcast_ref::<SecretsFound>() {
                eprintln!("vpak: {found}");
                for f in &found.0 {
                    eprintln!("  {}:{} {} {}", f.path.display(), f.line, f.kind, f.snippet);
                }
                return Ok(1);
            }
            return Err(e);
        }
    };
    // The structured-mode name may differ from the directory name.
    let out_abs = if a.out.is_none() {
        std::env::current_dir()?.join(format!("{}.vpak", st.manifest.name))
    } else {
        out_abs
    };

    let mut inspected = false;
    if !a.no_inspect {
        let config = Config::load()?;
        let name = a
            .runner
            .clone()
            .or(config.default_runner.clone())
            .unwrap_or_else(|| "claude".into());
        match runner::by_name(&name, &config) {
            Ok(r) if r.available()?.is_available() => {
                out::note(&format!("inspection pass with runner '{name}'"));
                let mut vars: HashMap<&str, String> = HashMap::new();
                vars.insert("name", st.manifest.name.clone());
                vars.insert(
                    "directives",
                    directives.clone().unwrap_or_else(|| "(none given)".into()),
                );
                let prompt = util::render(templates::PACK_INSPECT, &vars);
                let sys_file = st.path().join("provenance").join("inspect-system.md");
                std::fs::create_dir_all(st.path().join("provenance"))?;
                std::fs::write(&sys_file, "You are the vpak pack inspector. Follow the instructions in the user message exactly; write only inside the current directory.\n")?;
                let mut policy = Policy::default();
                policy.allow("Edit");
                policy.allow("Write");
                let req = RunRequest {
                    system_prompt_file: sys_file.clone(),
                    prompt,
                    cwd: st.path().to_path_buf(),
                    extra_dirs: vec![],
                    policy: policy.clone(),
                    budget: policy.budget.clone(),
                    session: None,
                    env: {
                        let mut env = runner::self_env();
                        env.push(("VPAK_PACK_STAGE".into(), st.path().display().to_string()));
                        env
                    },
                };
                let mut events = Vec::new();
                let outcome = r.run(&req, &mut |ev| {
                    if !json {
                        eprintln!("  [{}] {}", ev.kind, ev.summary);
                    }
                    events.push(format!("{} {}", ev.kind, ev.summary));
                })?;
                let _ = std::fs::remove_file(&sys_file);
                st.log(format!(
                    "inspection runner {name}: exit {:?} turns {:?} cost {:?}",
                    outcome.exit_code, outcome.num_turns, outcome.cost_usd
                ));
                if outcome.is_error {
                    st.log(format!(
                        "inspection runner reported an error: {}",
                        util::truncate(&outcome.stderr_tail, 400)
                    ));
                    out::note(
                        "inspection pass reported an error; packing anyway with what it wrote",
                    );
                }
                inspected = true;
            }
            Ok(r) => {
                let av = r.available()?;
                let survey = runner::survey(&config);
                eprintln!(
                    "vpak: runner '{name}' is not available: {}",
                    match av {
                        runner::Availability::Unavailable { reason } => reason,
                        _ => String::new(),
                    }
                );
                eprintln!("vpak: runners known to this machine:");
                for (n, av) in survey {
                    eprintln!("  {n}: {}", describe(&av));
                }
                bail!("no runner for the inspection pass; pass --runner <name> or --no-inspect");
            }
            Err(e) => bail!("{e}; pass --runner <name> or --no-inspect"),
        }
    }

    let report = archive::finalize(st, &out_abs, inspected)
        .with_context(|| format!("write {}", out_abs.display()))?;
    out::emit(json, &report, || {
        format!(
            "packed {} {} ({:?} source) -> {}\n  {} files, {} bytes, {}\n  inspection: {}",
            report.name,
            report.version,
            report.mode,
            report.out.display(),
            report.files,
            report.bytes,
            report.digest,
            if inspected { "ran" } else { "skipped" }
        )
    });
    Ok(EXIT_OK)
}

fn describe(av: &runner::Availability) -> String {
    match av {
        runner::Availability::Available { detail } => format!("available ({detail})"),
        runner::Availability::Unavailable { reason } => format!("unavailable: {reason}"),
    }
}

#[derive(Serialize)]
struct InspectReport {
    manifest: vpak_core::Manifest,
    digest: String,
    entries: Vec<archive::EntryInfo>,
    policy: Option<Policy>,
}

pub fn inspect(file: &Path, json: bool) -> Result<i32> {
    let (manifest, entries) = archive::list(file)?;
    let digest = format!("sha256:{}", util::sha256_file(file)?);
    // Policy is small; read it out of the archive for the summary.
    let policy = {
        let tmp = tempfile::tempdir()?;
        archive::unpack(file, tmp.path()).ok();
        let p = tmp.path().join(&manifest.bootstrap.policy);
        if p.is_file() {
            Policy::load(&p).ok()
        } else {
            None
        }
    };
    let rep = InspectReport {
        manifest,
        digest,
        entries,
        policy,
    };
    out::emit(json, &rep, || {
        let m = &rep.manifest;
        let mut s = format!("{} {} (format {})\n", m.name, m.version, m.format);
        if !m.summary.is_empty() {
            s.push_str(&format!("  {}\n", m.summary));
        }
        s.push_str(&format!(
            "  created {}  packer {}\n  {}\n",
            m.created.to_rfc3339(),
            m.packer.clone().unwrap_or_else(|| "-".into()),
            rep.digest
        ));
        s.push_str(&format!(
            "  reference kind: {}  entry: {}\n",
            m.reference.kind.clone().unwrap_or_else(|| "-".into()),
            m.reference.entry.join(", ")
        ));
        s.push_str(&format!("  intent: {}\n", m.intent.order.join(", ")));
        if !m.targets.is_empty() {
            s.push_str("  targets:\n");
            for t in &m.targets {
                s.push_str(&format!("    {}: {} ({})\n", t.name, t.summary, t.path));
            }
        }
        if let Some(p) = &rep.policy {
            s.push_str(&format!(
                "  policy: mode {} allow {} deny {} max_turns {}\n",
                p.permission_mode,
                p.allowed_tools.len(),
                p.disallowed_tools.len(),
                p.budget.max_turns
            ));
        }
        s.push_str(&format!("  {} files:\n", rep.entries.len()));
        for e in &rep.entries {
            s.push_str(&format!(
                "    {:>8}  {}\n",
                e.size,
                util::slash_path(&e.path)
            ));
        }
        s.trim_end().to_string()
    });
    Ok(EXIT_OK)
}
