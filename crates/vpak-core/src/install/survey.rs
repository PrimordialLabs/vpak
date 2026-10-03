//! Discovered facts: `survey/facts.toml`. Read-only findings about the host
//! and the target environment, each sourced to the command that produced it.

use std::str::FromStr;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Phase, Workdir};
use crate::util;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    #[default]
    Low,
    Med,
    High,
}

impl FromStr for Sensitivity {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "low" | "l" => Ok(Sensitivity::Low),
            "med" | "medium" | "m" => Ok(Sensitivity::Med),
            "high" | "h" => Ok(Sensitivity::High),
            _ => bail!("unknown sensitivity '{s}'; expected low, med or high"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub key: String,
    pub value: String,
    /// The command or observation that produced the value.
    pub source: String,
    #[serde(default)]
    pub sensitivity: Sensitivity,
    pub phase: Phase,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Facts {
    #[serde(default, rename = "fact")]
    pub facts: Vec<Fact>,
}

impl Facts {
    pub fn load(wd: &Workdir) -> Result<Self> {
        let p = wd.survey_dir().join("facts.toml");
        if !p.is_file() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(&util::read_to_string(&p)?)?)
    }

    pub fn save(&self, wd: &Workdir) -> Result<()> {
        util::write_atomic(
            &wd.survey_dir().join("facts.toml"),
            toml::to_string_pretty(self)?,
        )
    }

    /// Record a fact. A repeated key replaces the earlier value; the earlier
    /// value stays in the audit log.
    pub fn record(
        &mut self,
        key: &str,
        value: &str,
        source: &str,
        sensitivity: Sensitivity,
        phase: Phase,
    ) -> &Fact {
        let f = Fact {
            key: key.trim().to_string(),
            value: value.to_string(),
            source: source.to_string(),
            sensitivity,
            phase,
            ts: util::now(),
        };
        if let Some(pos) = self.facts.iter().position(|x| x.key == f.key) {
            self.facts[pos] = f;
            &self.facts[pos]
        } else {
            self.facts.push(f);
            self.facts.last().expect("pushed")
        }
    }

    pub fn get(&self, key: &str) -> Option<&Fact> {
        self.facts.iter().find(|f| f.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_replaces_same_key() {
        let mut f = Facts::default();
        f.record(
            "host.os",
            "darwin",
            "uname -s",
            Sensitivity::Low,
            Phase::Locate,
        );
        f.record(
            "host.os",
            "linux",
            "uname -s",
            Sensitivity::Low,
            Phase::Locate,
        );
        f.record(
            "aws.account",
            "123",
            "aws sts get-caller-identity",
            Sensitivity::High,
            Phase::Survey,
        );
        assert_eq!(f.facts.len(), 2);
        assert_eq!(f.get("host.os").unwrap().value, "linux");
        let text = toml::to_string_pretty(&f).unwrap();
        let back: Facts = toml::from_str(&text).unwrap();
        assert_eq!(back.facts, f.facts);
    }
}
