//! The append-only journal and the compacted `current.md` view.
//!
//! Entries are `bootstrap/journal/NNNN-<phase>-<slug>.md` with a small
//! front-matter block. `current.md` is regenerated from the entries plus the
//! other workdir records; it is what the runner receives on every turn and
//! what a restart reads.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{constraints, plan, questions, survey, target, Phase, Workdir};
use crate::util;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u32,
    pub phase: Phase,
    pub title: String,
    pub ts: DateTime<Utc>,
    pub actor: String,
    #[serde(skip)]
    pub body: String,
    #[serde(skip)]
    pub file: PathBuf,
}

#[derive(Debug, Default)]
pub struct Journal {
    pub entries: Vec<Entry>,
}

impl Journal {
    pub fn load(wd: &Workdir) -> Result<Self> {
        let dir = wd.journal_dir();
        let mut entries = Vec::new();
        if dir.is_dir() {
            let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect();
            files.sort();
            for f in files {
                let text = util::read_to_string(&f)?;
                if let Some(e) = parse_entry(&text, &f) {
                    entries.push(e);
                }
            }
        }
        Ok(Self { entries })
    }

    pub fn append(
        &mut self,
        wd: &Workdir,
        phase: Phase,
        title: &str,
        body: &str,
        actor: &str,
    ) -> Result<&Entry> {
        let seq = self.entries.last().map(|e| e.seq + 1).unwrap_or(1);
        let file = wd.journal_dir().join(format!(
            "{seq:04}-{}-{}.md",
            phase.as_str(),
            util::slug(title, 40)
        ));
        let e = Entry {
            seq,
            phase,
            title: title.to_string(),
            ts: util::now(),
            actor: actor.to_string(),
            body: body.trim_end().to_string(),
            file: file.clone(),
        };
        let text = format!(
            "---\nseq: {}\nphase: {}\ntitle: {}\nts: {}\nactor: {}\n---\n\n{}\n",
            e.seq,
            e.phase,
            e.title.replace('\n', " "),
            e.ts.to_rfc3339(),
            e.actor,
            e.body
        );
        // Append-only: refuse to overwrite an existing entry.
        if file.exists() {
            anyhow::bail!("journal entry {} already exists", file.display());
        }
        fs::create_dir_all(wd.journal_dir())?;
        fs::write(&file, text).with_context(|| format!("write {}", file.display()))?;
        self.entries.push(e);
        Ok(self.entries.last().expect("just pushed"))
    }

    /// Latest entry for each phase, in pipeline order.
    pub fn latest_per_phase(&self) -> Vec<(&Entry, usize)> {
        let mut out = Vec::new();
        for ph in Phase::ALL {
            let in_phase: Vec<&Entry> = self.entries.iter().filter(|e| e.phase == ph).collect();
            if let Some(last) = in_phase.last() {
                out.push((*last, in_phase.len()));
            }
        }
        out
    }
}

fn parse_entry(text: &str, file: &std::path::Path) -> Option<Entry> {
    let text = text.replace("\r\n", "\n");
    let rest = text.strip_prefix("---\n")?;
    let (head, body) = rest.split_once("\n---\n")?;
    let mut seq = None;
    let mut phase = None;
    let mut title = String::new();
    let mut ts = None;
    let mut actor = String::new();
    for line in head.lines() {
        let (k, v) = line.split_once(':')?;
        let v = v.trim();
        match k.trim() {
            "seq" => seq = v.parse().ok(),
            "phase" => phase = v.parse().ok(),
            "title" => title = v.to_string(),
            "ts" => {
                ts = DateTime::parse_from_rfc3339(v)
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }
            "actor" => actor = v.to_string(),
            _ => {}
        }
    }
    Some(Entry {
        seq: seq?,
        phase: phase?,
        title,
        ts: ts?,
        actor,
        body: body.trim().to_string(),
        file: file.to_path_buf(),
    })
}

