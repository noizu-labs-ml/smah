//! SW-05 CLI integration: the smah-run executor contract — dry-run by
//! default, approval gate, `--yes` non-interactive execution, fixed argv
//! with `{{steps.*.output}}` binding, in-process built-ins, the audit JSONL
//! (metadata + redaction), failure and timeout handling. Everything runs
//! in a sandboxed env tree; fixed argv only (no shell anywhere).

use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
    wrappers: PathBuf,
    tools: PathBuf,
    state: PathBuf,
    out: PathBuf,
    audit: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("root tmp");
        let wrappers = root.path().join("wrappers");
        let tools = root.path().join("tools");
        let state = root.path().join("xdg-state");
        let out = root.path().join("out");
        std::fs::create_dir_all(&wrappers).expect("wrappers dir");
        std::fs::create_dir_all(&tools).expect("tools dir");
        std::fs::create_dir_all(&state).expect("state dir");
        std::fs::create_dir_all(&out).expect("out dir");
        Sandbox {
            audit: state.join("smah").join("audit-smah-run.jsonl"),
            _root: root,
            wrappers,
            tools,
            state,
            out,
        }
    }

    fn wrapper(&self, name: &str, body: &str) {
        std::fs::write(self.wrappers.join(format!("{name}.md")), body).expect("wrapper fixture");
    }

    fn runner(&self, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("smah-run").expect("smah-run binary");
        cmd.env("SMAH_WRAPPERS_DIR", &self.wrappers)
            .env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env("XDG_STATE_HOME", &self.state)
            .env(
                "XDG_CACHE_HOME",
                self.state.parent().unwrap().join("xdg-cache"),
            )
            .args(args);
        cmd
    }
}

/// Approved three-step recipe: output binding, substitution into a later
/// argv, and an observable filesystem effect.
const APPROVED: &str = "---\n\
name: demo\ndescription: fixture recipe\napproved: 2026-11-09T09:00:00-08:00\n\
params:\n  - name: out_dir\n    required: true\n\
steps:\n  - id: greet\n    run: [\"echo\", \"hello\"]\n    output: greeting\n  - id: shout\n    run: [\"echo\", \"{{steps.greet.output}}!\"]\n    output: shouted\n  - id: mark\n    run: [\"touch\", \"{{params.out_dir}}/done.txt\"]\n    timeout_secs: 5\n\
outputs:\n  - name: greeting\n  - name: shouted\n---\n";

