//! The installer's target: prose (`target/target.md`) plus a structured table
//! (`target/target.toml`) that grows as the bootstrap learns.

use anyhow::{bail, Result};
use toml::Value;

use super::Workdir;
use crate::util;

#[derive(Debug, Clone, Default)]
pub struct Target {
    pub prose: String,
    /// The `[target]` table.
    pub table: toml::Table,
}

impl Target {
    pub fn load(wd: &Workdir) -> Result<Self> {
        let md = wd.target_dir().join("target.md");
        let tm = wd.target_dir().join("target.toml");
        let prose = if md.is_file() {
            util::read_to_string(&md)?.replace("\r\n", "\n")
        } else {
            String::new()
        };
        let table = if tm.is_file() {
            let doc: toml::Table = toml::from_str(&util::read_to_string(&tm)?)?;
            match doc.get("target") {
                Some(Value::Table(t)) => t.clone(),
                _ => toml::Table::new(),
            }
        } else {
            toml::Table::new()
        };
        Ok(Self { prose, table })
    }

    pub fn save(&self, wd: &Workdir) -> Result<()> {
        util::write_atomic(&wd.target_dir().join("target.md"), &self.prose)?;
        let mut doc = toml::Table::new();
        doc.insert("target".into(), Value::Table(self.table.clone()));
        util::write_atomic(
            &wd.target_dir().join("target.toml"),
            toml::to_string_pretty(&doc)?,
        )
    }

    /// Set a dotted key such as `access.credentials` or `mapping.cloud-run`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let parts: Vec<&str> = key
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            bail!("empty target key");
        }
        let mut cur = &mut self.table;
        for p in &parts[..parts.len() - 1] {
            let entry = cur
                .entry(p.to_string())
                .or_insert_with(|| Value::Table(toml::Table::new()));
            if !entry.is_table() {
                *entry = Value::Table(toml::Table::new());
            }
            cur = entry.as_table_mut().expect("table");
        }
        cur.insert(
            parts[parts.len() - 1].to_string(),
            Value::String(value.to_string()),
        );
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let mut cur: &Value = &Value::Table(self.table.clone());
        for p in key.split('.') {
            cur = cur.get(p)?;
        }
        cur.as_str().map(|s| s.to_string())
    }

    /// Append a dated note to the prose.
    pub fn note(&mut self, text: &str) {
        if !self.prose.is_empty() && !self.prose.ends_with('\n') {
            self.prose.push('\n');
        }
        self.prose.push_str(&format!(
            "\n_{}_: {}\n",
            util::now().format("%Y-%m-%d %H:%M UTC"),
            text.trim()
        ));
    }

    /// `key = "value"` lines, dotted, sorted, for the compacted view.
    pub fn render_flat(&self) -> String {
        let mut lines = Vec::new();
        flatten("", &self.table, &mut lines);
        lines.sort();
        lines.into_iter().map(|l| l + "\n").collect()
    }
}

fn flatten(prefix: &str, t: &toml::Table, out: &mut Vec<String>) {
    for (k, v) in t {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        match v {
            Value::Table(inner) => flatten(&key, inner, out),
            Value::String(s) => out.push(format!("{key} = \"{s}\"")),
            other => out.push(format!("{key} = {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_set_get_flatten() {
        let mut t = Target::default();
        t.set("provider", "aws").unwrap();
        t.set("access.credentials", "existing-profile").unwrap();
        t.set("mapping.cloud-run", "ecs-fargate").unwrap();
        assert_eq!(t.get("provider").as_deref(), Some("aws"));
        assert_eq!(
            t.get("access.credentials").as_deref(),
            Some("existing-profile")
        );
        assert_eq!(t.get("mapping.cloud-run").as_deref(), Some("ecs-fargate"));
        assert!(t.get("nope.x").is_none());
        let flat = t.render_flat();
        assert!(flat.contains("mapping.cloud-run = \"ecs-fargate\""));
        assert!(flat.contains("provider = \"aws\""));
        t.note("Use us-east-1.");
        assert!(t.prose.contains("Use us-east-1."));
    }
}
