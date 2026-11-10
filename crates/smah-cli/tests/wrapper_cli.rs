//! SW-02 CLI integration: the deterministic wrapper surface (flag form,
//! subcommands, error mapping). Every test runs in a fully sandboxed
//! HOME/XDG/config tree under a tempdir — never the real `~/.config`.

use assert_cmd::Command;
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
    wrappers: std::path::PathBuf,
    tools: std::path::PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().expect("root tmp");
        let wrappers = root.path().join("wrappers");
        let tools = root.path().join("tools");
        std::fs::create_dir_all(&wrappers).expect("wrappers dir");
        std::fs::create_dir_all(&tools).expect("tools dir");
        Sandbox {
            _root: root,
            wrappers,
            tools,
        }
    }

    /// Install a wrapper file.
    fn wrapper(&self, name: &str, body: &str) {
        std::fs::write(self.wrappers.join(format!("{name}.md")), body).expect("wrapper fixture");
    }

    /// A `smah` invocation wired to this sandbox.
    fn smah(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("SMAH_WRAPPERS_DIR", &self.wrappers)
            .env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env(
                "XDG_CONFIG_HOME",
                self.wrappers.parent().unwrap().join("xdg-config"),
            )
            .env(
                "XDG_STATE_HOME",
                self.wrappers.parent().unwrap().join("xdg-state"),
            )
            .env(
                "XDG_CACHE_HOME",
                self.wrappers.parent().unwrap().join("xdg-cache"),
            );
        cmd
    }
}

const DEMO: &str = "---\n\
name: session-transcript\ndescription: Turn a recording into a transcript\n\
params:\n  - name: cast_file\n    required: true\n  - name: label\n    default: transcript\n\
steps:\n  - id: to-text\n    use: cast-to-text\n    description: Convert cast events\n    with: {cast_file: \"{{params.cast_file}}\"}\n    output: text\n  - id: show\n    run: [\"echo\", \"{{params.label}}:\", \"{{steps.to-text.output}}\"]\n    timeout_secs: 15\noutputs:\n  - name: text\n    description: The transcript\n---\n";

#[test]
fn flag_path_prints_resolved_plan_and_smah_run_line() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args([
            "--wrapper",
            "session-transcript",
            "--param",
            "cast_file=s.cast",
        ])
        .assert()
        .code(0)
        .stdout(predicates::str::contains(
            "smah wrapper 'session-transcript' — Turn a recording into a transcript",
        ))
        .stdout(predicates::str::contains(
            "1. use cast-to-text [to-text] — Convert cast events",
        ))
        .stdout(predicates::str::contains(
            "argv: cast-to-text --cast-file s.cast",
        ))
        .stdout(predicates::str::contains("{{steps.to-text.output}}"))
        .stdout(predicates::str::contains("timeout: 15s"))
        .stdout(predicates::str::contains("outputs:"))
        .stdout(predicates::str::contains(
            "smah-run session-transcript --param cast_file=s.cast",
        ))
        .stdout(predicates::str::contains("NOT approved"));
}

#[test]
fn missing_required_param_is_usage_exit_2() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args(["--wrapper", "session-transcript"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("missing required param"))
        .stderr(predicates::str::contains("cast_file"));
}

