//! Virtual-tools spec: loader, directive scanner, allowlist/file surface,
//! driver loop (with a fake backend — no network), and the audit log
//! (plan §4 test matrix).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use smah_core::errors::SmahError;
use smah_core::virtual_tools::audit::{Audit, AuditRecord};
use smah_core::virtual_tools::directives::{Directive, ScanError, Scanner};
use smah_core::virtual_tools::driver::{
    run_with_turns, ChatBackend, ChatMessage, FsMode, Role, Sink,
};
use smah_core::virtual_tools::fs_surface::{FileSurface, Outcome};
use smah_core::virtual_tools::loader::{discover_in, load_by_name, load_from, resolve_roots};
use smah_core::virtual_tools::{
    is_valid_name, DEFAULT_INPUT_BYTES, DIRECTIVE_LINE_MAX, READ_FILE_MAX, RUN_AGGREGATE_WRITE_MAX,
    TURNS_MAX, WRITE_CONTENT_MAX,
};

/// Serializes the env-mutating tests (the environment is process-global and
/// cargo test runs in parallel threads).
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vtools")
}

fn write_tool(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(format!("{name}.md"));
    fs::write(&path, contents).expect("write fixture tool");
    path
}

fn valid_tool_yaml(name: &str, extra: &str) -> String {
    format!("---\nname: {name}\ndescription: fixture {name}\n{extra}---\nBody of {name}.\n")
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

#[test]
fn loader_valid_fixture_parses_all_keys() {
    let def = load_from(&fixtures_dir().join("valid.md")).expect("valid fixture loads");
    assert_eq!(def.name, "valid");
    assert_eq!(def.model.as_deref(), Some("anthropic.claude-opus-5-5"));
    // Canonicalization is spec'd: the file says `driver: emulated` (the
    // permanent alias) and the loader stores the canonical name.
    assert_eq!(def.driver, "cli");
    assert_eq!(def.interface, None);
    assert_eq!(
        def.constraints,
        vec!["Never invent log lines that are not in the input."]
    );
    assert_eq!(def.allow_roots, vec!["~/notes", "$PROJECT_LOGS"]);
    assert!(def.body.starts_with("You are a meticulous log analyst."));
}

#[test]
fn loader_stem_mismatch_rejected() {
    let err = load_from(&fixtures_dir().join("stem-mismatch.md")).unwrap_err();
    assert!(err.contains("does not match the filename stem"), "{err}");
    assert!(err.contains("different-name"), "{err}");
}

#[test]
fn loader_unknown_key_named_in_error() {
    let err = load_from(&fixtures_dir().join("unknown-key.md")).unwrap_err();
    assert!(
        err.contains("constrants"),
        "typo'd key must be named: {err}"
    );
}

#[test]
fn loader_reserved_name_rejected() {
    let err = load_from(&fixtures_dir().join("reserved-name.md")).unwrap_err();
    assert!(err.contains("reserved"), "{err}");
}

#[test]
fn loader_bad_name_rejected() {
    let err = load_from(&fixtures_dir().join("9start.md")).unwrap_err();
    assert!(err.contains("invalid"), "{err}");
}

#[test]
fn loader_unknown_driver_rejected() {
    let err = load_from(&fixtures_dir().join("unknown-driver.md")).unwrap_err();
    // 'fortran' is the fixture's deliberately-invalid driver; `db` became a
    // real taxonomy member in PR-0.
    assert!(err.contains("unknown driver 'fortran'"), "{err}");
    assert!(err.contains("'emulated' is an alias of 'cli'"), "{err}");
}

#[test]
fn loader_driver_taxonomy_accepts_all_nine_and_alias() {
    // Every taxonomy value loads; the "emulated" alias loads too and is
    // stored under its canonical name. In-test generation keeps this one
    // file (instead of 10 fixtures) — a PR-0 deviation noted in the plan.
    for value in smah_core::virtual_tools::drivers::DRIVER_NAMES {
        let dir = tempfile::tempdir().expect("tmp");
        write_tool(
            dir.path(),
            "d",
            &valid_tool_yaml("d", &format!("driver: {value}\n")),
        );
        let def = load_from(&dir.path().join("d.md")).expect(value);
        assert_eq!(def.driver, value, "canonical value stored verbatim");
    }
    let dir = tempfile::tempdir().expect("tmp");
    write_tool(dir.path(), "d", &valid_tool_yaml("d", "driver: emulated\n"));
    let def = load_from(&dir.path().join("d.md")).expect("alias loads");
    assert_eq!(def.driver, "cli");
}

#[test]
fn loader_driver_defaults_to_cli_when_absent() {
    let dir = tempfile::tempdir().expect("tmp");
    write_tool(dir.path(), "d", &valid_tool_yaml("d", ""));
    let def = load_from(&dir.path().join("d.md")).expect("loads");
    assert_eq!(def.driver, "cli");
}

#[test]
fn loader_interface_key_present_absent_and_wrong_type() {
    // String value → Some (the extension point other driver lanes read).
    let dir = tempfile::tempdir().expect("tmp");
    write_tool(
        dir.path(),
        "i",
        &valid_tool_yaml(
            "i",
            "driver: mcp\ninterface: https://example.com/smah.json\n",
        ),
    );
    let def = load_from(&dir.path().join("i.md")).expect("loads");
    assert_eq!(def.driver, "mcp");
    assert_eq!(
        def.interface.as_deref(),
        Some("https://example.com/smah.json")
    );

    // Absent → None (cli tools keep working unchanged).
    let dir = tempfile::tempdir().expect("tmp");
    write_tool(dir.path(), "i", &valid_tool_yaml("i", ""));
    let def = load_from(&dir.path().join("i.md")).expect("loads");
    assert_eq!(def.interface, None);

    // Non-string → rejected like every other typed key.
    let dir = tempfile::tempdir().expect("tmp");
    write_tool(
        dir.path(),
        "i",
        &valid_tool_yaml("i", "interface: [a, b]\n"),
    );
    let err = load_from(&dir.path().join("i.md")).unwrap_err();
    assert!(err.contains("'interface' must be a string"), "{err}");
}

#[test]
fn loader_missing_body_rejected() {
    let err = load_from(&fixtures_dir().join("no-body.md")).unwrap_err();
    assert!(err.contains("body"), "{err}");
}

#[test]
fn loader_discover_lists_invalid_with_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_tool(dir.path(), "good", &valid_tool_yaml("good", ""));
    write_tool(dir.path(), "broken", "---\nname: broken\n---\nbody\n"); // no description
    let (valid, invalid) = discover_in(dir.path());
    assert_eq!(valid.len(), 1);
    assert_eq!(valid[0].name, "good");
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].name, "broken");
    assert!(
        invalid[0].reason.contains("description"),
        "{}",
        invalid[0].reason
    );
}

