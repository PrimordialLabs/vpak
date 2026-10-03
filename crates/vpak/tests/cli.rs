//! End-to-end tests against the built `vpak` binary. Fixtures (a fake runner
//! agent and a fake vflt) are hidden subcommands of the same binary, so
//! nothing here depends on a shell or on the host platform.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn vpak_bin() -> &'static str {
    env!("CARGO_BIN_EXE_vpak")
}

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/hello-service")
        .canonicalize()
        .unwrap()
}

/// Quoted so the shlex-style splitter (and the Windows splitter) keep the path whole.
fn fixture_cmd(sub: &str) -> String {
    format!("\"{}\" __fixture {sub}", vpak_bin())
}

fn run(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(vpak_bin());
    c.args(args).current_dir(cwd);
    c.env_remove("VPAK_WORKDIR");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("spawn vpak")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn pack_example(cwd: &Path) -> PathBuf {
    let o = run(
        cwd,
        &["add", example_dir().to_str().unwrap(), "--no-inspect"],
        &[],
    );
    assert!(o.status.success(), "add failed: {}", stderr(&o));
    let p = cwd.join("hello-service.vpak");
    assert!(p.is_file());
    p
}

fn status_json(cwd: &Path, wd: &Path) -> serde_json::Value {
    let o = run(
        cwd,
        &["--workdir", wd.to_str().unwrap(), "--json", "status"],
        &[],
    );
    assert!(o.status.success(), "status failed: {}", stderr(&o));
    serde_json::from_str(&stdout(&o)).expect("status json")
}

#[test]
fn add_inspect_and_print_bootstrap() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());

    let o = run(
        tmp.path(),
        &["--json", "inspect", archive.to_str().unwrap()],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["manifest"]["name"], "hello-service");
    assert!(v["entries"].as_array().unwrap().len() > 10);
    assert_eq!(v["policy"]["permission_mode"], "auto");

    let wd = tmp.path().join("wd");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--target",
            "Install to my laptop with Docker. Nothing else.",
            "--constraints",
            "No cloud spend.",
            "--print-bootstrap",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let text = stdout(&o);
    assert!(text.contains("# vpak bootstrap: hello-service 0.1.0"));
    assert!(text.contains("# vpak primitives"));
    assert!(text.contains("Install to my laptop with Docker."));
    assert!(text.contains("[installer] No cloud spend."));
    assert!(wd.join("install.toml").is_file());
    assert!(wd.join("bootstrap/compiled.md").is_file());
    assert!(wd.join("bootstrap/journal").read_dir().unwrap().count() >= 1);
    assert!(wd.join("audit.jsonl").is_file());

    // Second print-bootstrap against the same workdir resumes rather than re-initializing.
    let o2 = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--print-bootstrap",
        ],
        &[],
    );
    assert!(o2.status.success(), "{}", stderr(&o2));
    assert!(stderr(&o2).contains("resuming install"));
}

#[test]
fn secret_scan_blocks_pack_unless_waived() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("proj");
    std::fs::create_dir_all(src.join("cfg")).unwrap();
    std::fs::write(src.join("main.go"), "package main\n").unwrap();
    std::fs::write(
        src.join("cfg/env.sh"),
        "export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n",
    )
    .unwrap();

    let o = run(
        tmp.path(),
        &[
            "add",
            src.to_str().unwrap(),
            "--no-inspect",
            "--directives",
            "A thing.",
        ],
        &[],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("aws-access-key-id"));
    assert!(stderr(&o).contains("env.sh"));

    let o = run(
        tmp.path(),
        &[
            "add",
            src.to_str().unwrap(),
            "--no-inspect",
            "--directives",
            "A thing.",
            "--allow-secret",
            "cfg/env.sh",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(tmp.path().join("proj.vpak").is_file());
}

#[test]
fn shell_runner_drives_every_phase_to_done() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let agent = fixture_cmd("agent");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--dest",
            tmp.path().join("dest").to_str().unwrap(),
            "--max-phases",
            "12",
        ],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(
        o.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&o),
        stderr(&o)
    );

    let st = status_json(tmp.path(), &wd);
    assert_eq!(st["phase"], "done");
    assert_eq!(st["status"], "done");
    assert_eq!(st["steps_total"], 2);
    assert_eq!(st["mappings"], 1);
    assert!(st["facts"].as_u64().unwrap() >= 7, "{st}");
    assert!(st["journal_entries"].as_u64().unwrap() >= 14, "{st}");

    let o = run(
        tmp.path(),
        &["--workdir", wd.to_str().unwrap(), "survey", "list"],
        &[],
    );
    let s = stdout(&o);
    assert!(s.contains("fixture.locate = ran"));
    assert!(s.contains("fixture.verify = ran"));

    let o = run(
        tmp.path(),
        &["--workdir", wd.to_str().unwrap(), "constraint", "list"],
        &[],
    );
    assert!(stdout(&o).contains("[discovered] fixture.plan=yes"));

    // Audit has runner start/end lines and primitive calls with the runner actor.
    let audit = std::fs::read_to_string(wd.join("audit.jsonl")).unwrap();
    assert!(audit.contains("\"kind\":\"runner.start\""));
    assert!(audit.contains("\"kind\":\"runner.end\""));
    assert!(audit.contains("\"actor\":\"runner:shell\""));
    assert!(audit.contains("\"kind\":\"phase.set\""));

    // The journal is append-only: every entry file is still there.
    let n = wd.join("bootstrap/journal").read_dir().unwrap().count();
    assert_eq!(n as u64, st["journal_entries"].as_u64().unwrap());
}

