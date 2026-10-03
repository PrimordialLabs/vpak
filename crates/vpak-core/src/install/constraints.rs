//! Constraints with provenance and precedence: installer > packer > discovered.
//! `constraints/resolved.toml` holds every constraint; `constraints/conflicts.toml`
//! holds disagreements that a human must settle.

use std::path::Path;
use std::str::FromStr;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Workdir;
use crate::util;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Installer,
    Packer,
    Discovered,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Installer => "installer",
            Source::Packer => "packer",
            Source::Discovered => "discovered",
        }
    }
    /// Higher wins.
    pub fn precedence(&self) -> u8 {
        match self {
            Source::Installer => 3,
            Source::Packer => 2,
            Source::Discovered => 1,
        }
    }
}

impl FromStr for Source {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "installer" => Ok(Source::Installer),
            "packer" => Ok(Source::Packer),
            "discovered" => Ok(Source::Discovered),
            _ => bail!("unknown constraint source '{s}'; expected installer, packer or discovered"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub text: String,
    pub ts: DateTime<Utc>,
    /// Set when a higher-precedence constraint on the same key replaced this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictStatus {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub a: String,
    pub b: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub status: ConflictStatus,
    #[serde(default)]
    pub kept: String,
    #[serde(default)]
    pub resolution: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ResolvedFile {
    #[serde(default, rename = "constraint")]
    constraints: Vec<Constraint>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ConflictsFile {
    #[serde(default, rename = "conflict")]
    conflicts: Vec<Conflict>,
}

/// Typed packer constraints shipped in `constraints/constraints.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackerConstraintsFile {
    #[serde(default, rename = "constraint")]
    pub constraints: Vec<PackerConstraint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackerConstraint {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Constraints {
    pub constraints: Vec<Constraint>,
    pub conflicts: Vec<Conflict>,
}

/// What happened when a constraint was added.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum AddOutcome {
    Added,
    /// The new constraint superseded an existing lower-precedence one.
    Superseded {
        previous: String,
    },
    /// The new constraint conflicts with an existing one; a conflict was recorded.
    Conflict {
        with: String,
        conflict_id: String,
    },
}

impl Constraints {
    pub fn load(wd: &Workdir) -> Result<Self> {
        let r = wd.constraints_dir().join("resolved.toml");
        let c = wd.constraints_dir().join("conflicts.toml");
        let constraints = if r.is_file() {
            toml::from_str::<ResolvedFile>(&util::read_to_string(&r)?)?.constraints
        } else {
            Vec::new()
        };
        let conflicts = if c.is_file() {
            toml::from_str::<ConflictsFile>(&util::read_to_string(&c)?)?.conflicts
        } else {
            Vec::new()
        };
        Ok(Self {
            constraints,
            conflicts,
        })
    }

    pub fn save(&self, wd: &Workdir) -> Result<()> {
        util::write_atomic(
            &wd.constraints_dir().join("resolved.toml"),
            toml::to_string_pretty(&ResolvedFile {
                constraints: self.constraints.clone(),
            })?,
        )?;
        util::write_atomic(
            &wd.constraints_dir().join("conflicts.toml"),
            toml::to_string_pretty(&ConflictsFile {
                conflicts: self.conflicts.clone(),
            })?,
        )
    }

    fn next_id(&self) -> String {
        format!("c-{:04}", self.constraints.len() + 1)
    }

    fn next_conflict_id(&self) -> String {
        format!("x-{:04}", self.conflicts.len() + 1)
    }

    /// Constraints not superseded.
    pub fn active(&self) -> Vec<&Constraint> {
        self.constraints
            .iter()
            .filter(|c| c.superseded_by.is_none())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Constraint> {
        self.constraints.iter().find(|c| c.id == id)
    }

    /// Add a constraint, applying precedence. Same `key` with a different
    /// `value` either supersedes (new source outranks) or conflicts (new source
    /// ranks equal or lower). `conflicts_with` forces a conflict with that id.
    pub fn add(
        &mut self,
        source: Source,
        key: Option<&str>,
        value: Option<&str>,
        text: &str,
        conflicts_with: Option<&str>,
    ) -> Result<(String, AddOutcome)> {
        let id = self.next_id();
        let new = Constraint {
            id: id.clone(),
            source,
            key: key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
            value: value.map(|v| v.to_string()),
            text: text.trim().to_string(),
            ts: util::now(),
            superseded_by: None,
        };
        if new.text.is_empty() && new.value.is_none() {
            bail!("a constraint needs --text or --value");
        }

        let mut outcome = AddOutcome::Added;

        if let Some(other_id) = conflicts_with {
            if self.get(other_id).is_none() {
                bail!("no constraint {other_id} to conflict with");
            }
            let cid = self.next_conflict_id();
            self.conflicts.push(Conflict {
                id: cid.clone(),
                a: other_id.to_string(),
                b: id.clone(),
                key: new.key.clone(),
                status: ConflictStatus::Open,
                kept: String::new(),
                resolution: String::new(),
                ts: util::now(),
            });
            outcome = AddOutcome::Conflict {
                with: other_id.to_string(),
                conflict_id: cid,
            };
        } else if let Some(k) = &new.key {
            let existing: Option<usize> = self
                .constraints
                .iter()
                .position(|c| c.superseded_by.is_none() && c.key.as_deref() == Some(k.as_str()));
            if let Some(pos) = existing {
                let old = &self.constraints[pos];
                let same_value =
                    old.value == new.value && (new.value.is_some() || old.text == new.text);
                if !same_value {
                    if new.source.precedence() > old.source.precedence() {
                        let prev = old.id.clone();
                        self.constraints[pos].superseded_by = Some(id.clone());
                        outcome = AddOutcome::Superseded { previous: prev };
                    } else {
                        let cid = self.next_conflict_id();
                        self.conflicts.push(Conflict {
                            id: cid.clone(),
                            a: old.id.clone(),
                            b: id.clone(),
                            key: Some(k.clone()),
                            status: ConflictStatus::Open,
                            kept: String::new(),
                            resolution: String::new(),
                            ts: util::now(),
                        });
                        outcome = AddOutcome::Conflict {
                            with: old.id.clone(),
                            conflict_id: cid,
                        };
                    }
                }
            }
        }
        self.constraints.push(new);
        Ok((id, outcome))
    }

    /// Resolve a conflict by keeping one side; the other is superseded.
    pub fn resolve(&mut self, conflict_id: &str, keep: &str, reason: &str) -> Result<()> {
        let pos = self
            .conflicts
            .iter()
            .position(|c| c.id == conflict_id)
            .ok_or_else(|| anyhow::anyhow!("no conflict {conflict_id}"))?;
        let (a, b) = (self.conflicts[pos].a.clone(), self.conflicts[pos].b.clone());
        let lose = if keep == a {
            b
        } else if keep == b {
            a
        } else {
            bail!("--keep must be {a} or {b}");
        };
        if let Some(c) = self.constraints.iter_mut().find(|c| c.id == lose) {
            c.superseded_by = Some(keep.to_string());
        }
        let c = &mut self.conflicts[pos];
        c.status = ConflictStatus::Resolved;
        c.kept = keep.to_string();
        c.resolution = reason.trim().to_string();
        Ok(())
    }

    fn has_packer_text(&self, text: &str) -> bool {
        let norm = normalize(text);
        self.constraints
            .iter()
            .any(|c| c.source == Source::Packer && normalize(&c.text) == norm)
    }

    /// Import packer constraints from the unpacked archive's `constraints/`.
    /// Typed entries come from `constraints.toml`; every bullet or line of a
    /// prose `*.md` file becomes one constraint. A prose line that restates a
    /// typed entry's text is skipped.
    pub fn import_packer(&mut self, dir: &Path) -> Result<usize> {
        let mut n = 0;
        if !dir.is_dir() {
            return Ok(0);
        }
        let typed = dir.join("constraints.toml");
        if typed.is_file() {
            let f: PackerConstraintsFile = toml::from_str(&util::read_to_string(&typed)?)?;
            for c in f.constraints {
                if self.has_packer_text(&c.text) {
                    continue;
                }
                self.add(
                    Source::Packer,
                    c.key.as_deref(),
                    c.value.as_deref(),
                    &c.text,
                    None,
                )?;
                n += 1;
            }
        }
        let mut mds: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        mds.sort();
        for md in mds {
            let text = util::read_to_string(&md)?;
            for line in text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
            {
                let line = line.trim_start_matches(['-', '*']).trim();
                if line.is_empty() || self.has_packer_text(line) {
                    continue;
                }
                self.add(Source::Packer, None, None, line, None)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packer_import_dedupes_prose_against_typed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("constraints.toml"), "[[constraint]]\nkey = \"tls\"\nvalue = \"managed\"\ntext = \"HTTPS with a managed certificate.\"\n").unwrap();
        std::fs::write(
            tmp.path().join("00-packer.md"),
            "# Packer\n\n- HTTPS with a managed certificate.\n- Runs as non-root.\n",
        )
        .unwrap();
        let mut cs = Constraints::default();
        let n = cs.import_packer(tmp.path()).unwrap();
        assert_eq!(n, 2);
        assert_eq!(cs.constraints.len(), 2);
        assert_eq!(cs.constraints[0].key.as_deref(), Some("tls"));
        assert_eq!(cs.constraints[1].text, "Runs as non-root.");
    }

    #[test]
    fn installer_supersedes_packer() {
        let mut cs = Constraints::default();
        let (p, o) = cs
            .add(
                Source::Packer,
                Some("region"),
                Some("us-central1"),
                "Reference region",
                None,
            )
            .unwrap();
        assert_eq!(o, AddOutcome::Added);
        let (i, o) = cs
            .add(
                Source::Installer,
                Some("region"),
                Some("us-east-1"),
                "Installer region",
                None,
            )
            .unwrap();
        assert_eq!(
            o,
            AddOutcome::Superseded {
                previous: p.clone()
            }
        );
        assert_eq!(
            cs.get(&p).unwrap().superseded_by.as_deref(),
            Some(i.as_str())
        );
        assert_eq!(cs.active().len(), 1);
        assert!(cs.conflicts.is_empty());
    }

    #[test]
    fn discovered_never_overrides_and_conflicts() {
        let mut cs = Constraints::default();
        cs.add(
            Source::Installer,
            Some("region"),
            Some("us-east-1"),
            "Installer region",
            None,
        )
        .unwrap();
        let (_d, o) = cs
            .add(
                Source::Discovered,
                Some("region"),
                Some("eu-west-1"),
                "Only region enabled",
                None,
            )
            .unwrap();
        match o {
            AddOutcome::Conflict { conflict_id, .. } => {
                assert_eq!(cs.conflicts.len(), 1);
                assert_eq!(cs.conflicts[0].status, ConflictStatus::Open);
                // Both remain active until resolved.
                assert_eq!(cs.active().len(), 2);
                cs.resolve(&conflict_id, "c-0001", "installer wins")
                    .unwrap();
                assert_eq!(cs.active().len(), 1);
                assert_eq!(cs.conflicts[0].status, ConflictStatus::Resolved);
            }
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[test]
    fn same_value_is_not_a_conflict() {
        let mut cs = Constraints::default();
        cs.add(Source::Packer, Some("db"), Some("postgres"), "", None)
            .unwrap();
        let (_, o) = cs
            .add(Source::Discovered, Some("db"), Some("postgres"), "", None)
            .unwrap();
        assert_eq!(o, AddOutcome::Added);
        assert!(cs.conflicts.is_empty());
    }

    #[test]
    fn explicit_conflicts_with() {
        let mut cs = Constraints::default();
        let (a, _) = cs
            .add(
                Source::Packer,
                None,
                None,
                "Must use managed Postgres",
                None,
            )
            .unwrap();
        let (_, o) = cs
            .add(
                Source::Discovered,
                None,
                None,
                "No managed databases allowed by org policy",
                Some(&a),
            )
            .unwrap();
        assert!(matches!(o, AddOutcome::Conflict { .. }));
        assert!(cs
            .add(Source::Discovered, None, None, "x", Some("c-9999"))
            .is_err());
    }
}