#[test]
fn loader_load_by_name_missing_gives_tool_not_found_with_installed() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    write_tool(dir.path(), "present", &valid_tool_yaml("present", ""));
    std::env::set_var("SMAH_VIRTUAL_TOOLS_DIR", dir.path());
    let result = load_by_name("absent");
    std::env::remove_var("SMAH_VIRTUAL_TOOLS_DIR");
    match result {
        Err(SmahError::ToolNotFound { name, installed }) => {
            assert_eq!(name, "absent");
            assert_eq!(installed, "present");
        }
        other => panic!("expected ToolNotFound, got {other:?}"),
    }
}

#[test]
fn loader_load_by_name_invalid_gives_tool_file_invalid() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    write_tool(dir.path(), "bad", &valid_tool_yaml("mismatch", ""));
    std::env::set_var("SMAH_VIRTUAL_TOOLS_DIR", dir.path());
    let result = load_by_name("bad");
    std::env::remove_var("SMAH_VIRTUAL_TOOLS_DIR");
    match result {
        Err(SmahError::ToolFileInvalid { detail, .. }) => {
            assert!(detail.contains("does not match"), "{detail}");
        }
        other => panic!("expected ToolFileInvalid, got {other:?}"),
    }
}

#[test]
fn loader_resolve_roots_expands_env_and_rejects_missing() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let real_root = fs::canonicalize(dir.path()).expect("canonical tempdir");
    let tool_path = write_tool(
        dir.path(),
        "roots-mix",
        &format!(
            "---\nname: roots-mix\ndescription: resolve test\nallow_roots:\n  - \"{}\"\n  - $VTOOLS_TEST_ROOT\n---\nbody\n",
            real_root.display()
        ),
    );
    let def = load_from(&tool_path).expect("parses");
    std::env::set_var("VTOOLS_TEST_ROOT", "/tmp");
    let roots = resolve_roots(&def).expect("roots resolve");
    std::env::remove_var("VTOOLS_TEST_ROOT");
    assert_eq!(roots[0], real_root);
    assert_eq!(
        roots[1],
        fs::canonicalize("/tmp").unwrap_or_else(|_| PathBuf::from("/tmp"))
    );

    // Missing env var ⇒ invalid at load-for-invoke, naming the variable.
    std::env::remove_var("VTOOLS_TEST_ROOT");
    match resolve_roots(&def) {
        Err(SmahError::ToolFileInvalid { detail, .. }) => {
            assert!(detail.contains("VTOOLS_TEST_ROOT"), "{detail}");
        }
        other => panic!("expected ToolFileInvalid, got {other:?}"),
    }
}

#[test]
fn loader_missing_dir_discover_is_empty() {
    let (valid, invalid) = discover_in(Path::new("/nonexistent/vtools/definitely/absent"));
    assert!(valid.is_empty() && invalid.is_empty());
}

#[test]
fn name_validator_matches_plan_regex() {
    assert!(is_valid_name("log-analyzer"));
    assert!(is_valid_name("a"));
    assert!(is_valid_name("a_b-c9"));
    assert!(!is_valid_name("BadName"));
    assert!(!is_valid_name("9start"));
    assert!(!is_valid_name("has space"));
    assert!(!is_valid_name(""));
    assert!(!is_valid_name(&"a".repeat(66)));
    assert!(is_valid_name(&"a".repeat(64)));
}

// ---------------------------------------------------------------------------
// Directive scanner
// ---------------------------------------------------------------------------

