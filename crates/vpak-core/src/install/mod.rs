//! The install working directory: where the bootstrap reasons, records, and
//! resumes from. Everything here is file-backed and append-friendly so the
//! trail is obvious to a human reading the directory.

pub mod audit;
pub mod constraints;
pub mod journal;
pub mod plan;
pub mod questions;
pub mod survey;
pub mod target;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::archive;
use crate::manifest::Manifest;
use crate::policy::Policy;
use crate::util;

pub const INSTALL_FILE: &str = "install.toml";

/// The seven bootstrap phases plus the terminal pseudo-phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Locate,
    Survey,
    Constrain,
    Plan,
    Decide,
    Execute,
    Verify,
    Done,
}

impl Phase {
    pub const ALL: [Phase; 8] = [
        Phase::Locate,
        Phase::Survey,
        Phase::Constrain,
        Phase::Plan,
        Phase::Decide,
        Phase::Execute,
        Phase::Verify,
        Phase::Done,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Locate => "locate",
            Phase::Survey => "survey",
            Phase::Constrain => "constrain",
            Phase::Plan => "plan",
            Phase::Decide => "decide",
            Phase::Execute => "execute",
            Phase::Verify => "verify",
            Phase::Done => "done",
        }
    }

    /// The natural successor in the forward pipeline.
    pub fn next(&self) -> Option<Phase> {
        let i = Phase::ALL.iter().position(|p| p == self)?;
        Phase::ALL.get(i + 1).copied()
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Phase::Done)
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Phase {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Phase::ALL
            .iter()
            .find(|p| p.as_str() == s.trim().to_lowercase())
            .copied()
            .ok_or_else(|| {
                anyhow!(
                    "unknown phase '{s}'; expected one of {}",
                    Phase::ALL
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Running,
    NeedsHuman,
    Done,
    Failed,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Running => "running",
            Status::NeedsHuman => "needs_human",
            Status::Done => "done",
            Status::Failed => "failed",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Interactive,
    Auto,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Interactive => "interactive",
            Mode::Auto => "auto",
        }
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "interactive" | "i" => Ok(Mode::Interactive),
            "auto" | "a" => Ok(Mode::Auto),
            _ => bail!("unknown mode '{s}'; expected interactive or auto"),
        }
    }
}

/// `install.toml`: identity and current state of one install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallState {
    pub id: String,
    pub vpak_name: String,
    pub vpak_version: String,
    pub vpak_digest: String,
    pub vpak_path: PathBuf,
    pub phase: Phase,
    pub status: Status,
    pub runner: String,
    pub mode: Mode,
    pub dest: PathBuf,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    /// Runner session id to resume, when the runner supports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Number of runner turns that ended without a phase change.
    #[serde(default)]
    pub stalls: u32,
}

#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Install id; generated when `None`.
    pub id: Option<String>,
    pub vpak_path: PathBuf,
    pub workdir: PathBuf,
    pub dest: PathBuf,
    pub runner: String,
    pub mode: Mode,
    pub target_text: Option<String>,
    pub constraints_text: Option<String>,
}

/// A handle on an install working directory.
#[derive(Debug, Clone)]
pub struct Workdir {
    root: PathBuf,
}

