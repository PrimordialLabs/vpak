//! Pack a source tree into a `.vpak` (gzip tar with a fixed layout), unpack
//! one, and verify its digests.
//!
//! Packing is split in two so an inspection pass can run in between:
//! [`stage`] builds the archive tree in a temporary directory, [`finalize`]
//! writes provenance and the archive itself.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use serde::Serialize;

use crate::manifest::{Manifest, MANIFEST_FILE};
use crate::secrets::{self, Finding};
use crate::templates;
use crate::util;

/// Top-level archive directories a structured source tree may provide.
pub const PACKER_DIRS: &[&str] = &["intent", "reference", "constraints", "targets", "bootstrap"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SourceMode {
    /// The source has a `vpak.toml` and the archive directories laid out already.
    Structured,
    /// The source is a plain project tree; all of it becomes `reference/`.
    Raw,
}

#[derive(Debug, Clone, Default)]
pub struct PackOptions {
    pub source: PathBuf,
    /// Literal directive text (already resolved from file or prompt).
    pub directives: Option<String>,
    /// Each entry is a glob to ignore, or a path to an ignore file.
    pub ignore: Vec<String>,
    /// Paths relative to the source whose secret findings are waived.
    pub allow_secret: Vec<PathBuf>,
    pub packer: Option<String>,
    /// Paths to exclude outright (the output archive if it lives in the source).
    pub exclude_abs: Vec<PathBuf>,
}

/// A staged archive tree, ready for inspection and then [`finalize`].
pub struct Staging {
    pub dir: tempfile::TempDir,
    pub manifest: Manifest,
    pub mode: SourceMode,
    pub log: Vec<String>,
    pub files: usize,
}

impl Staging {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
    pub fn log(&mut self, line: impl Into<String>) {
        self.log
            .push(format!("{} {}", util::now().to_rfc3339(), line.into()));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PackReport {
    pub out: PathBuf,
    pub name: String,
    pub version: String,
    pub mode: SourceMode,
    pub files: usize,
    pub bytes: u64,
    pub digest: String,
}

/// Secret findings that were not waived. Returned as an error from [`stage`].
#[derive(Debug, thiserror::Error)]
#[error("secret scan found {} unwaived finding(s); waive with --allow-secret <path> or remove them", .0.len())]
pub struct SecretsFound(pub Vec<Finding>);

/// Walk the source honoring `.gitignore`, `.vpakignore` and `--ignore`, and
/// return source-relative file paths in sorted order.
pub fn walk_source(opts: &PackOptions) -> Result<Vec<PathBuf>> {
    let source = util::canonical(&opts.source)?;
    let mut ov = OverrideBuilder::new(&source);
    ov.add("!.git").context("override")?;
    ov.add("!.vpak").context("override")?;
    for pat in &opts.ignore {
        let p = Path::new(pat);
        if p.is_file() {
            continue; // handled below as an ignore file
        }
        ov.add(&format!("!{pat}"))
            .with_context(|| format!("bad ignore glob {pat}"))?;
    }
    let mut w = WalkBuilder::new(&source);
    w.hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .add_custom_ignore_filename(".vpakignore")
        .overrides(ov.build().context("build overrides")?);
    for pat in &opts.ignore {
        let p = Path::new(pat);
        if p.is_file() {
            if let Some(e) = w.add_ignore(p) {
                bail!("ignore file {pat}: {e}");
            }
        }
    }
    let excluded: Vec<PathBuf> = opts
        .exclude_abs
        .iter()
        .filter_map(|p| util::canonical(p).ok())
        .collect();

    let mut files = Vec::new();
    for entry in w.build() {
        let entry = entry?;
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }
        let abs = entry.path();
        if excluded
            .iter()
            .any(|e| util::strip_verbatim(abs.to_path_buf()) == *e)
        {
            continue;
        }
        let abs_clean = util::strip_verbatim(abs.to_path_buf());
        let rel = abs_clean
            .strip_prefix(&source)
            .unwrap_or(&abs_clean)
            .to_path_buf();
        if rel.as_os_str().is_empty() {
            continue;
        }
        files.push(rel);
    }
    files.sort();
    Ok(files)
}

fn is_structured(source: &Path) -> bool {
    source.join(MANIFEST_FILE).is_file()
}

/// Build the archive tree in a temporary directory. Runs the secret scan and
/// fails with [`SecretsFound`] when an unwaived finding exists.
pub fn stage(opts: &PackOptions) -> Result<Staging> {
    let source = util::canonical(&opts.source)?;
    let mode = if is_structured(&source) {
        SourceMode::Structured
    } else {
        SourceMode::Raw
    };
    let dir = tempfile::Builder::new().prefix("vpak-stage-").tempdir()?;
    let mut st = Staging {
        dir,
        manifest: Manifest::new("placeholder", "0.1.0"),
        mode,
        log: Vec::new(),
        files: 0,
    };
    st.log(format!("pack source {} mode {:?}", source.display(), mode));

    let files = walk_source(opts)?;
    st.log(format!("walked {} files", files.len()));

    // Secret scan on source-relative paths so waivers are stable across modes.
    let mut findings = Vec::new();
    for rel in &files {
        let bytes = fs::read(source.join(rel))?;
        findings.extend(secrets::scan_file(rel, &bytes));
    }
    let unwaived: Vec<Finding> = secrets::unwaived(&findings, &opts.allow_secret)
        .into_iter()
        .cloned()
        .collect();
    if !unwaived.is_empty() {
        return Err(SecretsFound(unwaived).into());
    }
    if !findings.is_empty() {
        st.log(format!("secret scan: {} finding(s) waived", findings.len()));
    }

    // Copy into the staging layout.
    let mut skipped_top = Vec::new();
    for rel in &files {
        let dest_rel: PathBuf = match mode {
            SourceMode::Raw => Path::new("reference").join(rel),
            SourceMode::Structured => {
                let first = rel
                    .components()
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .unwrap_or("");
                if rel == Path::new(MANIFEST_FILE) {
                    continue; // written from the parsed manifest below
                }
                if !PACKER_DIRS.contains(&first) {
                    skipped_top.push(rel.clone());
                    continue;
                }
                rel.clone()
            }
        };
        let to = st.path().join(&dest_rel);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(source.join(rel), &to).with_context(|| format!("copy {}", rel.display()))?;
        st.files += 1;
    }
    if !skipped_top.is_empty() {
        st.log(format!(
            "structured source: skipped {} top-level file(s) outside {:?}: {}",
            skipped_top.len(),
            PACKER_DIRS,
            skipped_top
                .iter()
                .take(10)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Manifest.
    let mut manifest = match mode {
        SourceMode::Structured => Manifest::load(&source.join(MANIFEST_FILE))?,
        SourceMode::Raw => {
            let name = source
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| util::slug(s, 64))
                .unwrap_or_else(|| "vpak".into());
            let mut m = Manifest::new(&name, "0.1.0");
            m.summary = opts
                .directives
                .as_deref()
                .and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
                .map(|l| util::truncate(l.trim(), 200))
                .unwrap_or_default();
            m
        }
    };
    manifest.created = util::now();
    if manifest.packer.is_none() {
        manifest.packer = opts
            .packer
            .clone()
            .or_else(|| std::env::var("VPAK_PACKER").ok())
            .or_else(|| std::env::var("USER").ok());
    }

    // Intent: ensure at least one file.
    let intent_dir = st.path().join("intent");
    fs::create_dir_all(&intent_dir)?;
    let mut intent_files = list_files(&intent_dir)?;
    if intent_files.is_empty() {
        let body = match &opts.directives {
            Some(d) if !d.trim().is_empty() => format!("# Intent\n\n{}\n", d.trim()),
            _ => "# Intent\n\nNo intent was provided at pack time. The inspection pass should draft it from the directives and the reference, or the packer should add `intent/*.md` and re-pack.\n".to_string(),
        };
        fs::write(intent_dir.join("00-overview.md"), body)?;
        intent_files = vec!["00-overview.md".into()];
        st.log("intent: synthesized intent/00-overview.md");
    }
    if manifest.intent.order.is_empty() {
        manifest.intent.order = intent_files.clone();
    }
    if let Some(d) = &opts.directives {
        fs::create_dir_all(st.path().join("provenance"))?;
        fs::write(st.path().join("provenance/directives.md"), d)?;
    }

    // Reference entry hints.
    if manifest.reference.entry.is_empty() {
        let refroot = st.path().join("reference");
        for cand in [
            "Dockerfile",
            "docker-compose.yml",
            "terraform",
            "infra",
            ".github/workflows",
            "cloudbuild.yaml",
            "Makefile",
            "README.md",
        ] {
            if refroot.join(cand).exists() {
                manifest.reference.entry.push(cand.to_string());
            }
        }
    }

    // Bootstrap policy and seed.
    let boot = st.path().join("bootstrap");
    fs::create_dir_all(&boot)?;
    if !boot.join("policy.toml").is_file() {
        fs::write(boot.join("policy.toml"), templates::POLICY_DEFAULT)?;
        st.log("bootstrap: wrote default policy.toml");
    }
    if !boot.join("seed.md").is_file() {
        let seed = compile_seed(st.path(), &manifest)?;
        fs::write(boot.join("seed.md"), seed)?;
        st.log("bootstrap: compiled seed.md");
    }

    fs::write(st.path().join(MANIFEST_FILE), manifest.to_toml()?)?;
    st.manifest = manifest;
    Ok(st)
}

/// Build the seed bootstrap prompt from the template, intent and constraints.
pub fn compile_seed(stage_root: &Path, manifest: &Manifest) -> Result<String> {
    let mut intent = String::new();
    for f in &manifest.intent.order {
        let p = stage_root.join("intent").join(f);
        if p.is_file() {
            intent.push_str(&format!(
                "### {f}\n\n{}\n\n",
                fs::read_to_string(&p)?.trim()
            ));
        }
    }
    if intent.is_empty() {
        intent.push_str("(no intent recorded)\n");
    }
    let mut constraints = String::new();
    let cdir = stage_root.join("constraints");
    if cdir.is_dir() {
        for f in list_files(&cdir)? {
            if f.ends_with(".md") {
                constraints.push_str(&format!(
                    "### {f}\n\n{}\n\n",
                    fs::read_to_string(cdir.join(&f))?.trim()
                ));
            }
        }
    }
    if constraints.is_empty() {
        constraints.push_str("(none recorded by the packer)\n");
    }
    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("name", manifest.name.clone());
    vars.insert("version", manifest.version.clone());
    vars.insert(
        "summary",
        if manifest.summary.is_empty() {
            "(none)".into()
        } else {
            manifest.summary.clone()
        },
    );
    vars.insert("intent", intent.trim_end().to_string());
    vars.insert("constraints", constraints.trim_end().to_string());
    vars.insert(
        "reference_kind",
        manifest
            .reference
            .kind
            .clone()
            .unwrap_or_else(|| "unspecified".into()),
    );
    Ok(util::render(templates::BOOTSTRAP_SEED, &vars))
}

fn list_files(dir: &Path) -> Result<Vec<String>> {
    let mut v = Vec::new();
    if dir.is_dir() {
        for e in fs::read_dir(dir)? {
            let e = e?;
            if e.file_type()?.is_file() {
                if let Some(n) = e.file_name().to_str() {
                    v.push(n.to_string());
                }
            }
        }
    }
    v.sort();
    Ok(v)
}

/// Every file under `root`, as sorted relative paths.
pub fn tree_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn rec(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for e in fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            let ft = e.file_type()?;
            if ft.is_dir() {
                rec(root, &p, out)?;
            } else if ft.is_file() {
                out.push(p.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }
    rec(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

/// Write provenance (digests, pack log) and the archive.
pub fn finalize(mut st: Staging, out: &Path, inspected: bool) -> Result<PackReport> {
    let prov = st.path().join("provenance");
    fs::create_dir_all(&prov)?;
    if !prov.join("inspection.md").is_file() {
        let note = if inspected {
            "# Inspection\n\nThe inspection pass ran but wrote no report.\n"
        } else {
            "# Inspection\n\nSkipped (`--no-inspect`). REFERENCE.md and gap analysis were not generated.\n"
        };
        fs::write(prov.join("inspection.md"), note)?;
    }
    st.log(format!(
        "inspection pass: {}",
        if inspected { "ran" } else { "skipped" }
    ));

    // Digests over everything except the two provenance files written last.
    let mut lines = Vec::new();
    for rel in tree_files(st.path())? {
        if rel == Path::new("provenance/manifest.sha256") || rel == Path::new("provenance/pack.log")
        {
            continue;
        }
        let hex = util::sha256_file(&st.path().join(&rel))?;
        lines.push(format!("{hex}  {}", util::slash_path(&rel)));
    }
    fs::write(prov.join("manifest.sha256"), lines.join("\n") + "\n")?;
    st.log(format!("digested {} files", lines.len()));
    st.log(format!("archive {}", out.display()));
    fs::write(prov.join("pack.log"), st.log.join("\n") + "\n")?;

    if let Some(p) = out.parent() {
        if !p.as_os_str().is_empty() {
            fs::create_dir_all(p)?;
        }
    }
    let file = fs::File::create(out).with_context(|| format!("create {}", out.display()))?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    let mut bytes = 0u64;
    let mut count = 0usize;
    for rel in tree_files(st.path())? {
        let abs = st.path().join(&rel);
        bytes += fs::metadata(&abs)?.len();
        tar.append_path_with_name(&abs, util::slash_path(&rel))
            .with_context(|| format!("tar {}", rel.display()))?;
        count += 1;
    }
    let enc = tar.into_inner()?;
    enc.finish()?.flush()?;

    Ok(PackReport {
        out: out.to_path_buf(),
        name: st.manifest.name.clone(),
        version: st.manifest.version.clone(),
        mode: st.mode,
        files: count,
        bytes,
        digest: format!("sha256:{}", util::sha256_file(out)?),
    })
}

/// One entry of an archive listing.
#[derive(Debug, Clone, Serialize)]
pub struct EntryInfo {
    pub path: PathBuf,
    pub size: u64,
}

/// Read the manifest and the entry list without unpacking.
pub fn list(archive: &Path) -> Result<(Manifest, Vec<EntryInfo>)> {
    let f = fs::File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let mut ar = tar::Archive::new(GzDecoder::new(f));
    let mut entries = Vec::new();
    let mut manifest: Option<Manifest> = None;
    for e in ar.entries()? {
        let mut e = e?;
        let path = e.path()?.to_path_buf();
        check_safe(&path)?;
        let size = e.header().size()?;
        if path == Path::new(MANIFEST_FILE) {
            let mut s = String::new();
            e.read_to_string(&mut s)?;
            manifest = Some(Manifest::parse(&s)?);
        }
        if e.header().entry_type().is_file() {
            entries.push(EntryInfo { path, size });
        }
    }
    let manifest =
        manifest.ok_or_else(|| anyhow!("{} has no {}", archive.display(), MANIFEST_FILE))?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((manifest, entries))
}

/// Unpack into `dest` (created if needed). Refuses unsafe paths.
pub fn unpack(archive: &Path, dest: &Path) -> Result<Manifest> {
    fs::create_dir_all(dest)?;
    let f = fs::File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let mut ar = tar::Archive::new(GzDecoder::new(f));
    ar.set_overwrite(true);
    for e in ar.entries()? {
        let mut e = e?;
        let path = e.path()?.to_path_buf();
        check_safe(&path)?;
        if !e.unpack_in(dest)? {
            bail!("refused to unpack {}", path.display());
        }
    }
    Manifest::load(&dest.join(MANIFEST_FILE))
}

fn check_safe(p: &Path) -> Result<()> {
    if p.is_absolute() {
        bail!("archive contains absolute path {}", p.display());
    }
    for c in p.components() {
        match c {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("archive contains unsafe path {}", p.display())
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Mismatch {
    pub path: PathBuf,
    pub expected: String,
    pub actual: Option<String>,
}

/// Compare an unpacked tree against its `provenance/manifest.sha256`.
pub fn verify(unpacked: &Path) -> Result<Vec<Mismatch>> {
    let text = util::read_to_string(&unpacked.join("provenance/manifest.sha256"))?;
    let mut expected = BTreeMap::new();
    for line in text.lines() {
        if let Some((hex, path)) = line.split_once("  ") {
            expected.insert(PathBuf::from(path), hex.to_string());
        }
    }
    let mut out = Vec::new();
    for (path, hex) in &expected {
        let abs = unpacked.join(path);
        let actual = if abs.is_file() {
            Some(util::sha256_file(&abs)?)
        } else {
            None
        };
        if actual.as_deref() != Some(hex.as_str()) {
            out.push(Mismatch {
                path: path.clone(),
                expected: hex.clone(),
                actual,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_dir() -> PathBuf {
        util::canonical(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello-service"))
            .unwrap()
    }

    #[test]
    fn pack_unpack_round_trip_example() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("hello.vpak");
        let st = stage(&PackOptions {
            source: example_dir(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(st.mode, SourceMode::Structured);
        assert_eq!(st.manifest.name, "hello-service");
        let report = finalize(st, &out, false).unwrap();
        assert!(report.files > 5);
        assert!(report.digest.starts_with("sha256:"));

        let (m, entries) = list(&out).unwrap();
        assert_eq!(m.name, "hello-service");
        let paths: Vec<String> = entries
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        assert!(paths.contains(&"vpak.toml".to_string()));
        assert!(paths.contains(&"bootstrap/seed.md".to_string()));
        assert!(paths.contains(&"provenance/manifest.sha256".to_string()));
        assert!(paths.iter().any(|p| p.starts_with("reference/terraform/")));

        let dest = tmp.path().join("unpacked");
        let m2 = unpack(&out, &dest).unwrap();
        assert_eq!(m2.name, m.name);
        assert!(verify(&dest).unwrap().is_empty());

        // Tamper and verify again.
        fs::write(dest.join("intent/00-overview.md"), "changed").unwrap();
        let mm = verify(&dest).unwrap();
        assert_eq!(mm.len(), 1);
        assert_eq!(mm[0].path, PathBuf::from("intent/00-overview.md"));
    }

    #[test]
    fn raw_mode_and_ignore_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("proj");
        fs::create_dir_all(src.join("node_modules/x")).unwrap();
        fs::create_dir_all(src.join("build")).unwrap();
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join("main.go"), "package main").unwrap();
        fs::write(src.join("node_modules/x/i.js"), "x").unwrap();
        fs::write(src.join("build/out.bin"), "x").unwrap();
        fs::write(src.join(".git/HEAD"), "ref").unwrap();
        fs::write(src.join("notes.tmp"), "x").unwrap();
        fs::write(src.join(".gitignore"), "node_modules/\n").unwrap();
        fs::write(src.join(".vpakignore"), "build/\n").unwrap();

        let opts = PackOptions {
            source: src.clone(),
            directives: Some("Build a thing.\nMore detail.".into()),
            ignore: vec!["*.tmp".into()],
            ..Default::default()
        };
        let files = walk_source(&opts).unwrap();
        let names: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(names, vec![".gitignore", ".vpakignore", "main.go"]);

        let st = stage(&opts).unwrap();
        assert_eq!(st.mode, SourceMode::Raw);
        assert_eq!(st.manifest.name, "proj");
        assert_eq!(st.manifest.summary, "Build a thing.");
        assert!(st.path().join("reference/main.go").is_file());
        assert!(st.path().join("intent/00-overview.md").is_file());
        assert!(st.path().join("bootstrap/policy.toml").is_file());
        let seed = fs::read_to_string(st.path().join("bootstrap/seed.md")).unwrap();
        assert!(seed.contains("Build a thing."));
        assert!(seed.contains("vpak bootstrap: proj"));
    }

    #[test]
    fn secret_scan_blocks_and_waives() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("proj");
        fs::create_dir_all(src.join("cfg")).unwrap();
        fs::write(src.join("main.go"), "package main").unwrap();
        fs::write(
            src.join("cfg/env.sh"),
            "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
        )
        .unwrap();

        let err = stage(&PackOptions {
            source: src.clone(),
            ..Default::default()
        })
        .err()
        .expect("must fail");
        let found = err.downcast_ref::<SecretsFound>().expect("SecretsFound");
        assert_eq!(found.0.len(), 1);
        assert_eq!(found.0[0].path, PathBuf::from("cfg/env.sh"));

        let st = stage(&PackOptions {
            source: src,
            allow_secret: vec![PathBuf::from("cfg/env.sh")],
            ..Default::default()
        })
        .unwrap();
        assert!(st.log.iter().any(|l| l.contains("waived")));
    }

    #[test]
    fn unsafe_paths_rejected() {
        assert!(check_safe(Path::new("../x")).is_err());
        assert!(check_safe(Path::new("/etc/passwd")).is_err());
        assert!(check_safe(Path::new("a/b/c")).is_ok());
    }
}