fn collect(scanner: &mut Scanner, chunks: &[&str]) -> (String, Vec<Directive>) {
    let mut content = String::new();
    for chunk in chunks {
        scanner.feed(chunk, &mut |piece| content.push_str(piece));
    }
    scanner.finish(&mut |piece| content.push_str(piece));
    let directives = scanner.take_directives();
    (content, directives)
}

#[test]
fn scanner_mid_stream_directive_intercepted() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(
        &mut scanner,
        &[
            "Starting the ",
            "answer here.\n@@smah write {\"path\": \"out.md\", \"content\": \"hi\"}\n",
            "After the directive.\n",
        ],
    );
    assert_eq!(content, "Starting the answer here.\nAfter the directive.\n");
    assert_eq!(
        directives,
        vec![Directive::Write {
            path: "out.md".to_string(),
            content: "hi".to_string(),
        }]
    );
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_sentinel_prefix_bytes_held_not_leaked() {
    // Feed the sentinel byte-by-byte: nothing may reach content until the
    // line resolves either way.
    let mut scanner = Scanner::new();
    let mut content = String::new();
    for c in "@@smah read {\"path\": \"x\"}\ndone\n".chars() {
        scanner.feed(&c.to_string(), &mut |piece| content.push_str(piece));
    }
    scanner.finish(&mut |piece| content.push_str(piece));
    assert_eq!(content, "done\n");
    assert_eq!(scanner.take_directives().len(), 1);
}

#[test]
fn scanner_sentinel_mid_line_passes_through() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["see the docs at @@smah for more\n"]);
    assert_eq!(content, "see the docs at @@smah for more\n");
    assert!(directives.is_empty());
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_malformed_json_is_protocol_error() {
    let mut scanner = Scanner::new();
    let (_, directives) = collect(&mut scanner, &["@@smah write {not json}\n"]);
    assert!(directives.is_empty());
    assert!(
        matches!(
            scanner.error(),
            Some(ScanError::Protocol { detail }) if detail.contains("did not parse")
        ),
        "{:?}",
        scanner.error()
    );
}

#[test]
fn scanner_unknown_verb_is_violation_class() {
    let mut scanner = Scanner::new();
    let (_, directives) = collect(&mut scanner, &["@@smah delete {\"path\": \"x\"}\n"]);
    assert!(directives.is_empty());
    assert_eq!(
        scanner.error(),
        Some(&ScanError::UnknownVerb {
            verb: "delete".to_string(),
            known: smah_core::virtual_tools::KNOWN_VERBS.as_slice(),
        })
    );
}

#[test]
fn scanner_with_verbs_accepts_extension_verb_raw() {
    // The PR-0 seam: a driver lane (e.g. tui) builds a Scanner over its own
    // verb set; verbs outside the base set arrive as Directive::Extension
    // with the payload carried **raw** (no JSON validation here — the
    // driver owns the format).
    let mut scanner = Scanner::with_verbs(&["read", "ui"]);
    let (content, directives) = collect(&mut scanner, &["@@smah ui not-json-at-all\n"]);
    assert_eq!(content, "");
    assert_eq!(
        directives,
        vec![Directive::Extension {
            verb: "ui".to_string(),
            payload: "not-json-at-all".to_string(),
        }]
    );
    assert!(scanner.error().is_none());
    // Base verbs still work on an extended scanner.
    let (_, directives) = collect(&mut scanner, &["@@smah read {\"path\": \"a\"}\n"]);
    assert!(matches!(directives.first(), Some(Directive::Read { .. })));
}

#[test]
fn scanner_with_verbs_reports_its_own_known_set() {
    // An extended scanner that sees a verb outside *its* set names its own
    // verbs in the error (not the base set).
    let mut scanner = Scanner::with_verbs(&["ui", "focus"]);
    let (_, directives) = collect(&mut scanner, &["@@smah write {\"path\": \"x\"}\n"]);
    assert!(directives.is_empty());
    assert_eq!(
        scanner.error(),
        Some(&ScanError::UnknownVerb {
            verb: "write".to_string(),
            known: &["ui", "focus"],
        })
    );
}

#[test]
fn scanner_oversized_line_is_protocol_error() {
    let mut scanner = Scanner::new();
    let payload = "x".repeat(DIRECTIVE_LINE_MAX + 1);
    let (content, directives) = collect(
        &mut scanner,
        &[&format!(
            "@@smah write {{\"path\": \"p\", \"content\": \"{payload}\"}}\n"
        )],
    );
    assert!(directives.is_empty());
    assert!(content.is_empty());
    assert!(matches!(scanner.error(), Some(ScanError::Protocol { .. })));
}

#[test]
fn scanner_crlf_tolerated() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(
        &mut scanner,
        &["line one\r\n@@smah read {\"path\": \"a b.md\"}\r\nline two\r\n"],
    );
    assert_eq!(
        directives,
        vec![Directive::Read {
            path: "a b.md".to_string()
        }]
    );
    assert_eq!(content, "line one\r\nline two\r\n");
}

#[test]
fn scanner_unterminated_final_sentinel_line_is_directive_attempt() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["text\n@@smah read {\"path\": \"x\"}"]);
    assert_eq!(content, "text\n");
    assert_eq!(directives.len(), 1);
}

