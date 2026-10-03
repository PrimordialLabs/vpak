//! The `vpak.toml` manifest at the root of every archive.

use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "vpak.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    pub version: String,
    pub created: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packer: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub reference: ReferenceSection,
    #[serde(default)]
    pub intent: IntentSection,
    #[serde(default)]
    pub bootstrap: BootstrapSection,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceSection {
    #[serde(default = "default_reference_root")]
    pub root: String,
    /// Free-form hint of the reference environment, e.g. `gcp`, `aws`, `local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Paths under `root` that a reader should look at first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry: Vec<String>,
}

impl Default for ReferenceSection {
    fn default() -> Self {
        Self {
            root: default_reference_root(),
            kind: None,
            entry: Vec::new(),
        }
    }
}

fn default_reference_root() -> String {
    "reference/".to_string()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IntentSection {
    /// Intent files under `intent/`, in reading order.
    #[serde(default)]
    pub order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BootstrapSection {
    #[serde(default = "default_seed")]
    pub seed: String,
    #[serde(default = "default_policy")]
    pub policy: String,
}

impl Default for BootstrapSection {
    fn default() -> Self {
        Self {
            seed: default_seed(),
            policy: default_policy(),
        }
    }
}

fn default_seed() -> String {
    "bootstrap/seed.md".to_string()
}
fn default_policy() -> String {
    "bootstrap/policy.toml".to_string()
}

/// A target the packer suggests, or one realized by a prior install.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetEntry {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    pub path: String,
}

impl Manifest {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            format: FORMAT_VERSION,
            name: name.to_string(),
            version: version.to_string(),
            created: Utc::now(),
            packer: None,
            summary: String::new(),
            reference: ReferenceSection::default(),
            intent: IntentSection::default(),
            bootstrap: BootstrapSection::default(),
            targets: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let m: Manifest = toml::from_str(text).context("parse vpak.toml")?;
        m.validate()?;
        Ok(m)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT_VERSION {
            bail!(
                "unsupported vpak format {} (this build reads format {})",
                self.format,
                FORMAT_VERSION
            );
        }
        if self.name.trim().is_empty() {
            bail!("vpak.toml: name is required");
        }
        if self.name.contains('/') || self.name.contains("..") {
            bail!("vpak.toml: name must not contain path separators");
        }
        if self.version.trim().is_empty() {
            bail!("vpak.toml: version is required");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut m = Manifest::new("payments-service", "0.3.0");
        m.packer = Some("someone@example.com".into());
        m.summary = "Payments".into();
        m.reference.kind = Some("gcp".into());
        m.reference.entry = vec!["terraform/".into(), "Dockerfile".into()];
        m.intent.order = vec!["00-overview.md".into(), "10-auth.md".into()];
        m.targets.push(TargetEntry {
            name: "origin".into(),
            summary: "as shipped".into(),
            path: "targets/origin.toml".into(),
        });
        let text = m.to_toml().unwrap();
        let back = Manifest::parse(&text).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn defaults_fill_in() {
        let text = r#"
format = 1
name = "x"
version = "1.0.0"
created = "2026-10-02T21:00:00Z"
"#;
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.reference.root, "reference/");
        assert_eq!(m.bootstrap.seed, "bootstrap/seed.md");
        assert!(m.targets.is_empty());
    }

    #[test]
    fn rejects_bad_format() {
        let text =
            "format = 2\nname = \"x\"\nversion = \"1\"\ncreated = \"2026-10-02T21:00:00Z\"\n";
        assert!(Manifest::parse(text).is_err());
    }
}
