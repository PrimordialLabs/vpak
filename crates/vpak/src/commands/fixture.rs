//! Hidden test fixtures, built into the binary so tests stay cross-platform
//! without shell scripts.
//!
//! `vpak __fixture vflt <args...>` — a fake vflt: logs its argv as a JSON line
//! to `$VPAK_FIXTURE_LOG`, creates a collective on `collective init`, prints
//! an item id on `item add`.
//!
//! `vpak __fixture agent` — a fake runner agent: reads the prompt on stdin,
//! then drives the real `vpak` primitives (via this same executable) to record
//! a fact, a constraint and a journal entry and advance the phase. Env knobs:
//! `VPAK_FIXTURE_STALL=1` does nothing; `VPAK_FIXTURE_ASK=1` asks a question.

use std::io::Read;
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn run(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("vflt") => fake_vflt(&args[1..]),
        Some("agent") => fake_agent(),
        _ => bail!("usage: vpak __fixture vflt <args...> | vpak __fixture agent"),
    }
}

fn fake_vflt(args: &[String]) -> Result<i32> {
    let log = std::env::var("VPAK_FIXTURE_LOG").ok();
    let mut count = 0usize;
    if let Some(l) = &log {
        if let Ok(text) = std::fs::read_to_string(l) {
            count = text.lines().filter(|x| x.contains("\"item\"")).count();
        }
        vpak_core::util::append_line(std::path::Path::new(l), &serde_json::to_string(args)?)?;
    }
    let pos: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Some(i) = pos.iter().position(|a| *a == "collective") {
        if pos.get(i + 1) == Some(&"init") {
            if let Some(dir) = pos.get(i + 2) {
                std::fs::create_dir_all(dir)?;
                std::fs::write(
                    std::path::Path::new(dir).join("collective.toml"),
                    "name = \"fake\"\n[store]\nkind = \"file\"\n",
                )?;
            }
        }
    }
    if let Some(i) = pos.iter().position(|a| *a == "item") {
        if pos.get(i + 1) == Some(&"add") {
            let id = format!("vf-fake-{:04}", count + 1);
            if pos.contains(&"--json") {
                println!("{{\"id\":\"{id}\"}}");
            } else {
                println!("{id}");
            }
        }
    }
    Ok(0)
}

fn vpak(args: &[&str]) -> Result<i32> {
    let exe = std::env::current_exe().context("current_exe")?;
    let status = Command::new(exe)
        .args(args)
        .status()
        .with_context(|| format!("run vpak {}", args.join(" ")))?;
    Ok(status.code().unwrap_or(1))
}

fn fake_agent() -> Result<i32> {
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    let phase = std::env::var("VPAK_PHASE").unwrap_or_else(|_| "locate".into());
    eprintln!("fake-agent: phase {phase}, prompt {} bytes", prompt.len());
    if std::env::var("VPAK_FIXTURE_STALL").ok().as_deref() == Some("1") {
        println!("stalling on purpose");
        return Ok(0);
    }
    if std::env::var("VPAK_FIXTURE_ASK").ok().as_deref() == Some("1") {
        let code = vpak(&[
            "ask",
            "--question",
            "Which region?",
            "--options",
            "us-east-1,eu-west-1",
            "--as",
            "target",
            "--key",
            "region",
        ])?;
        println!("asked, exit {code}");
        return Ok(0);
    }
    vpak(&[
        "survey",
        "record",
        "--key",
        &format!("fixture.{phase}"),
        "--value",
        "ran",
        "--source",
        "fake-agent",
    ])?;
    vpak(&[
        "constraint",
        "add",
        "--source",
        "discovered",
        "--key",
        &format!("fixture.{phase}"),
        "--value",
        "yes",
        "--text",
        "fixture constraint",
    ])?;
    vpak(&[
        "journal",
        "append",
        "--phase",
        &phase,
        "--title",
        "fixture turn",
        "--body",
        &format!("The fake agent handled phase {phase}."),
    ])?;
    if phase == "plan" {
        vpak(&[
            "plan",
            "map",
            "--from",
            "cloud-run",
            "--to",
            "docker",
            "--why",
            "fixture",
        ])?;
        vpak(&["plan", "add", "--step", "build image", "--kind", "code"])?;
        vpak(&[
            "plan",
            "add",
            "--step",
            "run container",
            "--mutating",
            "--cmd",
            "docker run hello",
        ])?;
    }
    if phase == "execute" {
        vpak(&[
            "plan",
            "mark",
            "1",
            "--status",
            "done",
            "--result",
            "fixture built image",
        ])?;
        vpak(&[
            "plan",
            "mark",
            "2",
            "--status",
            "done",
            "--result",
            "fixture ran container",
        ])?;
    }
    let next: &str = match phase.as_str() {
        "locate" => "survey",
        "survey" => "constrain",
        "constrain" => "plan",
        "plan" => "decide",
        "decide" => "execute",
        "execute" => "verify",
        _ => "done",
    };
    vpak(&["phase", "set", next, "--reason", "fixture"])?;
    println!("fake-agent advanced {phase} -> {next}");
    Ok(0)
}
