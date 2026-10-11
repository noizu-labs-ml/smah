import pathlib

import smah

# Files allowed to touch subprocess: the OS-info readers use subprocess.check_output
# to inspect the operating system (e.g. sw_vers, lsb_release). They never execute
# model-suggested commands.
ALLOWED_SUBPROCESS_FILES = {
    "smah/settings/system/operating_system/info/base_info.py",
    "smah/settings/system/operating_system/info/bsd_info.py",
    "smah/settings/system/operating_system/info/darwin_info.py",
    "smah/settings/system/operating_system/info/linux_info.py",
    "smah/settings/system/operating_system/info/windows_info.py",
}


def test_no_shell_execution_in_source():
    """
    No code path may reach a shell-execution call: SMAH is print-only by design.
    Scans every module under smah/ for subprocess.run / Popen / os.system / shell=True.
    """
    root = pathlib.Path(smah.__file__).parent.parent
    offenders = []
    for py in sorted(root.glob("smah/**/*.py")):
        rel = py.relative_to(root).as_posix()
        if rel in ALLOWED_SUBPROCESS_FILES:
            continue
        source = py.read_text(encoding="utf-8")
        if (
            "subprocess.run" in source
            or "shell=True" in source
            or "os.system" in source
            or "Popen" in source
        ):
            offenders.append(rel)
    assert offenders == [], f"shell execution found in: {offenders}"


def test_runner_does_not_import_subprocess():
    root = pathlib.Path(smah.__file__).parent.parent
    runner_source = (root / "smah" / "runner" / "runner.py").read_text(encoding="utf-8")
    assert "import subprocess" not in runner_source
    assert "subprocess." not in runner_source
