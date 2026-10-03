//! Questions to the human: `questions/NNNN-<slug>.toml`. In interactive mode
//! they are answered on the spot; in auto mode they accumulate and the install
//! stops with `needs_human` until someone answers them.

use std::fs;
use std::str::FromStr;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Workdir;
use crate::util;

/// How an answer is applied once given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApplyAs {
    /// Journal note only.
    #[default]
    Note,
    /// Installer constraint (optionally keyed).
    Constraint,
    /// Target field (`--key`) or target prose note.
    Target,
}

impl ApplyAs {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApplyAs::Note => "note",
            ApplyAs::Constraint => "constraint",
            ApplyAs::Target => "target",
        }
    }
}

impl FromStr for ApplyAs {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "note" => Ok(ApplyAs::Note),
            "constraint" => Ok(ApplyAs::Constraint),
            "target" => Ok(ApplyAs::Target),
            _ => bail!("unknown --as '{s}'; expected constraint, target or note"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionStatus {
    Open,
    Answered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub apply_as: ApplyAs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub status: QuestionStatus,
    #[serde(default)]
    pub answer: String,
    pub asked_by: String,
    pub asked: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<DateTime<Utc>>,
}

fn path_for(wd: &Workdir, q: &Question) -> std::path::PathBuf {
    wd.questions_dir()
        .join(format!("{}-{}.toml", q.id, util::slug(&q.question, 32)))
}

pub fn all(wd: &Workdir) -> Result<Vec<Question>> {
    let dir = wd.questions_dir();
    let mut out = Vec::new();
    if dir.is_dir() {
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for f in files {
            out.push(toml::from_str::<Question>(&util::read_to_string(&f)?)?);
        }
    }
    Ok(out)
}

pub fn open(wd: &Workdir) -> Result<Vec<Question>> {
    Ok(all(wd)?
        .into_iter()
        .filter(|q| q.status == QuestionStatus::Open)
        .collect())
}

pub fn get(wd: &Workdir, id: &str) -> Result<Question> {
    all(wd)?
        .into_iter()
        .find(|q| q.id == id)
        .ok_or_else(|| anyhow::anyhow!("no question {id}"))
}

pub fn ask(
    wd: &Workdir,
    question: &str,
    options: &[String],
    apply_as: ApplyAs,
    key: Option<&str>,
    asked_by: &str,
) -> Result<Question> {
    let n = all(wd)?.len() + 1;
    let q = Question {
        id: format!("q-{n:04}"),
        question: question.trim().to_string(),
        options: options.to_vec(),
        apply_as,
        key: key.map(|k| k.to_string()),
        status: QuestionStatus::Open,
        answer: String::new(),
        asked_by: asked_by.to_string(),
        asked: util::now(),
        answered: None,
    };
    save(wd, &q)?;
    Ok(q)
}

pub fn save(wd: &Workdir, q: &Question) -> Result<()> {
    util::write_atomic(&path_for(wd, q), toml::to_string_pretty(q)?)
}

/// Record an answer and apply it per `apply_as`.
pub fn answer(wd: &Workdir, id: &str, answer: &str, actor: &str) -> Result<Question> {
    let mut q = get(wd, id)?;
    if q.status == QuestionStatus::Answered {
        bail!("question {id} is already answered");
    }
    q.answer = answer.trim().to_string();
    q.status = QuestionStatus::Answered;
    q.answered = Some(util::now());
    save(wd, &q)?;
    apply(wd, &q, actor)?;
    Ok(q)
}

fn apply(wd: &Workdir, q: &Question, actor: &str) -> Result<()> {
    use super::{constraints, journal, target};
    let st = wd.state()?;
    match q.apply_as {
        ApplyAs::Constraint => {
            let mut cs = constraints::Constraints::load(wd)?;
            let value = q.key.as_ref().map(|_| q.answer.as_str());
            cs.add(
                constraints::Source::Installer,
                q.key.as_deref(),
                value,
                &format!("{} (answer to {}: {})", q.answer, q.id, q.question),
                None,
            )?;
            cs.save(wd)?;
        }
        ApplyAs::Target => {
            let mut t = target::Target::load(wd)?;
            match &q.key {
                Some(k) => t.set(k, &q.answer)?,
                None => t.note(&format!("{} → {}", q.question, q.answer)),
            }
            t.save(wd)?;
        }
        ApplyAs::Note => {}
    }
    let mut j = journal::Journal::load(wd)?;
    j.append(
        wd,
        st.phase,
        &format!("answer to {}", q.id),
        &format!(
            "**Q:** {}\n\n**A:** {}\n\nApplied as: {}{}",
            q.question,
            q.answer,
            q.apply_as.as_str(),
            q.key
                .as_ref()
                .map(|k| format!(" (`{k}`)"))
                .unwrap_or_default()
        ),
        actor,
    )?;
    journal::regenerate_current(wd)?;
    Ok(())
}