#[test]
fn scanner_trailing_at_sign_is_content_not_protocol() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["email me @"]);
    assert_eq!(content, "email me @");
    assert!(directives.is_empty());
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_content_concatenation_preserves_everything_else() {
    let mut scanner = Scanner::new();
    let (content, _) = collect(&mut scanner, &["a\nb\nc"]);
    assert_eq!(content, "a\nb\nc");
}

#[test]
fn scanner_blank_lines_are_content_not_protocol() {
    // Regression: a blank line stays "deciding" (`sentinel.starts_with("")`
    // is vacuously true), which used to route it into the directive parser
    // and abort the whole run post-stream with a protocol error.
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["para one\n", "\n", "para two\n"]);
    assert_eq!(content, "para one\n\npara two\n");
    assert!(directives.is_empty());
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_blank_line_before_a_sentinel_line() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(
        &mut scanner,
        &[
            "intro\n",
            "\n",
            "@@smah read {\"path\": \"x.md\"}\n",
            "outro\n",
        ],
    );
    assert_eq!(content, "intro\n\noutro\n");
    assert_eq!(
        directives,
        vec![Directive::Read {
            path: "x.md".to_string()
        }]
    );
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_blank_line_after_a_sentinel_line() {
    let mut scanner = Scanner::new();
    let (content, directives) = collect(
        &mut scanner,
        &[
            "intro\n",
            "@@smah read {\"path\": \"x.md\"}\n",
            "\n",
            "outro\n",
        ],
    );
    assert_eq!(content, "intro\n\noutro\n");
    assert_eq!(
        directives,
        vec![Directive::Read {
            path: "x.md".to_string()
        }]
    );
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_lone_sentinel_prefix_line_is_content_not_protocol() {
    // "@\n" mid-stream used to abort; now the buffered prefix streams
    // through with its newline, byte-for-byte.
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["before\n", "@\n", "after\n"]);
    assert_eq!(content, "before\n@\nafter\n");
    assert!(directives.is_empty());
    assert!(scanner.error().is_none());
}

#[test]
fn scanner_unterminated_incomplete_sentinel_still_aborts_in_finish() {
    // A sentinel line that never completes (no trailing newline, malformed
    // payload) is still a directive attempt at end of stream — the
    // `finish()` guard is unchanged by the blank-line fix.
    let mut scanner = Scanner::new();
    let (content, directives) = collect(&mut scanner, &["text\n", "@@smah write {\"path\": \"p\""]);
    assert_eq!(content, "text\n");
    assert!(directives.is_empty());
    assert!(matches!(scanner.error(), Some(ScanError::Protocol { .. })));
}

#[test]
fn input_cap_default_is_256k() {
    assert_eq!(DEFAULT_INPUT_BYTES, 256 * 1024);
    assert_eq!(WRITE_CONTENT_MAX, 128 * 1024);
    assert_eq!(READ_FILE_MAX, 1024 * 1024);
    assert_eq!(RUN_AGGREGATE_WRITE_MAX, 1024 * 1024);
    assert_eq!(TURNS_MAX, 8);
}

// ---------------------------------------------------------------------------
// Allowlist / file surface
// ---------------------------------------------------------------------------

fn surface_with_roots(root: &Path) -> FileSurface {
    FileSurface::new(vec![root.to_path_buf()])
}

fn resolved(root: &Path, rel: &str) -> PathBuf {
    fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .join(rel)
}

#[test]
fn fs_write_read_append_happy_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("notes")).expect("notes dir");
    let mut surface = surface_with_roots(dir.path());

    let out = surface.execute(&Directive::Write {
        path: "notes/summary.md".to_string(),
        content: "# Summary\n\nbody".to_string(),
    });
    assert_eq!(
        out,
        Outcome::Allowed {
            resolved: resolved(dir.path(), "notes/summary.md"),
            bytes: "# Summary\n\nbody".len(),
            text: None,
        }
    );

    let out = surface.execute(&Directive::Append {
        path: "notes/summary.md".to_string(),
        content: " tail".to_string(),
    });
    assert!(matches!(out, Outcome::Allowed { bytes: 5, .. }));
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/summary.md")).expect("read back"),
        "# Summary\n\nbody tail"
    );

    let out = surface.execute(&Directive::Read {
        path: "notes/summary.md".to_string(),
    });
    match out {
        Outcome::Allowed { text, .. } => {
            assert_eq!(text.as_deref(), Some("# Summary\n\nbody tail"))
        }
        other => panic!("expected Allowed read, got {other:?}"),
    }
    assert_eq!(surface.aggregate_written(), "# Summary\n\nbody".len() + 5);
}

#[test]
fn fs_dotdot_traversal_is_violation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut surface = surface_with_roots(dir.path());
    let out = surface.execute(&Directive::Read {
        path: "sub/../../etc/passwd".to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));
}

#[test]
fn fs_absolute_outside_roots_is_violation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut surface = surface_with_roots(dir.path());
    let out = surface.execute(&Directive::Write {
        path: "/etc/passwd".to_string(),
        content: "nope".to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));
}

#[test]
fn fs_root_prefix_confusion_is_violation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let evil = dir.path().join("root-evil");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&evil).expect("evil");
    let mut surface = FileSurface::new(vec![root]);
    let out = surface.execute(&Directive::Read {
        path: evil.join("secret.txt").display().to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));
}

