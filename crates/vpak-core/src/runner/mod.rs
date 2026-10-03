//! A runner executes one agent turn. Headless Claude Code is the default;
//! any process that can be told what to do can be a runner.

pub mod claude;
pub mod process;
pub mod shell;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::policy::{Budget, Policy};

#[derive(Debug, Clone, Serialize)]
pub enum Availability {
    Available { detail: String },
    Unavailable { reason: String },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available { .. })
    }
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    /// File holding the compiled system prompt.
    pub system_prompt_file: PathBuf,
    /// The turn's instruction (phase prompt). Delivered on stdin.
    pub prompt: String,
    pub cwd: PathBuf,
    pub extra_dirs: Vec<PathBuf>,
    pub policy: Policy,
    pub budget: Budget,
    pub session: Option<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Denial {
    pub tool: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunOutcome {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub result_text: String,
    pub session_id: Option<String>,
    pub cost_usd: Option<f64>,
    pub num_turns: Option<u64>,
    pub is_error: bool,
    pub denials: Vec<Denial>,
    pub stderr_tail: String,
}

/// A compact view of one runner event, mirrored into the audit log.
#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub kind: String,
    pub summary: String,
}

pub trait Runner: Send + Sync {
    fn name(&self) -> &str;
    fn available(&self) -> Result<Availability>;
    fn run(&self, req: &RunRequest, sink: &mut dyn FnMut(RunEvent)) -> Result<RunOutcome>;
}

/// `~/.config/vpak/config.toml` (or `VPAK_CONFIG`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default_runner: Option<String>,
    #[serde(default)]
    pub runners: BTreeMap<String, RunnerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    /// `claude` or `shell`.
    pub kind: String,
    /// Executable for `claude`; ignored for `shell`.
    #[serde(default)]
    pub bin: Option<String>,
    /// Command template for `shell`, split shell-style without a shell.
    #[serde(default)]
    pub command: Option<String>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("VPAK_CONFIG") {
            return Some(PathBuf::from(p));
        }
        dirs::config_dir().map(|d| d.join("vpak").join("config.toml"))
    }

    pub fn load() -> Result<Self> {
        match Self::path() {
            Some(p) if p.is_file() => Ok(toml::from_str(&crate::util::read_to_string(&p)?)?),
            _ => Ok(Self::default()),
        }
    }
}

/// Build a runner by name from config, environment and built-ins.
/// Environment entries every runner launch should carry so the agent's
/// `vpak ...` calls hit the exact binary that is driving the install:
/// `VPAK_BIN` and a `PATH` with that binary's directory prepended.
pub fn self_env() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(exe) = std::env::current_exe() else {
        return out;
    };
    out.push(("VPAK_BIN".into(), exe.display().to_string()));
    if let Some(dir) = exe.parent() {
        let mut paths: Vec<PathBuf> = vec![dir.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            out.push(("PATH".into(), joined.to_string_lossy().into_owned()));
        }
    }
    out
}

pub fn by_name(name: &str, config: &Config) -> Result<Box<dyn Runner>> {
    if let Some(rc) = config.runners.get(name) {
        return match rc.kind.as_str() {
            "claude" => Ok(Box::new(claude::ClaudeRunner::new(
                rc.bin.clone().unwrap_or_else(|| "claude".into()),
            ))),
            "shell" => match &rc.command {
                Some(c) => Ok(Box::new(shell::ShellRunner::new(name, c))),
                None => bail!("runner '{name}' is kind shell but has no command"),
            },
            other => bail!("runner '{name}' has unknown kind '{other}'"),
        };
    }
    match name {
        "claude" => Ok(Box::new(claude::ClaudeRunner::new(
            std::env::var("VPAK_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()),
        ))),
        "shell" => match std::env::var("VPAK_SHELL_RUNNER_CMD") {
            Ok(c) if !c.trim().is_empty() => Ok(Box::new(shell::ShellRunner::new("shell", &c))),
            _ => bail!(
                "the shell runner needs VPAK_SHELL_RUNNER_CMD or [runners.shell] command in config"
            ),
        },
        other => bail!(
            "unknown runner '{other}'; known: claude, shell, and any [runners.<name>] in {}",
            Config::path()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
    }
}

/// Names of every runner this process knows about, with availability.
pub fn survey(config: &Config) -> Vec<(String, Availability)> {
    let mut names: Vec<String> = vec!["claude".into(), "shell".into()];
    for k in config.runners.keys() {
        if !names.contains(k) {
            names.push(k.clone());
        }
    }
    names
        .into_iter()
        .map(|n| {
            let av = match by_name(&n, config) {
                Ok(r) => r.available().unwrap_or_else(|e| Availability::Unavailable {
                    reason: e.to_string(),
                }),
                Err(e) => Availability::Unavailable {
                    reason: e.to_string(),
                },
            };
            (n, av)
        })
        .collect()
}

pub(crate) fn exists_on_path(bin: &str) -> Option<PathBuf> {
    let p = Path::new(bin);
    if p.components().count() > 1 && p.is_file() {
        return Some(p.to_path_buf());
    }
    which::which(bin).ok()
}
