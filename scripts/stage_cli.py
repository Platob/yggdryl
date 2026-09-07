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
import json
import os
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


def built(messages: str) -> pathlib.Path | None:
    """Where cargo says it put the binary, rather than where we guessed.

    A build inside the musllinux image targets a triple of its own, so cargo
    writes to ``target/<triple>/<profile>`` and a path composed here from the
    profile alone names a file that was never written - staging then fails
    after a build that worked, which is what the 0.1.2 release did on both
    musl wheels while every manylinux one passed. ``CARGO_TARGET_DIR`` moves
    the same path again. The JSON stream carries what cargo actually wrote,
    so nothing here has to know which of those applied.
    """
    for line in messages.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        produced = message.get("executable")
        if produced and pathlib.Path(produced).name == NAME:
            return pathlib.Path(produced)
    return None


def environment() -> dict[str, str]:
    """The environment cargo builds under, minus a wrapper that is not installed.

    `RUSTC_WRAPPER` names a program cargo runs in place of `rustc`, and the
    release matrix sets it to sccache. The Linux wheels build inside the
    manylinux and musllinux images, where the variable crosses into the
    container but the program does not, so cargo refuses before it compiles
    anything:

        error: could not execute process `sccache .../rustc -vV`
        Caused by: No such file or directory (os error 2)

    A wrapper that cannot be resolved is dropped rather than obeyed. Where the
    wrapper is real the cache is still used; where it is only a name, the build
    is slower and not broken.
    """
    resolved = dict(os.environ)
    for variable in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
        wrapper = resolved.get(variable)
        if wrapper and shutil.which(wrapper) is None:
            print(f"{variable}={wrapper} is not executable here; building without it")
            del resolved[variable]
    return resolved


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
    # The JSON stream names the binary; diagnostics still render to stderr, so
    # a failing build reads the way it always did.
    command += ["--message-format", "json-render-diagnostics"]
    print(" ".join(command))
    result = subprocess.run(
        command, cwd=ROOT, check=False, env=environment(), stdout=subprocess.PIPE, text=True
    )
    if result.returncode != 0:
        return result.returncode

    source = built(result.stdout)
    if source is None or not source.exists():
        print("cargo reported no ygg binary to stage", file=sys.stderr)
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
