#!/usr/bin/env python3
"""Install the native build dependencies the fast (`gzip-zlib-ng`) bindings need.

The bindings build zlib-ng through CMake, which needs **CMake** and a build
generator (**Ninja** here). This script installs any that are missing — via `pip`
by default (the `cmake` / `ninja` PyPI packages ship real binaries), so it works in
a virtualenv with no admin rights — and reports the environment to use.

Usage:
    python scripts/setup-build-deps.py          # install what's missing
    python scripts/setup-build-deps.py --check   # report only, exit 1 if missing

A C compiler is still required (MSVC on Windows, cc/clang elsewhere) and is not
installed here; the script only checks for it.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys


TOOLS = {
    "cmake": "cmake",  # command -> pip package
    "ninja": "ninja",
}


def have(tool: str) -> bool:
    return shutil.which(tool) is not None


def pip_install(packages: list[str]) -> None:
    # `uv` is the project's Python toolchain (CLAUDE.md); fall back to pip if absent.
    if shutil.which("uv"):
        subprocess.check_call(["uv", "pip", "install", "--python", sys.executable, *packages])
    else:
        subprocess.check_call([sys.executable, "-m", "pip", "install", *packages])


def c_compiler_present() -> bool:
    if sys.platform == "win32":
        # cargo/cc locate MSVC via the registry even when cl.exe is off PATH, so a
        # plain PATH probe is only a hint; treat a VS install dir as good enough.
        import os

        roots = [
            os.environ.get("ProgramFiles(x86)", ""),
            os.environ.get("ProgramFiles", ""),
        ]
        for root in roots:
            if root and os.path.isdir(os.path.join(root, "Microsoft Visual Studio")):
                return True
        return have("cl") or have("clang")
    return have("cc") or have("clang") or have("gcc")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="report only; do not install")
    args = parser.parse_args()

    missing = [pkg for tool, pkg in TOOLS.items() if not have(tool)]

    if args.check:
        for tool in TOOLS:
            print(f"{tool:8} {'ok' if have(tool) else 'MISSING'}")
        print(f"{'c-compiler':8} {'ok' if c_compiler_present() else 'MISSING'}")
        return 1 if missing else 0

    if missing:
        print(f"Installing missing build deps via pip: {', '.join(missing)}")
        pip_install(missing)
    else:
        print("CMake and Ninja already available.")

    if not c_compiler_present():
        print(
            "WARNING: no C compiler found. Install MSVC (Windows) / cc or clang "
            "(Unix) before building the zlib-ng bindings.",
            file=sys.stderr,
        )

    print("\nBuild the fast bindings with Ninja pinned (this repo sets it in")
    print(".cargo/config.toml already):  CMAKE_GENERATOR=Ninja")
    print("  (cd bindings/python && uv run maturin develop --release)")
    print("  (cd bindings/node && npm run build)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