#[test]
fn fs_symlink_root_canonicalized_to_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let real = dir.path().join("real");
    fs::create_dir_all(&real).expect("real");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, dir.path().join("link")).expect("symlink");
    #[cfg(unix)]
    {
        let mut surface = FileSurface::new(vec![dir.path().join("link")]);
        let out = surface.execute(&Directive::Write {
            path: "inside.md".to_string(),
            content: "x".to_string(),
        });
        assert!(matches!(out, Outcome::Allowed { .. }));
        assert_eq!(
            fs::read_to_string(real.join("inside.md")).expect("written"),
            "x"
        );
    }
}

#[test]
fn fs_symlink_escape_read_rejected_write_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("target.txt"), "secret").expect("target");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.join("target.txt"), root.join("escape.txt"))
        .expect("symlink");

    // Read: the canonical target is outside the root — containment rejects.
    let mut surface = surface_with_roots(&root);
    let out = surface.execute(&Directive::Read {
        path: "escape.txt".to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));

    // Write: the final component is a symlink — refused outright.
    let out = surface.execute(&Directive::Write {
        path: "escape.txt".to_string(),
        content: "pwn".to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));
    assert_eq!(
        fs::read_to_string(outside.join("target.txt")).expect("target untouched"),
        "secret"
    );
}

#[test]
fn fs_symlink_to_inside_root_read_allowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    fs::create_dir_all(&root).expect("root");
    fs::write(root.join("real.txt"), "fine").expect("real");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("real.txt"), root.join("alias.txt")).expect("symlink");
    let mut surface = surface_with_roots(&root);
    let out = surface.execute(&Directive::Read {
        path: "alias.txt".to_string(),
    });
    match out {
        Outcome::Allowed { text, .. } => assert_eq!(text.as_deref(), Some("fine")),
        other => panic!("expected Allowed, got {other:?}"),
    }
}

#[test]
fn fs_new_file_under_existing_dir_allowed_missing_subdir_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("exists")).expect("dir");
    let mut surface = surface_with_roots(dir.path());

    let out = surface.execute(&Directive::Write {
        path: "exists/new.md".to_string(),
        content: "x".to_string(),
    });
    assert!(matches!(out, Outcome::Allowed { .. }));

    let out = surface.execute(&Directive::Write {
        path: "does/not/exist/new.md".to_string(),
        content: "x".to_string(),
    });
    assert!(matches!(out, Outcome::SoftError { .. }));
}

#[test]
fn fs_directory_read_is_soft_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut surface = surface_with_roots(dir.path());
    let out = surface.execute(&Directive::Read {
        path: ".".to_string(),
    });
    assert!(matches!(out, Outcome::SoftError { .. }));
}

#[test]
fn fs_oversize_read_is_soft_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let big = "a".repeat(READ_FILE_MAX + 1);
    fs::write(dir.path().join("big.txt"), &big).expect("big file");
    let mut surface = surface_with_roots(dir.path());
    let out = surface.execute(&Directive::Read {
        path: "big.txt".to_string(),
    });
    assert!(matches!(out, Outcome::SoftError { .. }));
}

#[test]
fn fs_over_cap_write_is_soft_reject_file_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut surface = surface_with_roots(dir.path());
    let big = "b".repeat(WRITE_CONTENT_MAX + 1);
    let out = surface.execute(&Directive::Write {
        path: "big.md".to_string(),
        content: big,
    });
    assert!(matches!(out, Outcome::SoftError { .. }));
    assert!(!dir.path().join("big.md").exists());
    assert_eq!(surface.aggregate_written(), 0);
}

#[test]
fn fs_aggregate_cap_breach_is_violation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut surface = surface_with_roots(dir.path());
    // Per-directive cap is 128 KiB, aggregate 1 MiB: eight max-size writes
    // are allowed, the ninth breaches the aggregate and aborts.
    let chunk = "x".repeat(WRITE_CONTENT_MAX);
    for i in 0..8 {
        let outcome = surface.execute(&Directive::Write {
            path: format!("f{i}.md"),
            content: chunk.clone(),
        });
        assert!(
            matches!(outcome, Outcome::Allowed { .. }),
            "write {i} should fit: {outcome:?}"
        );
    }
    let ninth = surface.execute(&Directive::Write {
        path: "f9.md".to_string(),
        content: chunk,
    });
    assert!(matches!(ninth, Outcome::Violation { .. }));
}

#[test]
fn fs_eacces_is_soft_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join("locked");
    fs::create_dir_all(&locked).expect("dir");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
        if fs::metadata(&locked).expect("meta").permissions().mode() & 0o777 != 0 {
            // Filesystem does not honor the mode (e.g. some CI mounts) — skip.
            return;
        }
        let mut surface = surface_with_roots(dir.path());
        let out = surface.execute(&Directive::Write {
            path: "locked/file.md".to_string(),
            content: "x".to_string(),
        });
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");
        assert!(matches!(out, Outcome::SoftError { .. }));
    }
}

