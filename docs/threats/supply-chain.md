# Supply Chain

Details for [../THREAT-MODEL.md](../THREAT-MODEL.md).

## Dependencies

Poetry-managed (`pyproject.toml` + `poetry.lock`): `rich`, `openai`,
`anthropic`, `pyyaml`, `psutil`, `lxml`, `cssselect`; dev `pytest`; docs
group (sphinx/myst/recommonmark/rtd-theme/mermaid). `pyperclip` is
deliberately **optional** (not a dependency; silent degrade). No vendored
code. Lockfile is the tamper control; no automated audit step exists (a
`pip-audit`/`poetry audit` CI step would close that cheaply — not tracked
as a register item until adopted).

## CI / Release Path (B5)

- **PyPI publishing**: Trusted Publishing via OIDC — `environment: pypi`,
  `permissions: id-token: write`, no API-token secret anywhere; a
  tag↔`pyproject` version-match gate precedes the build. Long-lived
  credential theft is structurally excluded.
- **T-007 (Low, Open)**: actions pinned by **moving tags**, not SHAs —
  `actions/checkout@v4`, `actions/setup-python@v5` (both workflows),
  `pypa/gh-action-pypi-publish@release/v1`. A tag move on any of these
  runs attacker code in the release path with `id-token: write`.
  **Fix shape**: SHA-pin all three (dependabot can maintain SHAs).
- **Workflow hygiene (clean)**: no `pull_request_target`, no `secrets.*`
  context, no `github.event.*` interpolated into `run:`; the only
  `${{ }}` in scripts is the trusted `matrix.python-version`.
- **Print-only grep gate**: CI asserts no shell-execution paths
  (`ci.yml` "Assert no shell-execution paths"), same allowlist as the
  pytest scan — supply chain can't silently introduce an execution path
  without failing CI.

## Distribution

Consumers install from PyPI (`smah`); no post-install scripts declared.
ReadTheDocs builds docs only. No container/OCI artifacts exist.
