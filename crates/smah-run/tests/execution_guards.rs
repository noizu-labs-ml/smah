//! smah-run execution-guard integration tests (review fixes C1/C2/H1/M1):
//! content-bound approval (`smah-run approve` + exit 4 refusals),
//! audit-before-execution fail-closed, process-group timeout kills, and
//! audit redaction of every param value. Every test runs in a fully
//! sandboxed XDG tree under a tempdir — never the real `~/.config`.

use std::collections::HashSet;
use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
    wrappers: PathBuf,
    state: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().expect("root tmp");
        let wrappers = root.path().join("wrappers");
        let state = root.path().join("state");
        std::fs::create_dir_all(&wrappers).expect("wrappers dir");
        std::fs::create_dir_all(&state).expect("state dir");
        Sandbox {
            _root: root,
            wrappers,
            state,
        }
    }

    fn wrapper_path(&self, name: &str) -> PathBuf {
        self.wrappers.join(format!("{name}.md"))
    }

    fn write_wrapper(&self, name: &str, body: &str) {
        std::fs::write(self.wrapper_path(name), body).expect("wrapper fixture");
    }

    /// Stamp the wrapper exactly the way `smah-run approve` does — the
    /// programmatic, TTY-free equivalent of the same byte path.
    fn approve(&self, name: &str) {
        let path = self.wrapper_path(name);
        let raw = std::fs::read_to_string(&path).expect("read wrapper");
        let def = smah_core::wrappers::loader::parse_wrapper(&raw, name, &HashSet::new())
            .expect("fixture loads");
        let stamp = smah_core::wrappers::approval::stamp_for(&def, "2026-11-10T08:00:00-08:00");
        let mut stamped = def.clone();
        stamped.approved = Some(stamp);
        let content = smah_core::wrappers::prompt::render_wrapper_file(&stamped);
        std::fs::write(&path, content).expect("write stamp");
    }

    /// A `smah-run` invocation wired to this sandbox.
    fn run(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah-run").expect("smah-run binary");
        cmd.env("SMAH_WRAPPERS_DIR", &self.wrappers)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CONFIG_HOME", self._root.path().join("config"))
            .env("XDG_CACHE_HOME", self._root.path().join("cache"));
        cmd
    }

    fn audit_lines(&self) -> Vec<String> {
        let path = self.state.join("smah/audit-smah-run.jsonl");
        std::fs::read_to_string(path)
            .expect("audit file exists")
            .lines()
            .map(str::to_string)
            .collect()
    }
}

const WRAPPER: &str = "---\n\
name: demo\ndescription: d\n\
params:\n  - name: greeting\n    default: hello\n\
steps:\n  - id: s1\n    run: [\"echo\", \"{{params.greeting}}\"]\n\
---\n";

#[test]
fn approve_then_execute_happy_path() {
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    // Approval is a human act: a non-TTY without the explicit flag refuses.
    sb.run()
        .args(["approve", "demo"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--yes-i-reviewed-this"));
    // The explicit flag (scripted review) stamps the file.
    sb.run()
        .args(["approve", "demo", "--yes-i-reviewed-this"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("approved '"));
    let stamped = std::fs::read_to_string(sb.wrapper_path("demo")).expect("read");
    assert!(stamped.contains("approved:"), "{stamped}");
    assert!(stamped.contains("sha256:"), "{stamped}");
    assert!(stamped.contains("at:"), "{stamped}");
    // And the stamped wrapper executes.
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("completed"));
}

#[test]
fn execute_refuses_without_stamp_exit_4() {
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("no approval stamp"));
}

#[test]
fn edit_after_approve_is_refused_exit_4() {
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    sb.approve("demo");
    // Tamper with an executable part AFTER approval — the stamp must
    // invalidate (C1 is about content, not time). Edit the STAMPED file so
    // the stamp itself survives, mismatched.
    let stamped = std::fs::read_to_string(sb.wrapper_path("demo")).expect("read");
    let edited = stamped.replace("- echo", "- echo-tampered");
    assert_ne!(edited, stamped);
    sb.write_wrapper("demo", &edited);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("content changed since approval"));
}

#[test]
fn forged_stamp_refuses_exit_4() {
    let sb = Sandbox::new();
    // A plausible-looking stamp whose hash binds nothing real: well-formed
    // shape, wrong content — refused exactly like an edit-after-approve.
    let forged = WRAPPER.replacen(
        "---\n",
        &format!(
            "---\napproved:\n  at: 2026-11-10T08:00:00-08:00\n  sha256: {}\n",
            "a".repeat(64)
        ),
        1,
    );
    sb.write_wrapper("demo", &forged);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("content changed since approval"));
}

#[test]
fn malformed_stamps_refuse_exit_4() {
    // Bad hex.
    let sb = Sandbox::new();
    let bad_hex = WRAPPER.replacen(
        "---\n",
        "---\napproved:\n  at: 2026-11-10T08:00:00-08:00\n  sha256: zz\n",
        1,
    );
    sb.write_wrapper("demo", &bad_hex);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("malformed approval stamp"));

    // Bad timestamp.
    let sb = Sandbox::new();
    let bad_at = WRAPPER.replacen(
        "---\n",
        "---\napproved:\n  at: yesterday\n  sha256: 0123\n",
        1,
    );
    sb.write_wrapper("demo", &bad_at);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("malformed approval stamp"));

    // Structurally broken (missing the hash field) is a load error.
    let sb = Sandbox::new();
    let missing_sha = WRAPPER.replacen(
        "---\n",
        "---\napproved:\n  at: 2026-11-10T08:00:00-08:00\n",
        1,
    );
    sb.write_wrapper("demo", &missing_sha);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("missing 'sha256'"));
}

