//! Headless Claude Code as a runner: `claude -p` with the policy mapped onto
//! flags and an inline `--settings` JSON carrying the `autoMode` block.

use std::process::Command;

use anyhow::Result;
use serde_json::Value;

use super::{process, Availability, Denial, RunEvent, RunOutcome, RunRequest, Runner};
use crate::util;

pub struct ClaudeRunner {
    bin: String,
}

impl ClaudeRunner {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    /// The inline settings JSON: classifier rules as Claude Code's `autoMode`.
    pub fn settings_json(req: &RunRequest) -> String {
        serde_json::json!({ "autoMode": req.policy.auto_mode_json() }).to_string()
    }

    /// Argument vector (without the program) for a request.
    pub fn build_args(req: &RunRequest) -> Vec<String> {
        let mut a: Vec<String> = vec!["-p".into()];
        a.push("--permission-mode".into());
        a.push(req.policy.permission_mode.clone());
        a.push("--append-system-prompt-file".into());
        a.push(req.system_prompt_file.display().to_string());
        if !req.policy.allowed_tools.is_empty() {
            a.push("--allowedTools".into());
            a.push(req.policy.allowed_tools.join(","));
        }
        if !req.policy.disallowed_tools.is_empty() {
            a.push("--disallowedTools".into());
            a.push(req.policy.disallowed_tools.join(","));
        }
        a.push("--add-dir".into());
        a.push(req.cwd.display().to_string());
        for d in &req.extra_dirs {
            a.push("--add-dir".into());
            a.push(d.display().to_string());
        }
        a.push("--settings".into());
        a.push(Self::settings_json(req));
        a.push("--permission-prompts".into());
        a.push("none".into());
        a.push("--max-turns".into());
        a.push(req.budget.max_turns.to_string());
        if let Some(s) = &req.session {
            a.push("--resume".into());
            a.push(s.clone());
        }
        a.push("--output-format".into());
        a.push("stream-json".into());
        a.push("--verbose".into());
        a
    }

    pub fn build_command(&self, req: &RunRequest) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.args(Self::build_args(req));
        cmd.current_dir(&req.cwd);
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        cmd
    }
}

