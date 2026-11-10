//! Approval integrity (C1): an `approved:` stamp **binds to content**, not
//! to a timestamp. The stamp is a mapping
//!
//! ```yaml
//! approved:
//!   at: 2026-11-10T08:00:00-08:00   # when the review happened (RFC 3339)
//!   sha256: <64 hex>                # digest of the executable sections
//! ```
//!
//! where `sha256` digests the **canonicalized executable parts** of the
//! wrapper — the `params`, `steps`, and `outputs` sections rendered in the
//! file writer's stable YAML form (the exact bytes
//! [`canonical_executable_yaml`] produces). Editing anything executable
//! after approval invalidates the stamp; reordering `with:` keys or
//! rewriting comments does not survive canonicalization, and non-executable
//! metadata (`description`, `model`, `behavior_prompt`, `preconditions`,
//! the body) is not bound — it never reaches `smah-run`'s execution path.
//!
//! The legacy bare-timestamp format (`approved: 2026-…`) is **rejected** by
//! the loader: a time alone proves nothing about what was reviewed.
//!
//! Verification lives at execution time (`smah-run --execute`), never at
//! load time — a stamp that no longer matches must still LOAD, so
//! `smah-run approve <wrapper>` can re-approve the edited file.

use crate::wrappers::loader::WrapperDef;
use crate::wrappers::prompt::{outputs_value, params_value, sha256_hex, steps_value};

/// One approval stamp (`approved: {at, sha256}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalStamp {
    /// When the review happened (RFC 3339, local time).
    pub at: String,
    /// sha256 of [`canonical_executable_yaml`] at approval time.
    pub sha256: String,
}

/// Why a wrapper fails approval verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampProblem {
    /// No `approved:` field at all.
    Missing,
    /// A stamp is present but not a well-formed `{at, sha256}` pair.
    Malformed(String),
    /// The stamp is well-formed but the content hash differs — the
    /// executable sections changed after approval.
    Mismatch { stored: String, actual: String },
}

impl StampProblem {
    /// One-line description for `smah-run`'s refusal message.
    pub fn describe(&self) -> String {
        match self {
            StampProblem::Missing => "no approval stamp: the wrapper carries no \
                'approved:' stamp. Review the plan ('smah --wrapper <name>') \
                and stamp it with 'smah-run approve <name>'."
                .to_string(),
            StampProblem::Malformed(detail) => format!(
                "malformed approval stamp ({detail}) — re-approve with \
                'smah-run approve <name>'."
            ),
            StampProblem::Mismatch { stored, actual } => format!(
                "content changed since approval: the stamp binds sha256 \
                {} but the current executable sections hash to {} — \
                re-review and re-approve with 'smah-run approve <name>'.",
                &stored[..stored.len().min(12)],
                &actual[..actual.len().min(12)],
            ),
        }
    }
}

/// The canonicalized executable content the approval hash binds: the
/// `params` (when declared), `steps`, and `outputs` (when declared)
/// sections in the file writer's stable YAML form.
pub fn canonical_executable_yaml(wrapper: &WrapperDef) -> String {
    let mut map = serde_yaml_ng::Mapping::new();
    if let Some(p) = params_value(&wrapper.params) {
        map.insert(serde_yaml_ng::Value::String("params".into()), p);
    }
    map.insert(
        serde_yaml_ng::Value::String("steps".into()),
        steps_value(&wrapper.steps),
    );
    if let Some(o) = outputs_value(&wrapper.outputs) {
        map.insert(serde_yaml_ng::Value::String("outputs".into()), o);
    }
    serde_yaml_ng::to_string(&serde_yaml_ng::Value::Mapping(map)).unwrap_or_default()
}

/// The sha256 hex digest of the canonical executable content.
pub fn content_sha256(wrapper: &WrapperDef) -> String {
    sha256_hex(canonical_executable_yaml(wrapper).as_bytes())
}

/// Build a stamp for `wrapper` as of `at_rfc3339` (the approve flow).
pub fn stamp_for(wrapper: &WrapperDef, at_rfc3339: &str) -> ApprovalStamp {
    ApprovalStamp {
        at: at_rfc3339.to_string(),
        sha256: content_sha256(wrapper),
    }
}

/// Shape check: `at` parses as RFC 3339 and `sha256` is 64 lowercase hex.
pub fn is_well_formed(stamp: &ApprovalStamp) -> Result<(), String> {
    if chrono::DateTime::parse_from_rfc3339(&stamp.at).is_err() {
        return Err(format!(
            "'at' must be an RFC 3339 timestamp, got '{}'",
            stamp.at
        ));
    }
    let hex = &stamp.sha256;
    if hex.len() != 64 || !hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(format!(
            "'sha256' must be 64 lowercase hex characters, got '{}'",
            &hex[..hex.len().min(16)]
        ));
    }
    Ok(())
}