#[test]
fn fs_no_roots_every_directive_is_violation() {
    let mut surface = FileSurface::new(vec![]);
    let out = surface.execute(&Directive::Read {
        path: "anything.md".to_string(),
    });
    assert!(matches!(out, Outcome::Violation { .. }));
}

// ---------------------------------------------------------------------------
// Driver loop (FakeBackend — no network)
// ---------------------------------------------------------------------------

/// One recorded request: (model, system, messages).
type SeenRequest = (String, String, Vec<ChatMessage>);

/// Scripted backend: pops one response per turn; records every request.
struct FakeBackend {
    script: Mutex<Vec<String>>,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
}

impl FakeBackend {
    fn new(responses: &[&str]) -> (Self, Arc<Mutex<Vec<SeenRequest>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            FakeBackend {
                script: Mutex::new(responses.iter().map(|s| s.to_string()).collect()),
                seen: seen.clone(),
            },
            seen,
        )
    }
}

impl ChatBackend for FakeBackend {
    fn stream_turn(
        &self,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).push((
            model.to_string(),
            system.to_string(),
            messages.to_vec(),
        ));
        let mut script = self.script.lock().unwrap_or_else(|e| e.into_inner());
        if script.is_empty() {
            return Ok("SCRIPT EXHAUSTED".to_string());
        }
        let text = script.remove(0); // FIFO — scripted order
        let mid = text.len() / 2;
        let (head, tail) = text.split_at(mid);
        on_delta(head);
        on_delta(tail);
        Ok(text.clone())
    }
}

/// Scripted backend that emits each chunk as its own delta — deterministic
/// delta boundaries (unlike [`FakeBackend`]'s midpoint split), so tests can
/// place blank lines inside specific deltas.
struct ChunkedBackend(Vec<String>);

impl ChatBackend for ChunkedBackend {
    fn stream_turn(
        &self,
        _model: &str,
        _system: &str,
        _messages: &[ChatMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        for chunk in &self.0 {
            on_delta(chunk);
        }
        Ok(self.0.concat())
    }
}

/// A sink that records stdout/stderr separately.
#[derive(Default)]
struct MemSink {
    out: String,
    err: Vec<String>,
}

impl Sink for MemSink {
    fn content(&mut self, piece: &str) {
        self.out.push_str(piece);
    }
    fn notice(&mut self, line: &str) {
        self.err.push(line.to_string());
    }
}

fn tmp_tool(
    body: &str,
    roots: Option<Vec<String>>,
) -> (tempfile::TempDir, smah_core::virtual_tools::loader::ToolDef) {
    let dir = tempfile::tempdir().expect("tempdir");
    let roots_yaml = roots
        .map(|list| {
            let items = list
                .iter()
                .map(|r| format!("  - \"{r}\"\n"))
                .collect::<String>();
            format!("allow_roots:\n{items}")
        })
        .unwrap_or_default();
    let file = write_tool(
        dir.path(),
        "fixture",
        &format!("---\nname: fixture\ndescription: driver fixture\n{roots_yaml}---\n{body}\n"),
    );
    let def = load_from(&file).expect("fixture tool loads");
    (dir, def)
}

fn no_audit() -> Audit {
    let dir = tempfile::tempdir().expect("audit tempdir");
    let path = dir.path().join("audit.jsonl");
    let audit = Audit::open_at(&path, "run");
    std::mem::forget(dir); // keep the tempdir alive for the Audit's lifetime
    audit
}

#[test]
fn driver_no_directive_single_turn_exit_ok() {
    let (tool_dir, def) = tmp_tool("Be helpful.", None);
    let _ = tool_dir;
    let (backend, seen) = FakeBackend::new(&["Just the answer.\n"]);
    let mut sink = MemSink::default();
    let outcome = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "anthropic.claude-opus-5-5",
        &["--info".to_string()],
        Some("piped input"),
        false,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-1",
        TURNS_MAX,
    )
    .expect("run ok");
    assert_eq!(outcome.turns, 1);
    assert!(!outcome.wrote_files);
    assert_eq!(sink.out, "Just the answer.\n");
    let requests = seen.lock().unwrap_or_else(|e| e.into_inner());
    let (model, system, messages) = &requests[0];
    assert_eq!(model, "anthropic.claude-opus-5-5");
    assert!(system.contains("RESERVED TOKEN"));
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, Role::User);
    assert!(messages[0].content.contains("--info"));
    assert!(messages[0].content.contains("piped input"));
}

#[test]
fn driver_multi_paragraph_answer_streams_byte_for_byte() {
    // Regression for the exit-3 coin flip: deltas carrying blank lines must
    // stream through untouched — same bytes out, no directive protocol
    // error, single clean turn.
    let (tool_dir, def) = tmp_tool("Be helpful.", None);
    let _ = tool_dir;
    let backend = ChunkedBackend(vec![
        "Para one.\n".to_string(),
        "\n".to_string(),
        "Para two.\n\n".to_string(),
        "\n".to_string(),
        "Para three.\n".to_string(),
    ]);
    let mut sink = MemSink::default();
    let outcome = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "anthropic.claude-opus-5-5",
        &[],
        None,
        false,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-paragraphs",
        TURNS_MAX,
    )
    .expect("run ok");
    assert_eq!(outcome.turns, 1);
    assert_eq!(sink.out, "Para one.\n\nPara two.\n\n\nPara three.\n");
    assert!(sink.err.is_empty(), "no stderr notices: {:?}", sink.err);
}