/// Summarize one stream-json event for the audit log, and fold result fields
/// into the outcome.
fn absorb(v: &Value, out: &mut RunOutcome) -> RunEvent {
    let t = v.get("type").and_then(Value::as_str).unwrap_or("unknown");
    let sub = v.get("subtype").and_then(Value::as_str).unwrap_or("");
    if let Some(s) = v.get("session_id").and_then(Value::as_str) {
        out.session_id = Some(s.to_string());
    }
    match t {
        "assistant" => {
            let mut parts = Vec::new();
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for c in content {
                    match c.get("type").and_then(Value::as_str) {
                        Some("text") => parts.push(util::truncate(
                            c.get("text").and_then(Value::as_str).unwrap_or(""),
                            400,
                        )),
                        Some("tool_use") => {
                            let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
                            let input = c
                                .get("input")
                                .map(|i| util::truncate(&i.to_string(), 200))
                                .unwrap_or_default();
                            parts.push(format!("tool_use {name} {input}"));
                        }
                        _ => {}
                    }
                }
            }
            RunEvent {
                kind: "assistant".into(),
                summary: parts.join(" | "),
            }
        }
        "user" => {
            let n = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            RunEvent {
                kind: "tool_result".into(),
                summary: format!("{n} block(s)"),
            }
        }
        "result" => {
            out.result_text = v
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            out.cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
            out.num_turns = v.get("num_turns").and_then(Value::as_u64);
            out.is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                || sub.contains("error");
            if let Some(d) = v.get("permission_denials").and_then(Value::as_array) {
                for x in d {
                    let tool = x
                        .get("tool_name")
                        .or_else(|| x.get("tool"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string();
                    let detail = x
                        .get("tool_input")
                        .map(|i| util::truncate(&i.to_string(), 300))
                        .or_else(|| {
                            x.get("reason")
                                .and_then(Value::as_str)
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default();
                    out.denials.push(Denial { tool, detail });
                }
            }
            RunEvent {
                kind: format!(
                    "result{}",
                    if sub.is_empty() {
                        String::new()
                    } else {
                        format!(":{sub}")
                    }
                ),
                summary: util::truncate(&out.result_text, 400),
            }
        }
        "permission_denied" => {
            let tool = v
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let reason = v
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            out.denials.push(Denial {
                tool: tool.clone(),
                detail: reason.clone(),
            });
            RunEvent {
                kind: "permission_denied".into(),
                summary: format!("{tool}: {reason}"),
            }
        }
        "system" => RunEvent {
            kind: format!("system:{sub}"),
            summary: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        other => RunEvent {
            kind: other.to_string(),
            summary: String::new(),
        },
    }
}

impl Runner for ClaudeRunner {
    fn name(&self) -> &str {
        "claude"
    }

    fn available(&self) -> Result<Availability> {
        match super::exists_on_path(&self.bin) {
            Some(p) => {
                let v = Command::new(&p)
                    .arg("--version")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                Ok(Availability::Available {
                    detail: format!("{} {}", p.display(), v),
                })
            }
            None => Ok(Availability::Unavailable {
                reason: format!(
                    "'{}' not found on PATH (set VPAK_CLAUDE_BIN or install Claude Code)",
                    self.bin
                ),
            }),
        }
    }

    fn run(&self, req: &RunRequest, sink: &mut dyn FnMut(RunEvent)) -> Result<RunOutcome> {
        let mut cmd = self.build_command(req);
        let mut out = RunOutcome::default();
        let mut on_line = |line: &str| {
            let line = line.trim();
            if line.is_empty() {
                return;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(v) => sink(absorb(&v, &mut out)),
                Err(_) => sink(RunEvent {
                    kind: "stdout".into(),
                    summary: util::truncate(line, 400),
                }),
            }
        };
        let fin = process::run_streaming(
            &mut cmd,
            Some(&req.prompt),
            req.budget.wall_clock_duration(),
            &mut on_line,
        )?;
        out.exit_code = fin.exit_code;
        out.timed_out = fin.timed_out;
        out.stderr_tail = fin.stderr_tail;
        if out.exit_code != Some(0) && !out.is_error {
            out.is_error = true;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Budget, Classifier, Policy};
    use std::path::PathBuf;

    fn req() -> RunRequest {
        RunRequest {
            system_prompt_file: PathBuf::from("/w/bootstrap/compiled.md"),
            prompt: "do the phase".into(),
            cwd: PathBuf::from("/w"),
            extra_dirs: vec![PathBuf::from("/d")],
            policy: Policy {
                permission_mode: "auto".into(),
                allowed_tools: vec!["Read".into(), "Bash(vpak *)".into()],
                disallowed_tools: vec!["Bash(rm -rf *)".into()],
                classifier: Classifier {
                    environment: vec!["env".into()],
                    allow: vec!["a".into()],
                    soft_deny: vec!["s".into()],
                    hard_deny: vec!["h".into()],
                },
                budget: Budget::default(),
            },
            budget: Budget {
                max_turns: 12,
                wall_clock: "1m".into(),
            },
            session: Some("sess-1".into()),
            env: vec![],
        }
    }

    #[test]
    fn argv_shape() {
        let a = ClaudeRunner::build_args(&req());
        let s = a.join(" ");
        assert!(a[0] == "-p");
        assert!(s.contains("--permission-mode auto"));
        assert!(s.contains("--append-system-prompt-file"));
        assert!(s.contains("--allowedTools Read,Bash(vpak *)"));
        assert!(s.contains("--disallowedTools Bash(rm -rf *)"));
        assert!(
            s.contains("--add-dir /w --add-dir /d")
                || s.contains("--add-dir /w") && s.contains("--add-dir /d")
        );
        assert!(s.contains("--permission-prompts none"));
        assert!(s.contains("--max-turns 12"));
        assert!(s.contains("--resume sess-1"));
        assert!(s.contains("--output-format stream-json"));
        // No positional prompt: it goes on stdin.
        assert!(!a.contains(&"do the phase".to_string()));
    }

    #[test]
    fn settings_json_carries_auto_mode() {
        let j: Value = serde_json::from_str(&ClaudeRunner::settings_json(&req())).unwrap();
        assert_eq!(j["autoMode"]["environment"][0], "env");
        assert_eq!(j["autoMode"]["allow"][0], "a");
        assert_eq!(j["autoMode"]["soft_deny"][0], "s");
        assert_eq!(j["autoMode"]["hard_deny"][0], "h");
    }

    #[test]
    fn absorbs_result_event() {
        let mut out = RunOutcome::default();
        let v: Value = serde_json::json!({
            "type": "result", "subtype": "success", "result": "done", "session_id": "abc",
            "total_cost_usd": 0.42, "num_turns": 7, "is_error": false,
            "permission_denials": [{"tool_name": "Bash", "tool_input": {"command": "aws iam create-user"}}]
        });
        let ev = absorb(&v, &mut out);
        assert_eq!(ev.kind, "result:success");
        assert_eq!(out.session_id.as_deref(), Some("abc"));
        assert_eq!(out.cost_usd, Some(0.42));
        assert_eq!(out.num_turns, Some(7));
        assert_eq!(out.denials.len(), 1);
        assert_eq!(out.denials[0].tool, "Bash");
        assert!(out.denials[0].detail.contains("create-user"));
    }
}
