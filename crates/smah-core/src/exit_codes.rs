//! Process exit codes for the SMAH CLI (interface-spec §4).
//!
//! The table in `docs/ux/interface-spec.md` §4 fixes six classes. The spec
//! table's "internal error → 5" row is an erratum: the code and the spec
//! footer agree the internal class exits **1** (plan correction 1), and this
//! enum encodes the corrected mapping.

/// Exit code classes for every SMAH termination path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExitCode {
    /// Success (0).
    Ok,
    /// Unexpected internal error (1) — spec §4 erratum, not the table's 5.
    Internal,
    /// Usage error: unknown/removed flag, missing query (2).
    Usage,
    /// Configuration unreadable, invalid, or gated (3). Also session-db problems.
    Config,
    /// Authentication: missing key, zero enabled models, provider 401/403 (4).
    Auth,
    /// Provider failure: 4xx non-auth, 5xx, network/timeout (5).
    Provider,
}

impl ExitCode {
    /// Numeric process exit code.
    pub fn code(self) -> i32 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::Internal => 1,
            ExitCode::Usage => 2,
            ExitCode::Config => 3,
            ExitCode::Auth => 4,
            ExitCode::Provider => 5,
        }
    }
}

// Note: no conversion to the standard library's process-exit type is
// provided here — the numeric mapping happens at the binary edge (smah-cli)
// to keep this crate clear of execution primitives. The no-shell CI gate
// greps this tree for such paths, and the print-only invariant wants zero of
// them in crates/*/src.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_spec_table() {
        assert_eq!(ExitCode::Ok.code(), 0);
        assert_eq!(ExitCode::Internal.code(), 1);
        assert_eq!(ExitCode::Usage.code(), 2);
        assert_eq!(ExitCode::Config.code(), 3);
        assert_eq!(ExitCode::Auth.code(), 4);
        assert_eq!(ExitCode::Provider.code(), 5);
    }
}