impl Workdir {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn install_file(&self) -> PathBuf {
        self.root.join(INSTALL_FILE)
    }
    pub fn vpak_dir(&self) -> PathBuf {
        self.root.join("vpak")
    }
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }
    pub fn constraints_dir(&self) -> PathBuf {
        self.root.join("constraints")
    }
    pub fn survey_dir(&self) -> PathBuf {
        self.root.join("survey")
    }
    pub fn plan_dir(&self) -> PathBuf {
        self.root.join("plan")
    }
    pub fn bootstrap_dir(&self) -> PathBuf {
        self.root.join("bootstrap")
    }
    pub fn journal_dir(&self) -> PathBuf {
        self.bootstrap_dir().join("journal")
    }
    pub fn current_file(&self) -> PathBuf {
        self.bootstrap_dir().join("current.md")
    }
    pub fn compiled_file(&self) -> PathBuf {
        self.bootstrap_dir().join("compiled.md")
    }
    pub fn fleet_dir(&self) -> PathBuf {
        self.root.join("fleet")
    }
    pub fn questions_dir(&self) -> PathBuf {
        self.root.join("questions")
    }
    pub fn audit_file(&self) -> PathBuf {
        self.root.join("audit.jsonl")
    }
    pub fn policy_file(&self) -> PathBuf {
        self.root.join("policy.toml")
    }

    /// Open an existing workdir.
    pub fn open(root: &Path) -> Result<Self> {
        let root = util::canonical(root)?;
        if !root.join(INSTALL_FILE).is_file() {
            bail!(
                "{} is not a vpak install workdir (no {INSTALL_FILE})",
                root.display()
            );
        }
        Ok(Self { root })
    }

    /// Resolve the workdir from an explicit path, `VPAK_WORKDIR`, or the
    /// nearest ancestor of the current directory that holds `install.toml`.
    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        if let Some(p) = explicit {
            return Self::open(p);
        }
        if let Ok(v) = std::env::var("VPAK_WORKDIR") {
            if !v.trim().is_empty() {
                return Self::open(Path::new(&v));
            }
        }
        let cwd = std::env::current_dir()?;
        let mut cur: Option<&Path> = Some(&cwd);
        while let Some(d) = cur {
            if d.join(INSTALL_FILE).is_file() {
                return Self::open(d);
            }
            cur = d.parent();
        }
        bail!("no install workdir found: pass --workdir, set VPAK_WORKDIR, or run inside one")
    }

    /// Create a new workdir: unpack the archive, seed state, target,
    /// constraints and policy, and write the first journal entry.
    pub fn init(opts: InitOptions) -> Result<(Self, Manifest)> {
        let root = &opts.workdir;
        if root.join(INSTALL_FILE).exists() {
            bail!("{} already holds an install; use resume", root.display());
        }
        std::fs::create_dir_all(root)?;
        let root = util::canonical(root)?;
        let wd = Self { root: root.clone() };
        for d in [
            wd.vpak_dir(),
            wd.target_dir(),
            wd.constraints_dir(),
            wd.survey_dir(),
            wd.plan_dir().join("steps"),
            wd.journal_dir(),
            wd.questions_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }

        let vpak_path = util::canonical(&opts.vpak_path)?;
        let manifest = archive::unpack(&vpak_path, &wd.vpak_dir())
            .with_context(|| format!("unpack {}", vpak_path.display()))?;
        let digest = format!("sha256:{}", util::sha256_file(&vpak_path)?);
        let dest = if opts.dest.is_absolute() {
            opts.dest.clone()
        } else {
            std::env::current_dir()?.join(&opts.dest)
        };

        let now = util::now();
        let state = InstallState {
            id: opts.id.clone().unwrap_or_else(|| util::new_id("vp")),
            vpak_name: manifest.name.clone(),
            vpak_version: manifest.version.clone(),
            vpak_digest: digest,
            vpak_path,
            phase: Phase::Locate,
            status: Status::Pending,
            runner: opts.runner.clone(),
            mode: opts.mode,
            dest,
            created: now,
            updated: now,
            session: None,
            stalls: 0,
        };
        wd.save_state(&state)?;

        // Policy: start from the archive's, refine in the workdir.
        let archive_policy = wd.vpak_dir().join(&manifest.bootstrap.policy);
        let policy = if archive_policy.is_file() {
            Policy::load(&archive_policy)?
        } else {
            Policy::default()
        };
        util::write_atomic(&wd.policy_file(), policy.to_toml()?)?;

        // Target.
        let mut tgt = target::Target::load(&wd)?;
        match &opts.target_text {
            Some(t) if !t.trim().is_empty() => {
                tgt.prose = format!("{}\n", t.trim());
                tgt.set("origin", "explicit")?;
                tgt.set("summary", &util::first_sentence(t, 160))?;
            }
            _ => {
                tgt.prose = "No explicit target was given. The target is inferred as the closest analogy to the reference in the environment the survey finds.\n".into();
                tgt.set("origin", "inferred")?;
            }
        }
        tgt.save(&wd)?;

        // Constraints: installer prose, then packer entries imported.
        let mut cs = constraints::Constraints::load(&wd)?;
        if let Some(text) = &opts.constraints_text {
            if !text.trim().is_empty() {
                util::write_atomic(
                    &wd.constraints_dir().join("installer.md"),
                    format!("{}\n", text.trim()),
                )?;
                for line in text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                {
                    cs.add(constraints::Source::Installer, None, None, line, None)?;
                }
            }
        }
        cs.import_packer(&wd.vpak_dir().join("constraints"))?;
        cs.save(&wd)?;

        // First journal entry and audit line.
        let mut j = journal::Journal::load(&wd)?;
        j.append(
            &wd,
            Phase::Locate,
            "install created",
            &format!(
                "Install `{}` of {} {} created.\n\nArchive: `{}`\nDestination: `{}`\nRunner: `{}`\nMode: `{}`\n",
                state.id,
                state.vpak_name,
                state.vpak_version,
                state.vpak_path.display(),
                state.dest.display(),
                state.runner,
                state.mode.as_str()
            ),
            "vpak",
        )?;
        audit::record(
            &wd,
            "vpak",
            "install.init",
            serde_json::json!({"id": state.id, "vpak": state.vpak_path, "dest": state.dest}),
            "ok",
        )?;
        journal::regenerate_current(&wd)?;
        Ok((wd, manifest))
    }

    pub fn state(&self) -> Result<InstallState> {
        let text = util::read_to_string(&self.install_file())?;
        toml::from_str(&text).context("parse install.toml")
    }

    pub fn save_state(&self, state: &InstallState) -> Result<()> {
        let mut s = state.clone();
        s.updated = util::now();
        util::write_atomic(&self.install_file(), toml::to_string_pretty(&s)?)
    }

    pub fn manifest(&self) -> Result<Manifest> {
        Manifest::load(&self.vpak_dir().join(crate::manifest::MANIFEST_FILE))
    }

    pub fn policy(&self) -> Result<Policy> {
        if self.policy_file().is_file() {
            Policy::load(&self.policy_file())
        } else {
            Ok(Policy::default())
        }
    }

    pub fn save_policy(&self, p: &Policy) -> Result<()> {
        util::write_atomic(&self.policy_file(), p.to_toml()?)
    }

    /// Move to a phase. Any transition is allowed (loop-backs included); the
    /// move is journaled and audited. Moving to `done` sets the done status.
    pub fn set_phase(&self, to: Phase, reason: Option<&str>, actor: &str) -> Result<InstallState> {
        self.set_phase_opts(to, reason, actor, false)
    }

    /// Like `set_phase`, but `force` lets the install be marked done while
    /// plan steps are still pending. Without it, `done` is refused so an
    /// install cannot silently finish with unexecuted steps.
    pub fn set_phase_opts(
        &self,
        to: Phase,
        reason: Option<&str>,
        actor: &str,
        force: bool,
    ) -> Result<InstallState> {
        let mut st = self.state()?;
        let from = st.phase;
        if to.is_terminal() && !force {
            let plan = plan::Plan::load(self)?;
            let pending = plan.pending();
            if !pending.is_empty() {
                let list: Vec<String> = pending
                    .iter()
                    .map(|s| format!("{} {}", s.n, s.step))
                    .collect();
                audit::record(
                    self,
                    actor,
                    "phase.set",
                    serde_json::json!({"from": from, "to": to, "reason": reason}),
                    "refused: pending plan steps",
                )?;
                bail!(
                    "refusing to mark the install done: {} plan step(s) still pending ({}). \
Mark each with `vpak plan mark <n> --status done|failed|skipped --result ...`, \
or pass --force to finish anyway.",
                    pending.len(),
                    list.join("; ")
                );
            }
        }
        st.phase = to;
        st.stalls = 0;
        st.status = if to.is_terminal() {
            Status::Done
        } else {
            Status::Pending
        };
        self.save_state(&st)?;
        let mut j = journal::Journal::load(self)?;
        let body = match reason {
            Some(r) => format!("Phase {from} → {to}.\n\nReason: {r}\n"),
            None => format!("Phase {from} → {to}.\n"),
        };
        let title = if to.is_terminal() {
            "install complete".to_string()
        } else {
            format!("enter {to}")
        };
        j.append(self, to, &title, &body, actor)?;
        audit::record(
            self,
            actor,
            "phase.set",
            serde_json::json!({"from": from, "to": to, "reason": reason}),
            "ok",
        )?;
        journal::regenerate_current(self)?;
        Ok(st)
    }

    pub fn set_status(&self, status: Status, actor: &str, why: &str) -> Result<InstallState> {
        let mut st = self.state()?;
        st.status = status;
        self.save_state(&st)?;
        audit::record(
            self,
            actor,
            "status.set",
            serde_json::json!({"status": status, "why": why}),
            "ok",
        )?;
        Ok(st)
    }

    /// The current actor name for audit lines: `VPAK_ACTOR`, else `cli`.
    pub fn actor() -> String {
        std::env::var("VPAK_ACTOR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "cli".to_string())
    }

    /// Find existing installs under `<cwd>/.vpak/` whose digest matches.
    pub fn find_resumable(base: &Path, digest: &str) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let dir = base.join(".vpak");
        if !dir.is_dir() {
            return Ok(out);
        }
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let p = e.path();
            if !p.join(INSTALL_FILE).is_file() {
                continue;
            }
            if let Ok(wd) = Workdir::open(&p) {
                if let Ok(st) = wd.state() {
                    if st.vpak_digest == digest && st.status != Status::Done {
                        out.push(p);
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

/// A one-screen summary of an install, for `vpak status`.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub id: String,
    pub vpak: String,
    pub phase: Phase,
    pub status: Status,
    pub mode: Mode,
    pub runner: String,
    pub workdir: PathBuf,
    pub dest: PathBuf,
    pub facts: usize,
    pub constraints: usize,
    pub open_conflicts: usize,
    pub mappings: usize,
    pub steps_total: usize,
    pub steps_done: usize,
    pub open_questions: Vec<questions::Question>,
    pub journal_entries: usize,
    pub fleet: Option<PathBuf>,
}

pub fn status_report(wd: &Workdir) -> Result<StatusReport> {
    let st = wd.state()?;
    let facts = survey::Facts::load(wd)?;
    let cs = constraints::Constraints::load(wd)?;
    let plan = plan::Plan::load(wd)?;
    let qs = questions::open(wd)?;
    let j = journal::Journal::load(wd)?;
    let fleet = crate::fleet::handoff_record(wd)?.map(|h| h.path);
    Ok(StatusReport {
        id: st.id,
        vpak: format!("{} {}", st.vpak_name, st.vpak_version),
        phase: st.phase,
        status: st.status,
        mode: st.mode,
        runner: st.runner,
        workdir: wd.root().to_path_buf(),
        dest: st.dest,
        facts: facts.facts.len(),
        constraints: cs.constraints.len(),
        open_conflicts: cs
            .conflicts
            .iter()
            .filter(|c| c.status == constraints::ConflictStatus::Open)
            .count(),
        mappings: plan.mappings.len(),
        steps_total: plan.steps.len(),
        steps_done: plan
            .steps
            .iter()
            .filter(|s| s.status == plan::StepStatus::Done)
            .count(),
        open_questions: qs,
        journal_entries: j.entries.len(),
        fleet,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_order_and_parse() {
        assert_eq!(Phase::Locate.next(), Some(Phase::Survey));
        assert_eq!(Phase::Verify.next(), Some(Phase::Done));
        assert_eq!(Phase::Done.next(), None);
        assert_eq!("Survey".parse::<Phase>().unwrap(), Phase::Survey);
        assert!("nope".parse::<Phase>().is_err());
    }
}
