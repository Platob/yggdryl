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
# What the manifest calls the bin target, which is the name cargo reports it by
# on every platform - `NAME` is only what the file is called once it is written.
BIN = "ygg"


def linked(output: str) -> pathlib.Path | None:
    """The binary cargo says it linked, read out of its own artifact stream.

    `target/<profile>/` is where a binary lands only when nothing moves it, and
    the release matrix moves it three ways: `--target` on a cross build,
    `CARGO_TARGET_DIR` in an environment, and `CARGO_BUILD_TARGET` set by an
    image. The last is what the musllinux container does, so a build that had
    already succeeded left `target/<triple>/release/ygg` while a path
    reconstructed from the profile alone looked in `target/release/` and called
    the build empty:

        Finished `release` profile [optimized] target(s) in 4m 02s
        cargo built nothing at /home/runner/work/yggdryl/yggdryl/target/release/ygg

    So cargo names the file rather than the path being guessed from the pieces:
    under `--message-format json-render-diagnostics` every linked artifact is a
    line of JSON on stdout carrying the `executable` it produced, while
    diagnostics still render as text on stderr and read as they always did. A
    build that was already up to date reports its artifacts the same way, so a
    second run stages the same binary rather than finding none.
    """
    executable = None
    for line in output.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            # Anything on stdout that is not one of cargo's messages - a
            # wrapper's own chatter, or the blank line at the end.
            continue
        if not isinstance(message, dict):
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target") or {}
        if target.get("name") != BIN or "bin" not in (target.get("kind") or ()):
            continue
        if message.get("executable"):
            executable = pathlib.Path(message["executable"])
    return executable


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
    command += ["--message-format", "json-render-diagnostics"]
    print(" ".join(command))
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=False,
        env=environment(),
        stdout=subprocess.PIPE,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode != 0:
        return result.returncode

    source = linked(result.stdout)
    if source is None:
        print(f"cargo linked no {BIN} binary", file=sys.stderr)
        return 1
    if not source.exists():
        print(f"cargo named {source}, which is not there", file=sys.stderr)
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