#[test]
fn resume_continues_from_recorded_phase() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let agent = fixture_cmd("agent");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--max-phases",
            "1",
        ],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(status_json(tmp.path(), &wd)["phase"], "survey");

    let o = run(
        tmp.path(),
        &["resume", wd.to_str().unwrap(), "--max-phases", "1"],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(status_json(tmp.path(), &wd)["phase"], "constrain");

    // `install` with a discovered .vpak/ workdir resumes too.
    let cwd = tmp.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let o = run(
        &cwd,
        &[
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--max-phases",
            "1",
        ],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(
        &cwd,
        &["install", archive.to_str().unwrap(), "--max-phases", "1"],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("resuming install"));
}

#[test]
fn auto_mode_question_stops_with_needs_human_and_resumes_after_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let agent = fixture_cmd("agent");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--max-phases",
            "5",
        ],
        &[("VPAK_SHELL_RUNNER_CMD", &agent), ("VPAK_FIXTURE_ASK", "1")],
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "stdout: {}\nstderr: {}",
        stdout(&o),
        stderr(&o)
    );
    let st = status_json(tmp.path(), &wd);
    assert_eq!(st["status"], "needs_human");
    assert_eq!(st["phase"], "locate");
    assert_eq!(st["open_questions"].as_array().unwrap().len(), 1);
    assert_eq!(st["open_questions"][0]["id"], "q-0001");
    assert_eq!(st["open_questions"][0]["options"][1], "eu-west-1");

    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "question",
            "answer",
            "q-0001",
            "--text",
            "eu-west-1",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(
        tmp.path(),
        &["--workdir", wd.to_str().unwrap(), "target", "show"],
        &[],
    );
    assert!(stdout(&o).contains("region = \"eu-west-1\""));

    let o = run(
        tmp.path(),
        &["resume", wd.to_str().unwrap(), "--max-phases", "12"],
        &[("VPAK_SHELL_RUNNER_CMD", &agent)],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(status_json(tmp.path(), &wd)["phase"], "done");
}

#[test]
fn stalled_runner_raises_a_question() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let agent = fixture_cmd("agent");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--max-phases",
            "6",
        ],
        &[
            ("VPAK_SHELL_RUNNER_CMD", &agent),
            ("VPAK_FIXTURE_STALL", "1"),
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "stdout: {}\nstderr: {}",
        stdout(&o),
        stderr(&o)
    );
    let st = status_json(tmp.path(), &wd);
    assert_eq!(st["status"], "needs_human");
    assert!(st["open_questions"][0]["question"]
        .as_str()
        .unwrap()
        .contains("without advancing"));
}