#[test]
fn dry_run_is_the_default_and_executes_nothing() {
    let sb = Sandbox::new();
    sb.wrapper("demo", APPROVED);
    sb.runner(&["demo", "--param", "out_dir=/tmp", "--yes"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("dry-run"))
        .stdout(predicates::str::contains(
            "smah-run demo --param out_dir=/tmp",
        ));
    assert!(!sb.out.join("done.txt").exists(), "nothing ran");
    assert!(!sb.audit.exists(), "a dry-run writes no audit records");
}

#[test]
fn execute_without_an_approved_stamp_refuses() {
    let sb = Sandbox::new();
    sb.wrapper(
        "shy",
        "---\nname: shy\ndescription: unapproved\nsteps:\n  - id: s1\n    run: [\"true\"]\n---\n",
    );
    sb.runner(&["shy", "--execute", "--yes"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("approved:"))
        .stderr(predicates::str::contains("refused"));
    assert!(!sb.audit.exists(), "nothing ran, nothing audited");
}

#[test]
fn execute_on_a_pipe_without_yes_refuses() {
    let sb = Sandbox::new();
    sb.wrapper("demo", APPROVED);
    sb.runner(&["demo", "--param", "out_dir=x", "--execute"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--yes"));
    assert!(!sb.out.join("done.txt").exists());
}

#[test]
fn execute_yes_runs_steps_binds_outputs_and_audits() {
    let sb = Sandbox::new();
    sb.wrapper("demo", APPROVED);
    let out_dir = sb.out.display().to_string();
    sb.runner(&[
        "demo",
        "--param",
        &format!("out_dir={out_dir}"),
        "--execute",
        "--yes",
    ])
    .write_stdin("")
    .assert()
    .code(0)
    .stdout(predicates::str::contains("executing wrapper 'demo'"))
    .stdout(predicates::str::contains("output greeting"))
    .stdout(predicates::str::contains("hello"))
    .stdout(predicates::str::contains("completed"));

    assert!(sb.out.join("done.txt").exists(), "the touch step ran");
    let audit = std::fs::read_to_string(&sb.audit).expect("audit jsonl");
    let lines: Vec<&str> = audit.lines().collect();
    assert_eq!(lines.len(), 3, "one record per step: {audit}");
    for (i, line) in lines.iter().enumerate() {
        let rec: serde_json::Value = serde_json::from_str(line).expect("audit line is json");
        assert_eq!(rec["decision"], "executed");
        assert_eq!(rec["step"], ["greet", "shout", "mark"][i]);
        let argv = rec["argv"].as_str().unwrap();
        assert!(argv.starts_with(["echo", "echo", "touch"][i]), "{argv}");
    }
}

#[test]
fn failing_step_aborts_the_run_with_exit_1() {
    let sb = Sandbox::new();
    sb.wrapper(
        "bad",
        "---\nname: bad\ndescription: fails fast\napproved: 2026-11-09T09:00:00-08:00\nsteps:\n  - id: s1\n    run: [\"false\"]\n---\n",
    );
    sb.runner(&["bad", "--execute", "--yes"])
        .write_stdin("")
        .assert()
        .code(1)
        .stderr(predicates::str::contains("failed"))
        .stderr(predicates::str::contains("exit status 1"));
    let audit = std::fs::read_to_string(&sb.audit).expect("audit jsonl");
    assert!(audit.contains("\"decision\":\"failed\""), "{audit}");
}

#[test]
fn timeout_kills_the_step() {
    let sb = Sandbox::new();
    sb.wrapper(
        "slow",
        "---\nname: slow\ndescription: sleeps\napproved: 2026-11-09T09:00:00-08:00\nsteps:\n  - id: s1\n    run: [\"sleep\", \"30\"]\n    timeout_secs: 1\n---\n",
    );
    sb.runner(&["slow", "--execute", "--yes"])
        .write_stdin("")
        .assert()
        .code(1)
        .stderr(predicates::str::contains("timed out after 1s"));
    let audit = std::fs::read_to_string(&sb.audit).expect("audit jsonl");
    assert!(audit.contains("timed out"), "{audit}");
}

#[test]
fn builtins_execute_in_process_and_bind() {
    let sb = Sandbox::new();
    let cast = sb.out.join("session.cast");
    std::fs::write(
        &cast,
        concat!(
            r#"{"version": 2, "width": 80, "height": 24}"#,
            "\n",
            r#"[0.1, "o", "\u001b[32mhi\u001b[0m from the cast"]"#,
            "\n"
        ),
    )
    .expect("cast fixture");
    sb.wrapper(
        "transcribe",
        "---\n\
name: transcribe\ndescription: cast to text\napproved: 2026-11-09T09:00:00-08:00\n\
params:\n  - name: cast_file\n    required: true\n\
steps:\n  - id: to-text\n    use: cast-to-text\n    description: Convert cast events\n    with: {cast_file: \"{{params.cast_file}}\"}\n    output: text\n\
outputs:\n  - name: text\n---\n",
    );
    sb.runner(&[
        "transcribe",
        "--param",
        &format!("cast_file={}", cast.display()),
        "--execute",
        "--yes",
    ])
    .write_stdin("")
    .assert()
    .code(0)
    .stdout(predicates::str::contains("hi from the cast"));
    let audit = std::fs::read_to_string(&sb.audit).expect("audit jsonl");
    assert!(
        audit.contains("cast-to-text"),
        "the builtin step is audited: {audit}"
    );
}