#[test]
fn driver_write_only_turn_executes_and_finishes() {
    let tool_dir = tempfile::tempdir().expect("roots");
    let (host_dir, def) = tmp_tool("Saver.", Some(vec![tool_dir.path().display().to_string()]));
    let _ = host_dir;
    let response =
        "Saving.\n@@smah write {\"path\": \"out.md\", \"content\": \"saved body\"}\nDone.\n";
    let (backend, _) = FakeBackend::new(&[response]);
    let mut sink = MemSink::default();
    let outcome = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::Enabled(vec![tool_dir.path().to_path_buf()]),
        &no_audit(),
        "run-2",
        TURNS_MAX,
    )
    .expect("run ok");
    assert_eq!(outcome.turns, 1);
    assert!(outcome.wrote_files);
    assert_eq!(sink.out, "Saving.\nDone.\n");
    assert_eq!(
        fs::read_to_string(tool_dir.path().join("out.md")).expect("written"),
        "saved body"
    );
    assert!(sink
        .err
        .iter()
        .any(|line| line.contains("wrote") && line.contains("out.md")));
}

#[test]
fn driver_read_loops_with_results_then_reply() {
    let tool_dir = tempfile::tempdir().expect("roots");
    fs::write(tool_dir.path().join("data.txt"), "42").expect("data");
    let (host_dir, def) = tmp_tool("Reader.", Some(vec![tool_dir.path().display().to_string()]));
    let _ = host_dir;
    let (backend, seen) = FakeBackend::new(&[
        "@@smah read {\"path\": \"data.txt\"}\n",
        "The answer is 42.\n",
    ]);
    let mut sink = MemSink::default();
    let outcome = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::Enabled(vec![tool_dir.path().to_path_buf()]),
        &no_audit(),
        "run-3",
        TURNS_MAX,
    )
    .expect("run ok");
    assert_eq!(outcome.turns, 2);
    assert_eq!(sink.out, "The answer is 42.\n");
    let requests = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(requests.len(), 2);
    let (_, _, second) = &requests[1];
    assert_eq!(second.len(), 3); // invocation, assistant (stripped), results
    assert_eq!(second[1].role, Role::Assistant);
    assert!(!second[1].content.contains("@@smah"), "directives stripped");
    assert!(second[2].content.contains("[FILE"));
    assert!(second[2].content.contains("42"));
}

#[test]
fn driver_violation_aborts_mid_list_later_directives_dropped() {
    let tool_dir = tempfile::tempdir().expect("roots");
    let (host_dir, def) = tmp_tool("Greedy.", Some(vec![tool_dir.path().display().to_string()]));
    let _ = host_dir;
    let (backend, _) = FakeBackend::new(&[
        "@@smah write {\"path\": \"ok.md\", \"content\": \"kept\"}\n@@smah write {\"path\": \"/etc/passwd\", \"content\": \"pwned\"}\n@@smah write {\"path\": \"never.md\", \"content\": \"no\"}\n",
    ]);
    let mut sink = MemSink::default();
    let err = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::Enabled(vec![tool_dir.path().to_path_buf()]),
        &no_audit(),
        "run-4",
        TURNS_MAX,
    )
    .expect_err("violation aborts");
    assert!(matches!(err, SmahError::DirectiveViolation { .. }));
    assert_eq!(err.exit_code().code(), 3);
    // Prior directive in the same turn stays applied (documented
    // non-atomicity); the later one never ran.
    assert_eq!(
        fs::read_to_string(tool_dir.path().join("ok.md")).expect("kept"),
        "kept"
    );
    assert!(!tool_dir.path().join("never.md").exists());
}

#[test]
fn driver_turn_overflow_is_provider_class() {
    let tool_dir = tempfile::tempdir().expect("roots");
    let (host_dir, def) = tmp_tool("Looper.", Some(vec![tool_dir.path().display().to_string()]));
    let _ = host_dir;
    // Each turn asks for a read of a missing file — the soft error keeps the
    // loop going, so the run hits the turn limit (read → results → re-query).
    let responses: Vec<String> = (0..6)
        .map(|i| format!("@@smah read {{\"path\": \"missing{i}.md\"}}\n"))
        .collect();
    let refs: Vec<&str> = responses.iter().map(String::as_str).collect();
    let (backend, _) = FakeBackend::new(&refs);
    let mut sink = MemSink::default();
    let err = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::Enabled(vec![tool_dir.path().to_path_buf()]),
        &no_audit(),
        "run-5",
        5,
    )
    .expect_err("overflow");
    assert!(matches!(
        err,
        SmahError::DirectiveLoopOverflow { turns: 5, .. }
    ));
    assert_eq!(err.exit_code().code(), 5);
}

