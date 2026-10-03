//! Small shared helpers: ids, timestamps, slugs, digests, template substitution.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// A new lowercase ULID with the given prefix, e.g. `vp-01j9k3...`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", ulid::Ulid::new().to_string().to_lowercase())
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Lowercase, ASCII, hyphen-separated, at most `max` characters.
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Replace `{{key}}` placeholders. Unknown keys are left in place so a
/// missing value is visible rather than silently blank.
pub fn render(template: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

/// Create the parent directory of `path` if needed, then write.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    fs::write(&tmp, contents).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn append_line(path: &Path, line: &str) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())?;
    if !line.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    Ok(())
}

/// Read a file to a string with a path in the error.
pub fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

/// Truncate to roughly `max` characters on a char boundary, appending an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// The first sentence of `text` with whitespace collapsed, truncated to `max`.
pub fn first_sentence(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = collapsed
        .find(". ")
        .map(|i| i + 1)
        .unwrap_or(collapsed.len());
    truncate(collapsed[..end].trim(), max)
}

/// Resolve a `<file|prompt|->` argument: `-` reads stdin, an existing file is
/// read, anything else is taken as literal prose.
pub fn resolve_text_arg(arg: &str) -> Result<String> {
    if arg == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    let p = Path::new(arg);
    if p.is_file() {
        return read_to_string(p);
    }
    Ok(arg.to_string())
}

/// Canonicalize a path and strip the Windows verbatim prefix (`\\?\`) so the
/// result is safe to write into files and show to people.
pub fn canonical(path: &Path) -> Result<std::path::PathBuf> {
    let c = fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?;
    Ok(strip_verbatim(c))
}

/// Strip `\\?\` and `\\?\UNC\` prefixes on Windows; identity elsewhere.
pub fn strip_verbatim(p: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return std::path::PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(rest);
        }
    }
    p
}

/// A relative path as a `/`-separated string, for archive entries and logs.
pub fn slash_path(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            std::path::Component::CurDir => None,
            other => Some(other.as_os_str().to_string_lossy().into_owned()),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The current user's name: `USER`, then `USERNAME`, then the OS.
pub fn identity() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(whoami::username)
}

/// Split a command template into argv without invoking a shell. POSIX
/// shell-style on Unix; on Windows, whitespace-separated with double quotes
/// and no backslash escapes, so `C:\path\x.exe` survives.
pub fn split_argv(template: &str) -> Result<Vec<String>> {
    let argv = if cfg!(windows) {
        split_windows(template)
    } else {
        shlex::split(template)
    };
    let argv =
        argv.ok_or_else(|| anyhow::anyhow!("unbalanced quotes in command template: {template}"))?;
    if argv.is_empty() {
        anyhow::bail!("empty command template");
    }
    Ok(argv)
}

fn split_windows(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut had = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                had = true;
            }
            c if c.is_whitespace() && !in_q => {
                if had {
                    out.push(std::mem::take(&mut cur));
                    had = false;
                }
            }
            c => {
                cur.push(c);
                had = true;
            }
        }
    }
    if in_q {
        return None;
    }
    if had {
        out.push(cur);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(slug("Hello, World! 123", 40), "hello-world-123");
        assert_eq!(slug("   ", 40), "untitled");
        assert_eq!(slug("a-very-long-title-that-goes-on", 10), "a-very-lon");
    }

    #[test]
    fn render_leaves_unknown() {
        let mut v = HashMap::new();
        v.insert("name", "x".to_string());
        assert_eq!(render("{{name}} {{other}}", &v), "x {{other}}");
    }

    #[test]
    fn first_sentence_collapses() {
        assert_eq!(
            first_sentence("Install this\non my laptop. No cloud.", 160),
            "Install this on my laptop."
        );
        assert_eq!(
            first_sentence("No period here\nat all", 160),
            "No period here at all"
        );
    }

    #[test]
    fn windows_split_keeps_backslashes() {
        assert_eq!(
            split_windows(r#""C:\tools\vpak.exe" __fixture agent"#).unwrap(),
            vec![r"C:\tools\vpak.exe", "__fixture", "agent"]
        );
        assert!(split_windows("\"open").is_none());
    }

    #[test]
    fn id_shape() {
        let id = new_id("vp");
        assert!(id.starts_with("vp-"));
        assert_eq!(id.len(), 3 + 26);
        assert_eq!(id, id.to_lowercase());
    }
}
