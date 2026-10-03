//! Hand an install off to a vflt collective: init the collective, install a
//! supervisor profile carrying the compiled bootstrap, seed one item per plan
//! step. vflt is reached as a separate binary so the repos stay independent.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::install::{audit, journal, plan, Workdir};
use crate::{bootstrap, util};

/// `fleet/collective.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffRecord {
    pub path: PathBuf,
    pub runner: String,
    pub supervisor_profile: String,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub items: Vec<ItemRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRef {
    pub step: u32,
    pub id: String,
    pub stage: String,
}

/// A vflt profile file (`profiles/supervisor.toml`), in vflt's documented layout.
#[derive(Debug, Clone, Serialize)]
struct SupervisorProfile {
    name: String,
    description: String,
    stages: Vec<String>,
    runner: String,
    cycle: String,
    policy: crate::policy::Policy,
}

#[derive(Debug, thiserror::Error)]
#[error("vflt is not installed: {0}. Install vflt, set VFLT_BIN, or continue inline.")]
pub struct VfltMissing(pub String);

/// Locate vflt: `VFLT_BIN` (a program, optionally followed by leading
/// arguments), else `vflt` on PATH. Returns the argv prefix.
pub fn find_vflt() -> Result<Vec<String>> {
    if let Ok(v) = std::env::var("VFLT_BIN") {
        if !v.trim().is_empty() {
            let argv = util::split_argv(&v)?;
            if crate::runner::exists_on_path(&argv[0]).is_none() {
                return Err(VfltMissing(format!(
                    "VFLT_BIN points at '{}' which does not exist",
                    argv[0]
                ))
                .into());
            }
            return Ok(argv);
        }
    }
    match which::which("vflt") {
        Ok(p) => Ok(vec![p.display().to_string()]),
        Err(_) => Err(VfltMissing("no 'vflt' on PATH".into()).into()),
    }
}

pub fn handoff_record(wd: &Workdir) -> Result<Option<HandoffRecord>> {
    let p = wd.fleet_dir().join("collective.toml");
    if !p.is_file() {
        return Ok(None);
    }
    Ok(Some(toml::from_str(&util::read_to_string(&p)?)?))
}

fn vflt(argv: &[String], collective: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if let Some(c) = collective {
        cmd.arg("--collective").arg(c);
    }
    cmd.args(args);
    let out = cmd
        .output()
        .with_context(|| format!("run {}", argv.join(" ")))?;
    if !out.status.success() {
        bail!(
            "vflt {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn item_id_from(stdout: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
            return id.to_string();
        }
    }
    stdout.split_whitespace().next().unwrap_or("").to_string()
}