#[test]
fn constraint_precedence_and_conflicts_via_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--print-bootstrap",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let w = wd.to_str().unwrap();

    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "constraint",
            "add",
            "--source",
            "installer",
            "--key",
            "region",
            "--value",
            "us-east-1",
            "--text",
            "Installer region",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "constraint",
            "add",
            "--source",
            "discovered",
            "--key",
            "region",
            "--value",
            "eu-west-1",
            "--text",
            "Only enabled region",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("CONFLICT"));

    let o = run(
        tmp.path(),
        &["--workdir", w, "--json", "constraint", "conflicts"],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["status"], "open");
    let (a, cid) = (
        v[0]["a"].as_str().unwrap().to_string(),
        v[0]["id"].as_str().unwrap().to_string(),
    );

    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "constraint",
            "resolve",
            &cid,
            "--keep",
            &a,
            "--reason",
            "installer wins",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(
        tmp.path(),
        &["--workdir", w, "--json", "constraint", "conflicts"],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v[0]["status"], "resolved");

    // Packer constraints should not be addable from the CLI.
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "constraint",
            "add",
            "--source",
            "packer",
            "--text",
            "x",
        ],
        &[],
    );
    assert!(!o.status.success());

    // Phase loop-back is allowed and journaled.
    let o = run(
        tmp.path(),
        &["--workdir", w, "phase", "set", "plan", "--reason", "jump"],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(
        tmp.path(),
        &["--workdir", w, "phase", "set", "survey", "--reason", "back"],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(tmp.path(), &["--workdir", w, "journal", "list"], &[]);
    let s = stdout(&o);
    assert!(s.contains("enter plan"));
    assert!(s.contains("enter survey"));
}

#[test]
fn fleet_bootstrap_with_fake_vflt() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--dest",
            tmp.path().join("dest").to_str().unwrap(),
            "--print-bootstrap",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let w = wd.to_str().unwrap();
    run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "plan",
            "add",
            "--step",
            "write terraform",
            "--kind",
            "code",
        ],
        &[],
    );
    run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "plan",
            "add",
            "--step",
            "apply terraform",
            "--mutating",
        ],
        &[],
    );
    run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "plan",
            "add",
            "--step",
            "already done",
            "--kind",
            "test",
        ],
        &[],
    );
    run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "plan",
            "mark",
            "3",
            "--status",
            "done",
            "--result",
            "n/a",
        ],
        &[],
    );

    let log = tmp.path().join("vflt.log");
    let coll = tmp.path().join("coll");
    let fake = fixture_cmd("vflt");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "--json",
            "fleet",
            "bootstrap",
            "--collective",
            coll.to_str().unwrap(),
        ],
        &[
            ("VFLT_BIN", &fake),
            ("VPAK_FIXTURE_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(
        o.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&o),
        stderr(&o)
    );
    let rec: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        rec["items"].as_array().unwrap().len(),
        2,
        "only pending steps become items"
    );
    assert_eq!(rec["items"][0]["id"], "vf-fake-0001");
    assert_eq!(rec["items"][0]["stage"], "code");
    assert_eq!(rec["items"][1]["stage"], "deploy");
    assert_eq!(rec["supervisor_profile"], "supervisor");

    let calls: Vec<Vec<String>> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        &calls[0][..2],
        &["collective".to_string(), "init".to_string()]
    );
    assert!(calls[1].iter().any(|a| a == "--collective"));
    assert!(calls[1]
        .windows(2)
        .any(|w| w[0] == "--stage" && w[1] == "code"));
    assert!(calls[2]
        .windows(2)
        .any(|w| w[0] == "--stage" && w[1] == "deploy"));
    assert!(calls[1].contains(&"--json".to_string()));

    assert!(coll.join("collective.toml").is_file());
    let sup = std::fs::read_to_string(coll.join("profiles/supervisor.md")).unwrap();
    assert!(sup.contains("# vpak bootstrap: hello-service"));
    let sup_toml = std::fs::read_to_string(coll.join("profiles/supervisor.toml")).unwrap();
    assert!(sup_toml.contains("cycle = \"15m\""));
    assert!(sup_toml.contains("Bash(vflt *)"));
    assert!(wd.join("fleet/collective.toml").is_file());
    assert!(!wd.join("fleet/collective").exists(), "no symlink layout");

    let st = status_json(tmp.path(), &wd);
    assert!(st["fleet"].as_str().is_some());

    // A second handoff is refused.
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            w,
            "fleet",
            "bootstrap",
            "--collective",
            coll.to_str().unwrap(),
        ],
        &[
            ("VFLT_BIN", &fake),
            ("VPAK_FIXTURE_LOG", log.to_str().unwrap()),
        ],
    );
    assert!(!o.status.success());
}

#[test]
fn fleet_bootstrap_without_vflt_reports_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack_example(tmp.path());
    let wd = tmp.path().join("wd");
    let o = run(
        tmp.path(),
        &[
            "--workdir",
            wd.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--mode",
            "auto",
            "--runner",
            "shell",
            "--print-bootstrap",
        ],
        &[],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let missing = tmp.path().join("definitely-not-vflt");
    let o = run(
        tmp.path(),
        &["--workdir", wd.to_str().unwrap(), "fleet", "bootstrap"],
        &[("VFLT_BIN", missing.to_str().unwrap())],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("vflt is not installed"));
}

#[test]
fn runners_lists_claude_and_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let o = run(
        tmp.path(),
        &["--json", "runners"],
        &[("VPAK_SHELL_RUNNER_CMD", &fixture_cmd("agent"))],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let names: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x[0].as_str().unwrap())
        .collect();
    assert!(names.contains(&"claude"));
    assert!(names.contains(&"shell"));
    let shell = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x[0] == "shell")
        .unwrap();
    assert!(shell[1].get("Available").is_some(), "{shell}");
}
