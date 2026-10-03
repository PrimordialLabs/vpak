//! `vpak ask`: prompt the installer on a console (cross-platform via
//! dialoguer/console), fall back to stdin, or record the question and exit 3.

use std::io::BufRead;
use std::path::Path;

use anyhow::{bail, Result};
use vpak_core::install::{audit, questions, Mode, Workdir};
use vpak_core::util;

use crate::cli::AskArgs;
use crate::out::{self, EXIT_NEEDS_HUMAN, EXIT_OK};

/// Whether an interactive console can be used for prompts.
pub fn console_available() -> bool {
    console::user_attended_stderr() || console::user_attended()
}

/// Ask on the console. Errors if no console is usable.
pub fn prompt_console(q: &questions::Question) -> Result<String> {
    let term = console::Term::stderr();
    if !term.is_term() {
        bail!("no console");
    }
    if q.options.is_empty() {
        let s: String = dialoguer::Input::new()
            .with_prompt(format!("vpak {} ▸ {}", q.id, q.question))
            .allow_empty(false)
            .interact_text_on(&term)?;
        Ok(s)
    } else {
        let idx = dialoguer::Select::new()
            .with_prompt(format!("vpak {} ▸ {}", q.id, q.question))
            .items(&q.options)
            .default(0)
            .interact_on(&term)?;
        Ok(q.options[idx].clone())
    }
}

fn prompt_stdin(q: &questions::Question) -> Option<String> {
    eprintln!("vpak {} ▸ {}", q.id, q.question);
    if !q.options.is_empty() {
        eprintln!("  options: {}", q.options.join(", "));
    }
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(n) if n > 0 && !line.trim().is_empty() => Some(line.trim().to_string()),
        _ => None,
    }
}

pub fn run(a: AskArgs, workdir: Option<&Path>, json: bool) -> Result<i32> {
    let wd = Workdir::resolve(workdir)?;
    let st = wd.state()?;
    let mode = match std::env::var("VPAK_MODE") {
        Ok(m) if !m.trim().is_empty() => m.parse::<Mode>()?,
        _ => st.mode,
    };
    let apply_as: questions::ApplyAs = a.apply_as.parse()?;
    let options: Vec<String> = a
        .options
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let actor = Workdir::actor();
    let q = questions::ask(
        &wd,
        &a.question,
        &options,
        apply_as,
        a.key.as_deref(),
        &actor,
    )?;
    audit::record(
        &wd,
        &actor,
        "question.ask",
        serde_json::json!({"id": q.id, "question": q.question, "as": apply_as.as_str(), "key": q.key}),
        "ok",
    )?;

    if mode == Mode::Interactive {
        let answer = match prompt_console(&q) {
            Ok(s) => Some(s),
            Err(_) => prompt_stdin(&q),
        };
        if let Some(ans) = answer {
            let q = questions::answer(&wd, &q.id, &ans, &util::identity())?;
            audit::record(
                &wd,
                &util::identity(),
                "question.answer",
                serde_json::json!({"id": q.id}),
                "ok",
            )?;
            out::emit(json, &q, || q.answer.clone());
            return Ok(EXIT_OK);
        }
        out::note("no console or stdin answer available; question recorded for a human");
    }
    out::emit(json, &q, || {
        format!(
            "{} recorded; a human must answer it (vpak question answer {} --text ...)",
            q.id, q.id
        )
    });
    Ok(EXIT_NEEDS_HUMAN)
}
