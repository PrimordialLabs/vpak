//! `audit.jsonl`: one line per primitive call or runner event.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Workdir;
use crate::util;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLine {
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub kind: String,
    pub args: serde_json::Value,
    pub outcome: String,
}

pub fn record(
    wd: &Workdir,
    actor: &str,
    kind: &str,
    args: serde_json::Value,
    outcome: &str,
) -> Result<()> {
    let line = AuditLine {
        ts: util::now(),
        actor: actor.to_string(),
        kind: kind.to_string(),
        args,
        outcome: outcome.to_string(),
    };
    util::append_line(&wd.audit_file(), &serde_json::to_string(&line)?)
}

pub fn read(wd: &Workdir) -> Result<Vec<AuditLine>> {
    let p = wd.audit_file();
    if !p.is_file() {
        return Ok(Vec::new());
    }
    let text = util::read_to_string(&p)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(l) = serde_json::from_str::<AuditLine>(line) {
            out.push(l);
        }
    }
    Ok(out)
}