#[test]
fn legacy_bare_timestamp_is_a_rejected_format() {
    let sb = Sandbox::new();
    let legacy = WRAPPER.replacen("---\n", "---\napproved: 2026-11-10T08:00:00-08:00\n", 1);
    sb.write_wrapper("demo", &legacy);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("REJECTED legacy format"));
}

#[test]
fn prompt_accept_leaves_it_unapproved() {
    // The --prompt accept path (modeled through the same apply_changes seam
    // smah uses): an APPROVED source composed through a no-op change yields
    // an UNAPPROVED recipe — model output can never carry or add approval.
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    sb.approve("demo");
    let raw = std::fs::read_to_string(sb.wrapper_path("demo")).expect("read");
    let def =
        smah_core::wrappers::loader::parse_wrapper(&raw, "demo", &HashSet::new()).expect("loads");
    assert!(def.approved.is_some(), "fixture is approved");
    let changes: smah_core::wrappers::prompt::RecipeChanges =
        serde_json::from_str(r#"{"explanation":"no-op","changes":{}}"#).expect("changes");
    let (next, content) =
        smah_core::wrappers::prompt::apply_changes(&def, &changes, &HashSet::new())
            .expect("applies");
    assert!(next.approved.is_none(), "apply_changes strips approval");
    assert!(
        !content.contains("approved:"),
        "composed file carries no stamp: {content}"
    );
    sb.write_wrapper("demo", &content);
    sb.run()
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(4)
        .stderr(predicates::str::contains("no approval stamp"));
}

#[test]
fn timeout_kills_the_whole_process_group_and_audits_first() {
    // H1: the step spawns a GRANDCHILD (sh is the group leader; sleep
    // inherits the group). On timeout the WHOLE group must die. `sh -c` is
    // test-fixture-only — the gate scans crates/*/src, not tests/.
    let sb = Sandbox::new();
    let pidfile = sb._root.path().join("grandchild.pid");
    let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
    let wrapper = format!(
        "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"sh\", \"-c\", \"{script}\"]\n    timeout_secs: 1\n---\n"
    );
    sb.write_wrapper("demo", &wrapper);
    sb.approve("demo");

    let output = sb
        .run()
        .args(["demo", "--execute", "--yes"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(1), "a timeout fails the run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("timed out"), "{stderr}");

    // The grandchild did NOT survive the group kill.
    let pid = std::fs::read_to_string(&pidfile)
        .expect("grandchild pid recorded")
        .trim()
        .to_string();
    let alive = Command::new("kill")
        .args(["-0", &pid])
        .output()
        .expect("kill probe")
        .status
        .success();
    assert!(!alive, "grandchild {pid} survived the process-group kill");

    // C2: the durable `started` record exists (written BEFORE the spawn),
    // plus the terminal `finished` record naming the timeout.
    let lines = sb.audit_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"decision\":\"started\"") && l.contains("\"step\":\"s1\"")),
        "started record present: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("\"decision\":\"finished\"") && l.contains("timed out")),
        "finished record with the timeout: {lines:?}"
    );
}

#[test]
fn audit_records_every_param_as_name_plus_sha_prefix_unless_plain() {
    // M1 end-to-end: the required param's value is a name + sha256 prefix;
    // the `audit: plain` opt-in keeps its value (post-redactor).
    let sb = Sandbox::new();
    let wrapper = "---\n\
name: demo\ndescription: d\n\
params:\n  - name: token\n    required: true\n  - name: label\n    default: public\n    audit: plain\n\
steps:\n  - id: s1\n    run: [\"echo\", \"{{params.token}}\", \"{{params.label}}\"]\n\
---\n";
    sb.write_wrapper("demo", wrapper);
    sb.approve("demo");
    sb.run()
        .args([
            "demo",
            "--execute",
            "--yes",
            "--param",
            "token=hunter2-secret",
        ])
        .assert()
        .code(0);
    let joined = sb.audit_lines().join("\n");
    assert!(
        !joined.contains("hunter2-secret"),
        "the param value never lands verbatim: {joined}"
    );
    assert!(
        joined.contains("\"token\":\"sha256:"),
        "named + hashed: {joined}"
    );
    assert!(
        joined.contains("public"),
        "audit: plain keeps its value: {joined}"
    );
    assert!(
        joined.contains("\"label\":\"public\""),
        "plain param named and verbatim: {joined}"
    );
    // argv carries the tokens, not the values.
    let started = sb
        .audit_lines()
        .into_iter()
        .find(|l| l.contains("\"decision\":\"started\""))
        .expect("started record");
    assert!(!started.contains("hunter2-secret"), "{started}");
}

#[test]
fn unusable_audit_refuses_to_execute_fail_closed() {
    // C2: if the `started` record cannot be written durably, the step does
    // NOT run — here the state dir cannot even be created (XDG_STATE_HOME
    // points at a regular file).
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    sb.approve("demo");
    let blocker = sb._root.path().join("blocker");
    std::fs::write(&blocker, "x").expect("blocker file");
    sb.run()
        .env("XDG_STATE_HOME", &blocker)
        .args(["demo", "--execute", "--yes"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("fail-closed"));
}

#[test]
fn dry_run_writes_no_audit() {
    let sb = Sandbox::new();
    sb.write_wrapper("demo", WRAPPER);
    sb.run().arg("demo").assert().code(0);
    assert!(
        !sb.state.join("smah/audit-smah-run.jsonl").exists(),
        "dry-runs stay off the audit trail"
    );
}