/// Full verification: present + well-formed + hash matches current content.
/// This is the `smah-run --execute` gate — nothing here runs on failure.
pub fn verify(wrapper: &WrapperDef) -> Result<(), StampProblem> {
    let Some(stamp) = &wrapper.approved else {
        return Err(StampProblem::Missing);
    };
    if let Err(detail) = is_well_formed(stamp) {
        return Err(StampProblem::Malformed(detail));
    }
    let actual = content_sha256(wrapper);
    if actual != stamp.sha256 {
        return Err(StampProblem::Mismatch {
            stored: stamp.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrappers::loader::parse_wrapper;
    use std::collections::HashSet;

    fn def_of(raw: &str) -> WrapperDef {
        parse_wrapper(raw, "demo", &HashSet::new()).expect("loads")
    }

    const WRAPPER: &str = "---\n\
name: demo\ndescription: demo recipe\n\
params:\n  - name: src\n    required: true\n\
steps:\n  - id: s1\n    run: [\"echo\", \"{{params.src}}\"]\n    output: out\n\
outputs:\n  - name: out\n---\nbody\n";

    #[test]
    fn canonical_form_is_stable_across_formatting() {
        // Same executable content, wildly different file formatting:
        // comments, key order, quoting — one canonical hash.
        let a = def_of(WRAPPER);
        let reformatted = "---\n\
# a comment\nsteps:\n  - output: out\n    run:\n      - \"echo\"\n      - '{{params.src}}'\n    id: s1\noutputs:\n  - name: out\n\
params:\n  - name: src\n    required: true\n\
name: demo\ndescription: demo recipe\n---\nother body\n";
        let b = def_of(reformatted);
        assert_eq!(content_sha256(&a), content_sha256(&b));
    }

    #[test]
    fn any_executable_edit_changes_the_hash() {
        let a = def_of(WRAPPER);
        let edited = WRAPPER.replace("\"echo\"", "\"echo\", \"x\"");
        assert_ne!(content_sha256(&a), content_sha256(&def_of(&edited)));
        // A param default flip counts as executable change too.
        let with_default = WRAPPER.replace(
            "- name: src\n    required: true",
            "- name: src\n    required: true\n  - name: label\n    default: hi",
        );
        assert_ne!(content_sha256(&a), content_sha256(&def_of(&with_default)));
    }

    #[test]
    fn metadata_edits_do_not_change_the_hash() {
        let a = def_of(WRAPPER);
        let edited = WRAPPER
            .replace("description: demo recipe", "description: rewritten")
            .replace("body\n", "new body\n");
        assert_eq!(content_sha256(&a), content_sha256(&def_of(&edited)));
    }

    #[test]
    fn verify_round_trip_and_problems() {
        let w = def_of(WRAPPER);
        let mut unapproved = w.clone();
        unapproved.approved = None;
        assert_eq!(verify(&unapproved), Err(StampProblem::Missing));

        let mut malformed = w.clone();
        malformed.approved = Some(ApprovalStamp {
            at: "yesterday".into(),
            sha256: content_sha256(&w),
        });
        assert!(matches!(
            verify(&malformed),
            Err(StampProblem::Malformed(_))
        ));

        let mut forged = w.clone();
        forged.approved = Some(ApprovalStamp {
            at: "2026-11-10T08:00:00-08:00".into(),
            sha256: "0".repeat(64),
        });
        match verify(&forged) {
            Err(StampProblem::Mismatch { actual, .. }) => {
                assert_eq!(actual, content_sha256(&w));
            }
            other => panic!("expected mismatch, got {other:?}"),
        }

        let mut good = w.clone();
        good.approved = Some(stamp_for(&w, "2026-11-10T08:00:00-08:00"));
        assert_eq!(verify(&good), Ok(()));
    }

    #[test]
    fn stamped_content_round_trips_through_the_loader() {
        // A stamped wrapper renders, re-parses, and still verifies — the
        // approve flow depends on this byte path.
        let w = def_of(WRAPPER);
        let mut stamped = w.clone();
        stamped.approved = Some(stamp_for(&w, "2026-11-10T08:00:00-08:00"));
        let content = crate::wrappers::prompt::render_wrapper_file(&stamped);
        assert!(content.contains("approved:"), "{content}");
        assert!(content.contains("sha256:"));
        let reparsed = parse_wrapper(&content, "demo", &HashSet::new()).expect("reparses");
        assert_eq!(verify(&reparsed), Ok(()));
    }

    #[test]
    fn problem_messages_name_the_remedy() {
        assert!(StampProblem::Missing
            .describe()
            .contains("smah-run approve"));
        assert!(StampProblem::Malformed("bad".into())
            .describe()
            .contains("re-approve"));
        let m = StampProblem::Mismatch {
            stored: "a".repeat(64),
            actual: "b".repeat(64),
        }
        .describe();
        assert!(m.contains("content changed since approval"));
        assert!(m.contains("aaaaaaaaaaaa"));
    }
}
