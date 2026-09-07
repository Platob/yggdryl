"""The staged ``ygg`` comes from cargo's answer, not from a reconstructed path.

``target/<profile>/`` holds the binary only when nothing moves it, and the
release matrix moves it three ways: ``--target`` on a cross build,
``CARGO_TARGET_DIR`` in an environment, and ``CARGO_BUILD_TARGET`` set by an
image. The musllinux container sets the last, so the 0.1.2 release built the
command successfully and then refused its own wheel::

    Finished `release` profile [optimized] target(s) in 4m 02s
    cargo built nothing at /home/runner/work/yggdryl/yggdryl/target/release/ygg

Nothing on a plain runner sets any of the three, so no check that builds the
way a contributor does can see it. These read cargo's artifact stream directly
instead, which is what ``scripts/stage_cli.py`` now does.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import types

import pytest

SCRIPT = (
    pathlib.Path(__file__).resolve().parent.parent.parent / "scripts" / "stage_cli.py"
)
# A source distribution carries `python/tests/` but prunes `scripts/`, and it
# is right to: staging is what builds a wheel here, not what an installer
# runs. These tests are about that tooling, so where the tooling is absent
# they have nothing to say - and saying it at import time is what keeps them
# from failing the whole suite on collection.
if not SCRIPT.exists():  # pragma: no cover - only outside a checkout
    pytest.skip(f"{SCRIPT.name} is not shipped here", allow_module_level=True)


def _stage_cli() -> types.ModuleType:
    """`scripts/` is repository tooling rather than a package, so load by path."""
    specification = importlib.util.spec_from_file_location("stage_cli", SCRIPT)
    assert specification is not None and specification.loader is not None
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


stage_cli = _stage_cli()


def artifact(executable: str | None, name: str = "ygg", kind: str = "bin") -> str:
    """One `compiler-artifact` line, shaped the way cargo writes it."""
    return json.dumps(
        {
            "reason": "compiler-artifact",
            "package_id": "path+file:///yggdryl/cli#yggdryl-cli@0.1.2",
            "target": {"kind": [kind], "crate_types": [kind], "name": name},
            "profile": {"opt_level": "3", "debuginfo": 0, "test": False},
            "filenames": [executable] if executable else [],
            "executable": executable,
            "fresh": False,
        }
    )


def test_a_moved_target_directory_is_still_found() -> None:
    # The musllinux failure exactly: the build succeeded, under a triple.
    linked = stage_cli.linked(
        "\n".join(
            [
                artifact(None, name="yggdryl", kind="lib"),
                artifact("/work/target/x86_64-unknown-linux-musl/release/ygg"),
            ]
        )
    )
    assert linked == pathlib.Path("/work/target/x86_64-unknown-linux-musl/release/ygg")


def test_an_unmoved_target_directory_is_found_the_same_way() -> None:
    linked = stage_cli.linked(artifact("/work/target/release/ygg"))
    assert linked == pathlib.Path("/work/target/release/ygg")


def test_the_windows_binary_is_recognised_by_its_target_name() -> None:
    # The file gains `.exe`; the bin target cargo reports is `ygg` regardless,
    # which is why the artifact is matched on the target rather than the file.
    # `str` rather than `.name`, because a Windows path read on a POSIX host is
    # one long segment - the point here is that it was recognised at all.
    linked = stage_cli.linked(artifact("C:\\work\\target\\release\\ygg.exe"))
    assert linked is not None
    assert str(linked).endswith("ygg.exe")


def test_another_crate_s_binary_is_not_staged() -> None:
    # A workspace builds more than one bin; only this one belongs in the wheel.
    other = artifact("/work/target/release/other", name="other")
    assert stage_cli.linked(other) is None


def test_a_library_artifact_carries_no_command() -> None:
    assert stage_cli.linked(artifact(None, name="ygg", kind="lib")) is None


def test_a_build_that_linked_nothing_reports_nothing() -> None:
    # `linked` returning None is what makes the script exit non-zero rather
    # than stage a stale binary a previous run left behind.
    assert stage_cli.linked("") is None


def test_output_that_is_not_cargo_s_messages_is_stepped_over() -> None:
    linked = stage_cli.linked(
        "\n".join(
            [
                "warning: something a wrapper printed on stdout",
                "",
                artifact("/work/target/release/ygg"),
                "{not json",
            ]
        )
    )
    assert linked == pathlib.Path("/work/target/release/ygg")


def test_the_last_link_of_the_command_wins() -> None:
    # A rebuild inside one invocation reports the artifact more than once.
    linked = stage_cli.linked(
        "\n".join(
            [
                artifact("/work/target/debug/ygg"),
                artifact("/work/target/release/ygg"),
            ]
        )
    )
    assert linked == pathlib.Path("/work/target/release/ygg")


@pytest.mark.parametrize("flag", ["--message-format", "json-render-diagnostics"])
def test_the_script_asks_cargo_to_name_its_artifacts(flag: str) -> None:
    # The parsing above is only reached if the command actually requests it,
    # and `json-render-diagnostics` is what keeps a failing build readable:
    # the JSON goes to stdout while errors still render as text on stderr.
    assert flag in SCRIPT.read_text(encoding="utf-8")


def test_the_staged_name_carries_the_extension_windows_needs() -> None:
    expected = "ygg.exe" if sys.platform == "win32" else "ygg"
    assert stage_cli.NAME == expected
