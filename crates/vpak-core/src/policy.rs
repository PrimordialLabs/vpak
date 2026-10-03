//! Runner policy: what the agent may do, and advisory rules for its
//! permission classifier. Shipped by the packer in `bootstrap/policy.toml`,
//! refined at install time in the workdir's `policy.toml`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default = "default_mode")]
    pub permission_mode: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub classifier: Classifier,
    #[serde(default)]
    pub budget: Budget,
}

fn default_mode() -> String {
    "auto".to_string()
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            permission_mode: default_mode(),
            allowed_tools: vec![
                "Read".into(),
                "Grep".into(),
                "Glob".into(),
                "Bash(vpak *)".into(),
            ],
            disallowed_tools: vec!["Bash(rm -rf *)".into()],
            classifier: Classifier::default(),
            budget: Budget::default(),
        }
    }
}

/// Advisory rules for the runner's permission classifier. The claude runner
/// maps these onto Claude Code's `autoMode` settings block.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Classifier {
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub soft_deny: Vec<String>,
    #[serde(default)]
    pub hard_deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(default = "default_turns")]
    pub max_turns: u32,
    /// Human duration such as `45m` or `2h`. Empty means no limit.
    #[serde(default)]
    pub wall_clock: String,
}

fn default_turns() -> u32 {
    60
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_turns: default_turns(),
            wall_clock: String::new(),
        }
    }
}

impl Budget {
    pub fn wall_clock_duration(&self) -> Option<Duration> {
        parse_duration(&self.wall_clock)
    }
}

/// Parse `30s`, `45m`, `2h`, `1d`, or a bare number of seconds.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num, unit) = match s.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, "s"),
    };
    let n: u64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "s" | "sec" => 1,
        "m" | "min" => 60,
        "h" | "hr" => 3600,
        "d" => 86400,
        _ => return None,
    };
    Some(Duration::from_secs(n * mult))
}

impl Policy {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("parse policy.toml")
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// The `autoMode` block Claude Code reads from settings JSON.
    pub fn auto_mode_json(&self) -> serde_json::Value {
        serde_json::json!({
            "environment": self.classifier.environment,
            "allow": self.classifier.allow,
            "soft_deny": self.classifier.soft_deny,
            "hard_deny": self.classifier.hard_deny,
        })
    }

    pub fn allow(&mut self, tool: &str) {
        if !self.allowed_tools.iter().any(|t| t == tool) {
            self.allowed_tools.push(tool.to_string());
        }
        self.disallowed_tools.retain(|t| t != tool);
    }

    pub fn deny(&mut self, tool: &str) {
        if !self.disallowed_tools.iter().any(|t| t == tool) {
            self.disallowed_tools.push(tool.to_string());
        }
        self.allowed_tools.retain(|t| t != tool);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_design_example() {
        let text = r#"
permission_mode  = "auto"
allowed_tools    = ["Read", "Bash(vpak *)"]
disallowed_tools = ["Bash(rm -rf *)"]

[classifier]
environment = ["Survey phase is read-only."]
allow       = ["Read-only describe calls."]
soft_deny   = ["Any apply before execute."]
hard_deny   = ["Printing credential material."]

[budget]
max_turns = 40
wall_clock = "45m"
"#;
        let p = Policy::parse(text).unwrap();
        assert_eq!(p.permission_mode, "auto");
        assert_eq!(p.classifier.hard_deny.len(), 1);
        assert_eq!(p.budget.max_turns, 40);
        assert_eq!(
            p.budget.wall_clock_duration(),
            Some(Duration::from_secs(45 * 60))
        );
        let j = p.auto_mode_json();
        assert_eq!(j["soft_deny"][0], "Any apply before execute.");
        let back = Policy::parse(&p.to_toml().unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("3weeks"), None);
    }

    #[test]
    fn allow_deny_toggle() {
        let mut p = Policy::default();
        p.deny("Bash(vpak *)");
        assert!(!p.allowed_tools.contains(&"Bash(vpak *)".to_string()));
        p.allow("Bash(vpak *)");
        assert!(p.allowed_tools.contains(&"Bash(vpak *)".to_string()));
        assert!(!p.disallowed_tools.contains(&"Bash(vpak *)".to_string()));
    }
}
