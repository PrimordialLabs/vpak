//! The primitives: deterministic, audited operations on the install workdir.

use std::path::Path;

use anyhow::{bail, Result};
use vpak_core::install::{
    audit, constraints, journal, plan, questions, status_report, survey, target, Phase, Workdir,
};
use vpak_core::runner::{self, Config};
use vpak_core::util;

use crate::cli::{
    Cmd, ConstraintCmd, FleetCmd, JournalCmd, PhaseCmd, PlanCmd, PolicyCmd, QuestionCmd, SurveyCmd,
    TargetCmd,
};
use crate::out::{self, EXIT_OK};

pub fn run(cmd: Cmd, workdir: Option<&Path>, json: bool) -> Result<i32> {
    let wd = Workdir::resolve(workdir)?;
    let actor = Workdir::actor();
    match cmd {
        Cmd::Status => {
            let rep = status_report(&wd)?;
            out::emit(json, &rep, || {
                let mut s = format!(
                    "{} — {}\n  phase {}  status {}  mode {}  runner {}\n  workdir {}\n  dest    {}\n  facts {}  constraints {}  open conflicts {}  mappings {}  steps {}/{} done  journal {}",
                    rep.id, rep.vpak, rep.phase, rep.status, rep.mode.as_str(), rep.runner, rep.workdir.display(), rep.dest.display(), rep.facts, rep.constraints, rep.open_conflicts, rep.mappings, rep.steps_done, rep.steps_total, rep.journal_entries
                );
                if let Some(f) = &rep.fleet {
                    s.push_str(&format!("\n  fleet   {}", f.display()));
                }
                if !rep.open_questions.is_empty() {
                    s.push_str(&format!(
                        "\n  open questions ({}):",
                        rep.open_questions.len()
                    ));
                    for q in &rep.open_questions {
                        s.push_str(&format!("\n    {}: {}", q.id, q.question));
                    }
                }
                s
            });
        }
        Cmd::Phase {
            cmd:
                PhaseCmd::Set {
                    phase,
                    reason,
                    force,
                },
        } => {
            let to: Phase = phase.parse()?;
            let st = wd.set_phase_opts(to, reason.as_deref(), &actor, force)?;
            out::emit(json, &st, || {
                format!("phase {} status {}", st.phase, st.status)
            });
        }
        Cmd::Journal { cmd } => match cmd {
            JournalCmd::Append {
                phase,
                title,
                body,
                body_file,
            } => {
                let st = wd.state()?;
                let ph: Phase = match phase {
                    Some(p) => p.parse()?,
                    None => st.phase,
                };
                let text = match (body, body_file) {
                    (Some(b), _) => b,
                    (None, Some(f)) => util::resolve_text_arg(&f)?,
                    (None, None) => String::new(),
                };
                let mut j = journal::Journal::load(&wd)?;
                let e = j.append(&wd, ph, &title, &text, &actor)?;
                let (seq, file) = (e.seq, e.file.clone());
                audit::record(
                    &wd,
                    &actor,
                    "journal.append",
                    serde_json::json!({"phase": ph, "title": title, "seq": seq}),
                    "ok",
                )?;
                journal::regenerate_current(&wd)?;
                out::emit(json, &serde_json::json!({"seq": seq, "file": file}), || {
                    format!("journal entry {seq} -> {}", file.display())
                });
            }
            JournalCmd::Current => {
                let text = journal::regenerate_current(&wd)?;
                println!("{text}");
            }
            JournalCmd::List => {
                let j = journal::Journal::load(&wd)?;
                out::emit(json, &j.entries, || {
                    j.entries
                        .iter()
                        .map(|e| {
                            format!(
                                "{:04} {} {} {} ({})",
                                e.seq,
                                e.ts.to_rfc3339(),
                                e.phase,
                                e.title,
                                e.actor
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            }
        },
        Cmd::Target { cmd } => {
            let mut t = target::Target::load(&wd)?;
            match cmd {
                TargetCmd::Show => {
                    out::emit(
                        json,
                        &serde_json::json!({"prose": t.prose, "structured": t.render_flat()}),
                        || format!("{}\n{}", t.prose.trim(), t.render_flat()),
                    );
                }
                TargetCmd::Set { key, value } => {
                    t.set(&key, &value)?;
                    t.save(&wd)?;
                    audit::record(
                        &wd,
                        &actor,
                        "target.set",
                        serde_json::json!({"key": key, "value": value}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(
                        json,
                        &serde_json::json!({"key": key, "value": value}),
                        || format!("target.{key} = {value}"),
                    );
                }
                TargetCmd::Note { text } => {
                    t.note(&text);
                    t.save(&wd)?;
                    audit::record(
                        &wd,
                        &actor,
                        "target.note",
                        serde_json::json!({"text": text}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(json, &serde_json::json!({"noted": text}), || "noted".into());
                }
            }
        }
        Cmd::Survey { cmd } => {
            let mut f = survey::Facts::load(&wd)?;
            match cmd {
                SurveyCmd::Record {
                    key,
                    value,
                    source,
                    sensitivity,
                } => {
                    let sens: survey::Sensitivity = sensitivity.parse()?;
                    let st = wd.state()?;
                    let fact = f.record(&key, &value, &source, sens, st.phase).clone();
                    f.save(&wd)?;
                    let logged = if sens == survey::Sensitivity::High {
                        "[redacted]".to_string()
                    } else {
                        value.clone()
                    };
                    audit::record(
                        &wd,
                        &actor,
                        "survey.record",
                        serde_json::json!({"key": key, "value": logged, "source": source, "sensitivity": sens}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(json, &fact, || {
                        format!(
                            "{} = {}",
                            fact.key,
                            if sens == survey::Sensitivity::High {
                                "[redacted]".to_string()
                            } else {
                                fact.value.clone()
                            }
                        )
                    });
                }
                SurveyCmd::List => {
                    out::emit(json, &f.facts, || {
                        f.facts
                            .iter()
                            .map(|x| {
                                format!(
                                    "{} = {} (via {}) [{}]",
                                    x.key,
                                    if x.sensitivity == survey::Sensitivity::High {
                                        "[redacted]".to_string()
                                    } else {
                                        x.value.clone()
                                    },
                                    x.source,
                                    x.phase
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
            }
        }
        Cmd::Constraint { cmd } => {
            let mut cs = constraints::Constraints::load(&wd)?;
            match cmd {
                ConstraintCmd::Add {
                    source,
                    text,
                    key,
                    value,
                    conflicts_with,
                } => {
                    let src: constraints::Source = source.parse()?;
                    if src == constraints::Source::Packer {
                        bail!("packer constraints come from the archive; use --source installer or discovered");
                    }
                    let (id, outcome) = cs.add(
                        src,
                        key.as_deref(),
                        value.as_deref(),
                        text.as_deref().unwrap_or(""),
                        conflicts_with.as_deref(),
                    )?;
                    cs.save(&wd)?;
                    audit::record(
                        &wd,
                        &actor,
                        "constraint.add",
                        serde_json::json!({"id": id, "source": src, "key": key, "value": value, "text": text, "outcome": outcome}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(
                        json,
                        &serde_json::json!({"id": id, "outcome": outcome}),
                        || match &outcome {
                            constraints::AddOutcome::Added => format!("{id} added"),
                            constraints::AddOutcome::Superseded { previous } => {
                                format!("{id} added; supersedes {previous}")
                            }
                            constraints::AddOutcome::Conflict { with, conflict_id } => format!(
                                "{id} added; CONFLICT {conflict_id} with {with} — resolve or ask"
                            ),
                        },
                    );
                }
                ConstraintCmd::List => {
                    out::emit(json, &cs.constraints, || {
                        cs.constraints
                            .iter()
                            .map(|c| {
                                format!(
                                    "{} [{}]{}{} {}{}",
                                    c.id,
                                    c.source.as_str(),
                                    c.key.as_ref().map(|k| format!(" {k}")).unwrap_or_default(),
                                    c.value
                                        .as_ref()
                                        .map(|v| format!("={v}"))
                                        .unwrap_or_default(),
                                    c.text,
                                    c.superseded_by
                                        .as_ref()
                                        .map(|s| format!(" (superseded by {s})"))
                                        .unwrap_or_default()
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
                ConstraintCmd::Conflicts => {
                    out::emit(json, &cs.conflicts, || {
                        if cs.conflicts.is_empty() {
                            return "no conflicts".into();
                        }
                        cs.conflicts
                            .iter()
                            .map(|x| {
                                format!(
                                    "{} [{:?}] {} vs {} on {}{}",
                                    x.id,
                                    x.status,
                                    x.a,
                                    x.b,
                                    x.key.as_deref().unwrap_or("-"),
                                    if x.kept.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" -> kept {}: {}", x.kept, x.resolution)
                                    }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                }
                ConstraintCmd::Resolve { id, keep, reason } => {
                    cs.resolve(&id, &keep, &reason)?;
                    cs.save(&wd)?;
                    audit::record(
                        &wd,
                        &actor,
                        "constraint.resolve",
                        serde_json::json!({"conflict": id, "keep": keep, "reason": reason}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(
                        json,
                        &serde_json::json!({"conflict": id, "kept": keep}),
                        || format!("{id} resolved; kept {keep}"),
                    );
                }
            }
        }
        Cmd::Plan { cmd } => {
            let mut p = plan::Plan::load(&wd)?;
            match cmd {
                PlanCmd::Map { from, to, why } => {
                    p.map(&wd, &from, &to, &why)?;
                    audit::record(
                        &wd,
                        &actor,
                        "plan.map",
                        serde_json::json!({"from": from, "to": to, "why": why}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(json, &serde_json::json!({"from": from, "to": to}), || {
                        format!("{from} -> {to}")
                    });
                }
                PlanCmd::Add {
                    step,
                    mutating,
                    cmd,
                    kind,
                } => {
                    let k = match kind {
                        Some(k) => Some(k.parse::<plan::StepKind>()?),
                        None => None,
                    };
                    let s = p.add_step(&wd, &step, mutating, cmd.as_deref(), k)?.clone();
                    audit::record(
                        &wd,
                        &actor,
                        "plan.add",
                        serde_json::json!({"n": s.n, "step": s.step, "mutating": s.mutating, "kind": s.kind, "cmd": s.cmd}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(json, &s, || {
                        format!(
                            "step {} [{}]{} {}",
                            s.n,
                            s.kind.as_str(),
                            if s.mutating { " mutating" } else { "" },
                            s.step
                        )
                    });
                }
                PlanCmd::List => {
                    out::emit(
                        json,
                        &serde_json::json!({"mappings": p.mappings, "steps": p.steps}),
                        || {
                            let mut s = String::new();
                            for m in &p.mappings {
                                s.push_str(&format!("map {} -> {}: {}\n", m.from, m.to, m.why));
                            }
                            for st in &p.steps {
                                s.push_str(&format!(
                                    "{}. [{}] [{}]{} {}{}\n",
                                    st.n,
                                    st.status.as_str(),
                                    st.kind.as_str(),
                                    if st.mutating { " mutating" } else { "" },
                                    st.step,
                                    if st.result.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" — {}", st.result)
                                    }
                                ));
                            }
                            if s.is_empty() {
                                s.push_str("no plan yet");
                            }
                            s.trim_end().to_string()
                        },
                    );
                }
                PlanCmd::Mark { n, status, result } => {
                    let stt: plan::StepStatus = status.parse()?;
                    let s = p.mark(&wd, n, stt, &result)?.clone();
                    audit::record(
                        &wd,
                        &actor,
                        "plan.mark",
                        serde_json::json!({"n": n, "status": stt, "result": result}),
                        "ok",
                    )?;
                    journal::regenerate_current(&wd)?;
                    out::emit(json, &s, || format!("step {} {}", s.n, s.status.as_str()));
                }
            }
        }
        Cmd::Question { cmd } => match cmd {
            QuestionCmd::List => {
                let qs = questions::all(&wd)?;
                out::emit(json, &qs, || {
                    qs.iter()
                        .map(|q| {
                            format!(
                                "{} [{:?}] {}{}",
                                q.id,
                                q.status,
                                q.question,
                                if q.answer.is_empty() {
                                    String::new()
                                } else {
                                    format!(" -> {}", q.answer)
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            }
            QuestionCmd::Answer { id, text } => {
                let who = util::identity();
                let q = questions::answer(&wd, &id, &text, &who)?;
                audit::record(
                    &wd,
                    &who,
                    "question.answer",
                    serde_json::json!({"id": id}),
                    "ok",
                )?;
                out::emit(json, &q, || format!("{} answered", q.id));
            }
        },
        Cmd::Policy { cmd } => {
            let mut p = wd.policy()?;
            match cmd {
                PolicyCmd::Show => out::emit(json, &p, || p.to_toml().unwrap_or_default()),
                PolicyCmd::Allow { tool } => {
                    p.allow(&tool);
                    wd.save_policy(&p)?;
                    audit::record(
                        &wd,
                        &actor,
                        "policy.allow",
                        serde_json::json!({"tool": tool}),
                        "ok",
                    )?;
                    out::emit(json, &p, || format!("allowed {tool}"));
                }
                PolicyCmd::Deny { tool } => {
                    p.deny(&tool);
                    wd.save_policy(&p)?;
                    audit::record(
                        &wd,
                        &actor,
                        "policy.deny",
                        serde_json::json!({"tool": tool}),
                        "ok",
                    )?;
                    out::emit(json, &p, || format!("denied {tool}"));
                }
                PolicyCmd::Mode { mode } => {
                    p.permission_mode = mode.clone();
                    wd.save_policy(&p)?;
                    audit::record(
                        &wd,
                        &actor,
                        "policy.mode",
                        serde_json::json!({"mode": mode}),
                        "ok",
                    )?;
                    out::emit(json, &p, || format!("permission mode {mode}"));
                }
            }
        }
        Cmd::Fleet {
            cmd: FleetCmd::Bootstrap { collective },
        } => {
            let rec = match vpak_core::fleet::bootstrap(&wd, collective, &actor) {
                Ok(r) => r,
                Err(e) => {
                    if let Some(m) = e.downcast_ref::<vpak_core::fleet::VfltMissing>() {
                        audit::record(
                            &wd,
                            &actor,
                            "fleet.bootstrap",
                            serde_json::json!({}),
                            &format!("unavailable: {m}"),
                        )?;
                        eprintln!("vpak: {m}");
                        return Ok(1);
                    }
                    return Err(e);
                }
            };
            out::emit(json, &rec, || {
                format!("fleet engaged at {} with {} item(s); run: vflt --collective \"{}\" agent run --profile supervisor", rec.path.display(), rec.items.len(), rec.path.display())
            });
        }
        other => bail!("unhandled command {other:?}"),
    }
    Ok(EXIT_OK)
}

pub fn runners(json: bool) -> Result<i32> {
    let config = Config::load()?;
    let list = runner::survey(&config);
    out::emit(json, &list, || {
        list.iter()
            .map(|(n, av)| match av {
                runner::Availability::Available { detail } => format!("{n}: available ({detail})"),
                runner::Availability::Unavailable { reason } => {
                    format!("{n}: unavailable — {reason}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(EXIT_OK)
}
