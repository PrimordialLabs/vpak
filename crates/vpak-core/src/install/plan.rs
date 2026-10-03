//! The plan: component mappings (`plan/mapping.toml`, rendered to
//! `plan/mapping.md`) and ordered steps (`plan/steps/NNNN-<slug>.toml`).

use std::fs;
use std::str::FromStr;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Workdir;
use crate::util;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    pub from: String,
    pub to: String,
    pub why: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MappingFile {
    #[serde(default, rename = "mapping")]
    mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Done,
    Failed,
    Skipped,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Done => "done",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
    }
}

impl FromStr for StepStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "pending" => Ok(StepStatus::Pending),
            "done" => Ok(StepStatus::Done),
            "failed" => Ok(StepStatus::Failed),
            "skipped" => Ok(StepStatus::Skipped),
            _ => bail!("unknown step status '{s}'; expected pending, done, failed or skipped"),
        }
    }
}

/// What kind of work a step is; used as the vflt stage on fleet handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Survey,
    Code,
    Review,
    Test,
    Deploy,
}

impl StepKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepKind::Survey => "survey",
            StepKind::Code => "code",
            StepKind::Review => "review",
            StepKind::Test => "test",
            StepKind::Deploy => "deploy",
        }
    }
}

impl FromStr for StepKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "survey" => Ok(StepKind::Survey),
            "code" => Ok(StepKind::Code),
            "review" => Ok(StepKind::Review),
            "test" => Ok(StepKind::Test),
            "deploy" => Ok(StepKind::Deploy),
            _ => bail!("unknown step kind '{s}'; expected survey, code, review, test or deploy"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub n: u32,
    pub slug: String,
    pub step: String,
    #[serde(default)]
    pub mutating: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    pub kind: StepKind,
    pub status: StepStatus,
    #[serde(default)]
    pub result: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub mappings: Vec<Mapping>,
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn load(wd: &Workdir) -> Result<Self> {
        let mf = wd.plan_dir().join("mapping.toml");
        let mappings = if mf.is_file() {
            toml::from_str::<MappingFile>(&util::read_to_string(&mf)?)?.mappings
        } else {
            Vec::new()
        };
        let sdir = wd.plan_dir().join("steps");
        let mut steps = Vec::new();
        if sdir.is_dir() {
            let mut files: Vec<_> = fs::read_dir(&sdir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect();
            files.sort();
            for f in files {
                steps.push(toml::from_str::<Step>(&util::read_to_string(&f)?)?);
            }
        }
        steps.sort_by_key(|s| s.n);
        Ok(Self { mappings, steps })
    }

    pub fn map(&mut self, wd: &Workdir, from: &str, to: &str, why: &str) -> Result<()> {
        let m = Mapping {
            from: from.trim().to_string(),
            to: to.trim().to_string(),
            why: why.trim().to_string(),
            ts: util::now(),
        };
        if let Some(pos) = self.mappings.iter().position(|x| x.from == m.from) {
            self.mappings[pos] = m;
        } else {
            self.mappings.push(m);
        }
        util::write_atomic(
            &wd.plan_dir().join("mapping.toml"),
            toml::to_string_pretty(&MappingFile {
                mappings: self.mappings.clone(),
            })?,
        )?;
        let mut md = String::from("# Reference → target mapping\n\n| Reference component | Target component | Why |\n|---|---|---|\n");
        for m in &self.mappings {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                m.from,
                m.to,
                m.why.replace('|', "\\|")
            ));
        }
        util::write_atomic(&wd.plan_dir().join("mapping.md"), md)
    }

    pub fn add_step(
        &mut self,
        wd: &Workdir,
        step: &str,
        mutating: bool,
        cmd: Option<&str>,
        kind: Option<StepKind>,
    ) -> Result<&Step> {
        let n = self.steps.last().map(|s| s.n + 1).unwrap_or(1);
        let kind = kind.unwrap_or(if mutating {
            StepKind::Deploy
        } else {
            StepKind::Code
        });
        let now = util::now();
        let s = Step {
            n,
            slug: util::slug(step, 40),
            step: step.trim().to_string(),
            mutating,
            cmd: cmd.map(|c| c.to_string()).filter(|c| !c.trim().is_empty()),
            kind,
            status: StepStatus::Pending,
            result: String::new(),
            created: now,
            updated: now,
        };
        self.write_step(wd, &s)?;
        self.steps.push(s);
        Ok(self.steps.last().expect("pushed"))
    }

    pub fn mark(
        &mut self,
        wd: &Workdir,
        n: u32,
        status: StepStatus,
        result: &str,
    ) -> Result<&Step> {
        let pos = self
            .steps
            .iter()
            .position(|s| s.n == n)
            .ok_or_else(|| anyhow::anyhow!("no plan step {n}"))?;
        let s = &mut self.steps[pos];
        s.status = status;
        s.result = result.trim().to_string();
        s.updated = util::now();
        let s = s.clone();
        self.write_step(wd, &s)?;
        Ok(&self.steps[pos])
    }

    fn write_step(&self, wd: &Workdir, s: &Step) -> Result<()> {
        let path = wd
            .plan_dir()
            .join("steps")
            .join(format!("{:04}-{}.toml", s.n, s.slug));
        util::write_atomic(&path, toml::to_string_pretty(s)?)
    }

    pub fn pending(&self) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|s| s.status == StepStatus::Pending)
            .collect()
    }
}