/// Regenerate `bootstrap/current.md` from the journal and the other records.
pub fn regenerate_current(wd: &Workdir) -> Result<String> {
    let st = wd.state()?;
    let j = Journal::load(wd)?;
    let tgt = target::Target::load(wd)?;
    let cs = constraints::Constraints::load(wd)?;
    let facts = survey::Facts::load(wd)?;
    let plan = plan::Plan::load(wd)?;
    let qs = questions::open(wd)?;

    let mut out = String::new();
    out.push_str(&format!(
        "# Current state: {} {} (install {})\n\n",
        st.vpak_name, st.vpak_version, st.id
    ));
    out.push_str(&format!(
        "- Phase: **{}**  Status: **{}**  Mode: {}  Runner: {}\n",
        st.phase,
        st.status,
        st.mode.as_str(),
        st.runner
    ));
    out.push_str(&format!(
        "- Working directory: `{}`\n- Destination: `{}`\n- Updated: {}\n\n",
        wd.root().display(),
        st.dest.display(),
        st.updated.to_rfc3339()
    ));

    out.push_str("## Target\n\n");
    out.push_str(tgt.prose.trim());
    out.push_str("\n\n");
    let structured = tgt.render_flat();
    if !structured.is_empty() {
        out.push_str("```toml\n");
        out.push_str(&structured);
        out.push_str("```\n\n");
    }

    out.push_str("## Constraints\n\n");
    if cs.constraints.is_empty() {
        out.push_str("(none)\n\n");
    } else {
        for c in cs.active() {
            let key = c
                .key
                .as_deref()
                .map(|k| format!("`{k}` "))
                .unwrap_or_default();
            let val = c
                .value
                .as_deref()
                .map(|v| format!("= {v} "))
                .unwrap_or_default();
            out.push_str(&format!(
                "- [{}] {}{}{}\n",
                c.source.as_str(),
                key,
                val,
                c.text
            ));
        }
        out.push('\n');
    }
    let open: Vec<_> = cs
        .conflicts
        .iter()
        .filter(|c| c.status == constraints::ConflictStatus::Open)
        .collect();
    if !open.is_empty() {
        out.push_str("## Open conflicts\n\n");
        for c in open {
            out.push_str(&format!(
                "- {} : {} vs {} on `{}`\n",
                c.id,
                c.a,
                c.b,
                c.key.as_deref().unwrap_or("-")
            ));
        }
        out.push('\n');
    }

    out.push_str("## Survey facts\n\n");
    if facts.facts.is_empty() {
        out.push_str("(none yet)\n\n");
    } else {
        for f in &facts.facts {
            let v = if f.sensitivity == survey::Sensitivity::High {
                "[redacted: high sensitivity]".to_string()
            } else {
                util::truncate(&f.value, 120)
            };
            out.push_str(&format!(
                "- `{}` = {} (via `{}`)\n",
                f.key,
                v,
                util::truncate(&f.source, 80)
            ));
        }
        out.push('\n');
    }

    out.push_str("## Plan\n\n");
    if plan.mappings.is_empty() && plan.steps.is_empty() {
        out.push_str("(none yet)\n\n");
    } else {
        for m in &plan.mappings {
            out.push_str(&format!(
                "- map `{}` → `{}`: {}\n",
                m.from,
                m.to,
                util::truncate(&m.why, 120)
            ));
        }
        if !plan.mappings.is_empty() {
            out.push('\n');
        }
        for s in &plan.steps {
            let flag = if s.mutating { " (mutating)" } else { "" };
            let res = if s.result.is_empty() {
                String::new()
            } else {
                format!(" — {}", util::truncate(&s.result, 100))
            };
            out.push_str(&format!(
                "{}. [{}]{} {}{}\n",
                s.n,
                s.status.as_str(),
                flag,
                s.step,
                res
            ));
        }
        out.push('\n');
    }

    if !qs.is_empty() {
        out.push_str("## Open questions\n\n");
        for q in &qs {
            out.push_str(&format!("- {}: {}", q.id, q.question));
            if !q.options.is_empty() {
                out.push_str(&format!(" [{}]", q.options.join(", ")));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## Journal (latest entry per phase)\n\n");
    for (e, count) in j.latest_per_phase() {
        let more = if count > 1 {
            format!(
                " ({} earlier entr{} in this phase)",
                count - 1,
                if count == 2 { "y" } else { "ies" }
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "### {} — {} ({}){}\n\n{}\n\n",
            e.phase,
            e.title,
            e.ts.to_rfc3339(),
            more,
            util::truncate(&e.body, 1500)
        ));
    }
    out.push_str(&format!(
        "_{} journal entries total; full trail in `bootstrap/journal/`._\n",
        j.entries.len()
    ));

    util::write_atomic(&wd.current_file(), &out)?;
    Ok(out)
}