#[test]
fn driver_sentinel_lines_never_reach_stdout() {
    let tool_dir = tempfile::tempdir().expect("roots");
    let (host_dir, def) = tmp_tool("Writer.", Some(vec![tool_dir.path().display().to_string()]));
    let _ = host_dir;
    let (backend, _) = FakeBackend::new(&[
        "before\n@@smah write {\"path\": \"x.md\", \"content\": \"v\"}\nafter\n",
    ]);
    let mut sink = MemSink::default();
    run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::Enabled(vec![tool_dir.path().to_path_buf()]),
        &no_audit(),
        "run-6",
        TURNS_MAX,
    )
    .expect("ok");
    assert!(!sink.out.contains("@@smah"), "stdout: {:?}", sink.out);
    assert_eq!(sink.out, "before\nafter\n");
}

#[test]
fn driver_no_fs_flag_converts_directives_to_soft_results() {
    let tool_dir = tempfile::tempdir().expect("roots");
    let (host_dir, def) = tmp_tool(
        "Blocked.",
        Some(vec![tool_dir.path().display().to_string()]),
    );
    let _ = host_dir;
    let (backend, _) = FakeBackend::new(&[
        "@@smah write {\"path\": \"x.md\", \"content\": \"v\"}\nFine, here is the summary.\n",
    ]);
    let mut sink = MemSink::default();
    let outcome = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::DisabledByFlag,
        &no_audit(),
        "run-7",
        TURNS_MAX,
    )
    .expect("run ok — disabled surface is soft");
    assert_eq!(outcome.turns, 2);
    assert!(!outcome.wrote_files);
    assert!(!tool_dir.path().join("x.md").exists());
}

#[test]
fn driver_none_declared_directive_is_violation() {
    let (host_dir, def) = tmp_tool("Nofiles.", None);
    let _ = host_dir;
    let (backend, _) = FakeBackend::new(&["@@smah read {\"path\": \"x.md\"}\n"]);
    let mut sink = MemSink::default();
    let err = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-8",
        TURNS_MAX,
    )
    .expect_err("violation");
    assert!(matches!(err, SmahError::DirectiveViolation { .. }));
}

#[test]
fn driver_protocol_abuse_is_directive_protocol_error() {
    let (host_dir, def) = tmp_tool("Sloppy.", None);
    let _ = host_dir;
    let (backend, _) = FakeBackend::new(&["@@smah nudge\n"]);
    let mut sink = MemSink::default();
    let err = run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-9",
        TURNS_MAX,
    )
    .expect_err("protocol");
    assert!(matches!(err, SmahError::DirectiveProtocol { .. }));
    assert_eq!(err.exit_code().code(), 3);
}

#[test]
fn driver_streams_deltas_even_when_backend_then_fails() {
    // First-byte discipline: content must reach the sink before the turn
    // errors out — the driver never buffers the whole answer.
    struct FailsAfterDelta;
    impl ChatBackend for FailsAfterDelta {
        fn stream_turn(
            &self,
            _model: &str,
            _system: &str,
            _messages: &[ChatMessage],
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, SmahError> {
            on_delta("first bytes ");
            Err(SmahError::ProviderUnavailable {
                provider: "anthropic".to_string(),
                code: 503,
            })
        }
    }
    let (host_dir, def) = tmp_tool("S.", None);
    let _ = host_dir;
    let mut sink = MemSink::default();
    let err = run_with_turns(
        &def,
        &FailsAfterDelta,
        &mut sink,
        "m",
        &[],
        None,
        false,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-10",
        TURNS_MAX,
    )
    .expect_err("backend error");
    assert!(matches!(err, SmahError::ProviderUnavailable { .. }));
    assert_eq!(
        sink.out, "first bytes ",
        "deltas reached stdout before the error"
    );
}

#[test]
fn driver_truncated_stdin_is_annotated_to_the_model() {
    let (host_dir, def) = tmp_tool("T.", None);
    let _ = host_dir;
    let (backend, seen) = FakeBackend::new(&["ok\n"]);
    let mut sink = MemSink::default();
    run_with_turns(
        &def,
        &backend,
        &mut sink,
        "m",
        &[],
        Some("partial data"),
        true,
        &FsMode::NoneDeclared,
        &no_audit(),
        "run-11",
        TURNS_MAX,
    )
    .expect("ok");
    let requests = seen.lock().unwrap_or_else(|e| e.into_inner());
    assert!(requests[0].2[0].content.contains("TRUNCATED"));
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

#[test]
fn audit_records_are_jsonl_metadata_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("state/audit.jsonl");
    let audit = Audit::open_at(&path, "run-42");
    audit
        .record(&AuditRecord {
            ts: "2026-10-18T12:00:00.000-07:00".to_string(),
            run: "run-42".to_string(),
            tool: "log-analyzer".to_string(),
            op: "write".to_string(),
            path: "~/notes/x.md".to_string(),
            resolved: Some("/Users/x/notes/x.md".to_string()),
            bytes: Some(2048),
            decision: "allowed",
            reason: String::new(),
            turn: 2,
        })
        .expect("record");
    let line = fs::read_to_string(&path).expect("log");
    let value: serde_json::Value = serde_json::from_str(line.trim()).expect("json line");
    assert_eq!(value["tool"], "log-analyzer");
    assert_eq!(value["decision"], "allowed");
    assert_eq!(value["run"], "run-42");
    assert_eq!(value["bytes"], 2048);
    let serialized = line.trim();
    assert!(!serialized.contains("saved body"), "contents never logged");
}