/// Perform the handoff. `collective` defaults to `<dest>/.vflt`.
pub fn bootstrap(wd: &Workdir, collective: Option<PathBuf>, actor: &str) -> Result<HandoffRecord> {
    if let Some(existing) = handoff_record(wd)? {
        bail!(
            "fleet already engaged at {}; remove fleet/collective.toml to redo",
            existing.path.display()
        );
    }
    let argv = find_vflt()?;
    let st = wd.state()?;
    let coll = collective.unwrap_or_else(|| st.dest.join(".vflt"));
    std::fs::create_dir_all(&coll)?;
    let coll = util::canonical(&coll)?;

    if !coll.join("collective.toml").is_file() {
        vflt(
            &argv,
            None,
            &[
                "collective",
                "init",
                &coll.display().to_string(),
                "--name",
                &format!("vpak-{}", st.vpak_name),
            ],
        )?;
    }

    // Supervisor profile: standing instructions are the compiled bootstrap.
    let compiled = bootstrap::compile(wd)?;
    let mut policy = wd.policy()?;
    policy.allow("Bash(vflt *)");
    let profiles = coll.join("profiles");
    std::fs::create_dir_all(&profiles)?;
    let profile = SupervisorProfile {
        name: "supervisor".into(),
        description: format!("vpak install supervisor for {}", st.vpak_name),
        stages: Vec::new(),
        runner: st.runner.clone(),
        cycle: "15m".into(),
        policy: policy.clone(),
    };
    let profile_toml = toml::to_string_pretty(&profile)?;
    util::write_atomic(&profiles.join("supervisor.toml"), profile_toml)?;
    let supervisor_md = format!(
        "# Supervisor: vpak install of {} {}\n\nYou supervise a vflt collective that is realizing a vpak. The install working directory is `{}`; use its `vpak` primitives to journal progress and `vflt` to run the board. Each cycle: read the board, file or reprioritize items so the plan below completes, close items whose work is verified, and raise anything needing a human with `vflt item raise`. When the board drains, journal it with `vpak journal append --phase execute --title \"board drained\" --body ...`.\n\n---\n\n{}",
        st.vpak_name,
        st.vpak_version,
        wd.root().display(),
        compiled
    );
    util::write_atomic(&profiles.join("supervisor.md"), supervisor_md)?;

    // Seed items from pending plan steps.
    let pl = plan::Plan::load(wd)?;
    let mut items = Vec::new();
    let tmp = tempfile::Builder::new().prefix("vpak-items-").tempdir()?;
    for s in pl.pending() {
        let spec = format!(
            "# {}\n\nPlan step {} of vpak install `{}` ({} {}).\n\n- Kind: {}\n- Mutating: {}\n{}\n- Working directory: `{}`\n- Destination: `{}`\n\nWhen done, mark the step in the install: `vpak --workdir \"{}\" plan mark {} --status done --result \"...\"`.\n",
            s.step,
            s.n,
            st.id,
            st.vpak_name,
            st.vpak_version,
            s.kind.as_str(),
            s.mutating,
            s.cmd.as_ref().map(|c| format!("- Command: `{c}`")).unwrap_or_default(),
            wd.root().display(),
            st.dest.display(),
            wd.root().display(),
            s.n
        );
        let spec_path = tmp.path().join(format!("step-{:04}.md", s.n));
        std::fs::write(&spec_path, spec)?;
        let title = util::truncate(&s.step, 80);
        let out = vflt(
            &argv,
            Some(&coll),
            &[
                "item",
                "add",
                "--title",
                &title,
                "--stage",
                s.kind.as_str(),
                "--spec-file",
                &spec_path.display().to_string(),
                "--json",
            ],
        )?;
        items.push(ItemRef {
            step: s.n,
            id: item_id_from(&out),
            stage: s.kind.as_str().to_string(),
        });
    }

    let rec = HandoffRecord {
        path: coll.clone(),
        runner: st.runner.clone(),
        supervisor_profile: "supervisor".into(),
        created: util::now(),
        items,
    };
    std::fs::create_dir_all(wd.fleet_dir())?;
    util::write_atomic(
        &wd.fleet_dir().join("collective.toml"),
        toml::to_string_pretty(&rec)?,
    )?;

    let mut j = journal::Journal::load(wd)?;
    j.append(
        wd,
        st.phase,
        "fleet engaged",
        &format!("vflt collective at `{}` with supervisor profile `supervisor` and {} seeded item(s).\n\nStart it with:\n\n```\nvflt --collective \"{}\" agent run --profile supervisor\n```\n", coll.display(), rec.items.len(), coll.display()),
        actor,
    )?;
    audit::record(
        wd,
        actor,
        "fleet.bootstrap",
        serde_json::json!({"collective": coll, "items": rec.items.len()}),
        "ok",
    )?;
    journal::regenerate_current(wd)?;
    Ok(rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_item_id() {
        assert_eq!(item_id_from(r#"{"id":"vf-abc","title":"x"}"#), "vf-abc");
        assert_eq!(item_id_from("vf-xyz created"), "vf-xyz");
        assert_eq!(item_id_from(""), "");
    }
}