#[test]
fn unknown_param_is_usage_exit_2() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args([
            "--wrapper",
            "session-transcript",
            "--param",
            "cast_file=a",
            "--param",
            "typo=x",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not declared"));
}

#[test]
fn unknown_wrapper_is_exit_2_with_installed_list() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args(["--wrapper", "ghost"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("no wrapper named 'ghost'"))
        .stderr(predicates::str::contains("session-transcript"));
}

#[test]
fn invalid_wrapper_file_is_exit_3_with_reason() {
    let sb = Sandbox::new();
    sb.wrapper(
        "broken",
        "---\nname: broken\ndescription: d\nsteps: []\n---\n",
    );
    sb.smah()
        .args(["--wrapper", "broken"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("invalid"))
        .stderr(predicates::str::contains("at least one step"));
}

#[test]
fn wrapper_list_table_and_json() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.wrapper(
        "broken",
        "---\nname: broken\ndescription: d\nsteps: []\n---\n",
    );
    sb.smah()
        .args(["wrapper", "list"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("session-transcript"))
        .stdout(predicates::str::contains("valid"))
        .stdout(predicates::str::contains("broken"))
        .stdout(predicates::str::contains("invalid"));
    sb.smah()
        .args(["wrapper", "list", "--json"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains(
            "\"name\": \"session-transcript\"",
        ))
        .stdout(predicates::str::contains("\"steps\": 2"))
        .stdout(predicates::str::contains("\"reason\""));
}

#[test]
fn wrapper_show_prints_the_file() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args(["wrapper", "show", "session-transcript"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("name: session-transcript"))
        .stdout(predicates::str::contains("use: cast-to-text"));
}

#[test]
fn wrapper_validate_green_with_param_hint() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args(["wrapper", "validate", "session-transcript"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("is valid"))
        .stdout(predicates::str::contains("--param for: cast_file"));
}

#[test]
fn wrapper_run_matches_flag_path() {
    let sb = Sandbox::new();
    sb.wrapper("session-transcript", DEMO);
    sb.smah()
        .args([
            "wrapper",
            "run",
            "session-transcript",
            "--param",
            "cast_file=s.cast",
        ])
        .assert()
        .code(0)
        .stdout(predicates::str::contains(
            "argv: cast-to-text --cast-file s.cast",
        ))
        .stdout(predicates::str::contains("smah-run session-transcript"));
}

#[test]
fn approved_wrapper_reports_approval_in_plan() {
    // Stamp programmatically (C1): the stamp is a content-bound {at, sha256}
    // mapping, computed over the wrapper's executable sections.
    let sb = Sandbox::new();
    let raw = "---\nname: ok\ndescription: d\nsteps:\n  - id: s1\n    run: [\"true\"]\n---\n";
    sb.wrapper("ok", raw);
    let def = smah_core::wrappers::loader::load_from(&sb.wrappers.join("ok.md")).expect("loads");
    let stamp = smah_core::wrappers::approval::stamp_for(&def, "2026-11-10T08:00:00-08:00");
    let mut stamped = def.clone();
    stamped.approved = Some(stamp);
    let content = smah_core::wrappers::prompt::render_wrapper_file(&stamped);
    std::fs::write(sb.wrappers.join("ok.md"), content).expect("stamped file");
    sb.smah()
        .args(["--wrapper", "ok"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains(
            "approved 2026-11-10T08:00:00-08:00",
        ))
        .stdout(predicates::str::contains("content sha256"));
}

#[test]
fn legacy_bare_timestamp_is_rejected_as_invalid() {
    let sb = Sandbox::new();
    sb.wrapper(
        "ok",
        "---\nname: ok\ndescription: d\napproved: 2026-11-09T09:00:00-08:00\nsteps:\n  - id: s1\n    run: [\"true\"]\n---\n",
    );
    sb.smah()
        .args(["--wrapper", "ok"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("REJECTED legacy format"));
}

#[test]
fn unknown_builtin_is_exit_3_at_resolve() {
    let sb = Sandbox::new();
    sb.wrapper(
        "ghosty",
        "---\nname: ghosty\ndescription: d\nsteps:\n  - id: s1\n    use: no-such-tool\n    with: {a: \"1\"}\n---\n",
    );
    sb.smah()
        .args(["--wrapper", "ghosty"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("unknown built-in tool"));
}

#[test]
fn back_compat_table_unaffected() {
    // The pre-existing surface still parses: bare smah and tools list —
    // wrappers did not displace the command tree. Bare smah's contract
    // changed at the RA sync (rust-ask owns the bare path now: help exit 0
    // on a non-TTY; a TTY without config gets the config-init offer) — the
    // old develop-era exit-2 M5 notice is gone.
    let sb = Sandbox::new();
    sb.smah()
        .assert()
        .code(0)
        .stdout(predicates::str::contains("Usage: smah"));
    sb.smah()
        .args(["tools", "list"])
        .assert()
        .code(0)
        .stderr(predicates::str::contains("no virtual tools"));
}
