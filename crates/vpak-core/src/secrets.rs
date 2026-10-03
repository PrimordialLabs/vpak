//! Pack-time secret scan. A vpak ships a reference realization; it must never
//! ship the credentials that realization used.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    /// Path relative to the pack source.
    pub path: PathBuf,
    /// 1-based line, or 0 for whole-file findings.
    pub line: usize,
    pub kind: &'static str,
    /// Redacted excerpt: the first few characters of the match.
    pub snippet: String,
}

struct Pattern {
    kind: &'static str,
    re: Regex,
}

fn patterns() -> &'static [Pattern] {
    static P: OnceLock<Vec<Pattern>> = OnceLock::new();
    P.get_or_init(|| {
        let mk = |kind, re: &str| Pattern { kind, re: Regex::new(re).expect("valid regex") };
        vec![
            mk("aws-access-key-id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
            mk("pem-private-key", r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----"),
            mk("github-token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b"),
            mk("slack-token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"),
            mk(
                "generic-secret-assignment",
                r#"(?i)\b(?:secret|token|password|passwd|api[_-]?key)\b\s*[:=]\s*['"][^'"\s]{16,}['"]"#,
            ),
        ]
    })
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// Scan one file's contents. `rel` is only used to label findings.
pub fn scan_file(rel: &Path, bytes: &[u8]) -> Vec<Finding> {
    if looks_binary(bytes) {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();

    // Whole-file shape: a GCP service account key.
    if text.contains("\"type\": \"service_account\"") && text.contains("\"private_key\"") {
        out.push(Finding {
            path: rel.to_path_buf(),
            line: 0,
            kind: "gcp-service-account-json",
            snippet: "service_account…".into(),
        });
    }

    for (i, line) in text.lines().enumerate() {
        for p in patterns() {
            if let Some(m) = p.re.find(line) {
                let s: String = m.as_str().chars().take(6).collect();
                out.push(Finding {
                    path: rel.to_path_buf(),
                    line: i + 1,
                    kind: p.kind,
                    snippet: format!("{s}…"),
                });
            }
        }
    }
    out
}

/// Which findings survive the `--allow-secret` waivers. A waiver matches a
/// finding when the finding's path equals the waiver or lives under it.
pub fn unwaived<'a>(findings: &'a [Finding], waivers: &[PathBuf]) -> Vec<&'a Finding> {
    findings
        .iter()
        .filter(|f| {
            !waivers
                .iter()
                .any(|w| f.path == *w || f.path.starts_with(w))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_aws_key() {
        let f = scan_file(
            Path::new("env.sh"),
            b"export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "aws-access-key-id");
        assert_eq!(f[0].line, 1);
        assert!(
            !f[0].snippet.contains("EXAMPLE"),
            "snippet must be redacted"
        );
    }

    #[test]
    fn finds_pem_and_generic() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\napi_key = \"0123456789abcdef0123\"\n";
        let f = scan_file(Path::new("k.txt"), text.as_bytes());
        let kinds: Vec<_> = f.iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&"pem-private-key"));
        assert!(kinds.contains(&"generic-secret-assignment"));
    }

    #[test]
    fn ignores_prose_and_variables() {
        let text = "The password is read from var.db_password at deploy time.\npassword = var.db_password\n";
        assert!(scan_file(Path::new("main.tf"), text.as_bytes()).is_empty());
    }

    #[test]
    fn gcp_service_account_whole_file() {
        let text = r#"{"type": "service_account", "project_id": "x", "private_key": "-----BEGIN PRIVATE KEY-----\nMII"}"#;
        let f = scan_file(Path::new("sa.json"), text.as_bytes());
        assert!(f.iter().any(|x| x.kind == "gcp-service-account-json"));
    }

    #[test]
    fn skips_binary() {
        assert!(scan_file(Path::new("bin"), b"AKIAIOSFODNN7EXAMPLE\0\0").is_empty());
    }

    #[test]
    fn waivers() {
        let f = scan_file(Path::new("fixtures/env.sh"), b"AKIAIOSFODNN7EXAMPLE");
        assert_eq!(unwaived(&f, &[]).len(), 1);
        assert_eq!(unwaived(&f, &[PathBuf::from("fixtures")]).len(), 0);
        assert_eq!(unwaived(&f, &[PathBuf::from("fixtures/env.sh")]).len(), 0);
        assert_eq!(unwaived(&f, &[PathBuf::from("other")]).len(), 1);
    }
}
