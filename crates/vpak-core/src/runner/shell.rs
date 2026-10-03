//! A runner that launches any command template with the prompt on stdin.
//! Used for tests and for wiring up other agent harnesses. The template is
//! split shell-style and spawned directly; no shell is involved.

use std::process::Command;

use anyhow::Result;

use super::{process, Availability, RunEvent, RunOutcome, RunRequest, Runner};
use crate::util;

pub struct ShellRunner {
    name: String,
    template: String,
}

impl ShellRunner {
    pub fn new(name: &str, template: &str) -> Self {
        Self {
            name: name.to_string(),
            template: template.to_string(),
        }
    }

    pub fn argv(&self) -> Result<Vec<String>> {
        util::split_argv(&self.template)
    }

    pub fn build_command(&self, req: &RunRequest) -> Result<Command> {
        let argv = self.argv()?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.current_dir(&req.cwd);
        cmd.env("VPAK_SYSTEM_PROMPT_FILE", &req.system_prompt_file);
        cmd.env("VPAK_MAX_TURNS", req.budget.max_turns.to_string());
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        Ok(cmd)
    }
}

impl Runner for ShellRunner {
    fn name(&self) -> &str {
        &self.name
    }

    fn available(&self) -> Result<Availability> {
        let argv = self.argv()?;
        match super::exists_on_path(&argv[0]) {
            Some(p) => Ok(Availability::Available {
                detail: format!("{} ({})", p.display(), self.template),
            }),
            None => Ok(Availability::Unavailable {
                reason: format!("'{}' not found", argv[0]),
            }),
        }
    }

    fn run(&self, req: &RunRequest, sink: &mut dyn FnMut(RunEvent)) -> Result<RunOutcome> {
        let mut cmd = self.build_command(req)?;
        let mut out = RunOutcome::default();
        let mut text = String::new();
        let mut on_line = |line: &str| {
            text.push_str(line);
            text.push('\n');
            sink(RunEvent {
                kind: "stdout".into(),
                summary: util::truncate(line, 400),
            });
        };
        let fin = process::run_streaming(
            &mut cmd,
            Some(&req.prompt),
            req.budget.wall_clock_duration(),
            &mut on_line,
        )?;
        out.result_text = text;
        out.exit_code = fin.exit_code;
        out.timed_out = fin.timed_out;
        out.stderr_tail = fin.stderr_tail;
        out.is_error = fin.exit_code != Some(0);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_template_without_shell() {
        let r = ShellRunner::new("t", r#"python3 "my agent.py" --flag 'x y'"#);
        assert_eq!(
            r.argv().unwrap(),
            vec!["python3", "my agent.py", "--flag", "x y"]
        );
        assert!(ShellRunner::new("t", "unbalanced 'quote").argv().is_err());
    }
}
