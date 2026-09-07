"""Build the ``ygg`` command and stage it for the Python wheel.

The wheel carries the compiled CLI beside the extension module: maturin copies
``python/wheel-data/scripts/`` into ``yggdryl-<version>.data/scripts/``, which
an installer puts straight on PATH. maturin copies rather than builds, so this
runs first and it is the only thing that knows the binary's name differs by
platform.

Usage:
    python scripts/stage_cli.py                   # release, host target
    python scripts/stage_cli.py --debug           # a faster, much larger binary
    python scripts/stage_cli.py --target <triple> # cross build, for a release matrix
    python scripts/stage_cli.py --clear           # remove the staged binary again
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
STAGE = ROOT / "python" / "wheel-data" / "scripts"
# What cargo calls the binary, and what it is called once staged. Windows keeps
# the extension because an installer copies the name verbatim onto PATH, and a
# `ygg` with no `.exe` there is not executable.
NAME = "ygg.exe" if sys.platform == "win32" else "ygg"


def built(profile: str, target: str | None) -> pathlib.Path:
    """Where cargo leaves the binary for one profile and target."""
    directory = ROOT / "target"
    if target:
        directory = directory / target
    return directory / profile / NAME


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debug", action="store_true", help="stage a debug build")
    parser.add_argument("--target", help="the Rust target triple to build for")
    parser.add_argument(
        "--clear",
        action="store_true",
        help="remove the staged binary and build nothing",
    )
    arguments = parser.parse_args()

    STAGE.mkdir(parents=True, exist_ok=True)
    staged = STAGE / NAME

    if arguments.clear:
        staged.unlink(missing_ok=True)
        print(f"cleared {staged.relative_to(ROOT)}")
        return 0

    command = ["cargo", "build", "-p", "yggdryl-cli"]
    if not arguments.debug:
        command.append("--release")
    if arguments.target:
        command += ["--target", arguments.target]
    print(" ".join(command))
    result = subprocess.run(command, cwd=ROOT, check=False)
    if result.returncode != 0:
        return result.returncode

    source = built("debug" if arguments.debug else "release", arguments.target)
    if not source.exists():
        print(f"cargo built nothing at {source}", file=sys.stderr)
        return 1
    shutil.copy2(source, staged)
    # The executable bit is what makes the staged file runnable once an
    # installer copies it onto PATH; a copy from a checkout may not carry it.
    if sys.platform != "win32":
        staged.chmod(0o755)
    print(f"staged {source.stat().st_size} bytes at {staged.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
