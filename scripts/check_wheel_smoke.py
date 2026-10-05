#!/usr/bin/env python3
"""Exercise an installed wheel the way a reader's first program does.

The release builds a wheel per platform and per interpreter and publishes
them, so the last thing worth asking before a publish is whether the thing
that comes out of ``pip install`` actually works: the extension module loads,
a handle opens a folder, and a table takes rows and gives them back.

This ran as a heredoc inside ``release.yml`` until it broke a release. When
#134 moved ``yggdryl.media.iceberg`` to ``yggdryl.iceberg``, the import here
was left pointing at the old module - and nothing noticed, because the only
copy of this script lived in a workflow step that runs after `main` already
has the commit. Every lane that smokes a wheel failed the 0.1.9 run, PyPI
never received it, and the tag that records a finished release was never cut.

So it is a file, and both sides run it: the release, against each wheel it is
about to publish that a runner can load (``docs/testing.md`` names the ones it
cannot), and ``ci.yml``'s Python lanes, against the wheels those jobs build -
the stable-ABI one under both PyArrow versions, the free-threaded 3.14 one and
the abi3t one on 3.15 and 3.15t. A rename now breaks the pull request that
makes it, which is the only place the cost of fixing it is small.

It runs in one of two halves, and the caller says which - never the
environment, so a lane that owes the whole smoke fails on a missing or broken
PyArrow rather than passing on the lesser half:

- With no flag, the whole of it: ``import yggdryl``, an Iceberg table round
  trip, and the table served over XMLA by the command the wheel installed.
- Under ``--extension-only`` - CPython 3.15, for which PyArrow publishes no
  wheel on any platform - only the extension module: ``import yggdryl`` imports PyArrow
  (``python/yggdryl/extension.py``), so the installed ``yggdryl/_native*`` file
  is loaded on its own under the bare name it initializes as, a datatype is
  parsed through it, and the command is found beside the interpreter. It
  prints one ``SKIPPED`` line naming what did not run and why, because a
  half that ran is not the whole that passed.

On a free-threaded interpreter both halves assert the GIL is still off once
the extension is loaded: an extension that does not declare itself safe to
run without the GIL turns it back on at import, with nothing worse than a
warning, so this is the only place that would notice.

It imports ``yggdryl`` and never ``python/yggdryl``: run it from anywhere,
and what it reports on is what the interpreter resolves - the installed
distribution, not the source tree beside it.

Usage:
    python scripts/check_wheel_smoke.py                     # the whole smoke
    python scripts/check_wheel_smoke.py --extension-only    # the extension alone
"""

from __future__ import annotations

import argparse
import fnmatch
import importlib.metadata
import importlib.util
import os
import pathlib
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import urllib.request

XMLA = "urn:schemas-microsoft-com:xml-analysis"


def command() -> str:
    """The `yggdryl` command the wheel installed beside the interpreter, or on PATH."""
    beside = pathlib.Path(sys.executable).with_name("yggdryl")
    for candidate in (beside, beside.with_suffix(".exe")):
        if candidate.exists():
            return str(candidate)
    found = shutil.which("yggdryl")
    if found is None:
        raise SystemExit("the wheel installed no `yggdryl` beside its interpreter or on PATH")
    return found


def envelope(body: str) -> bytes:
    return (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/">'
        f"<SOAP-ENV:Body>{body}</SOAP-ENV:Body></SOAP-ENV:Envelope>"
    ).encode()


def post(endpoint: str, action: str, body: str) -> str:
    request = urllib.request.Request(
        endpoint,
        data=envelope(body),
        headers={"Content-Type": "text/xml", "SOAPAction": f'"{XMLA}:{action}"'},
    )
    with urllib.request.urlopen(request, timeout=30) as answer:
        return answer.read().decode()


def serve_over_xmla(warehouse: pathlib.Path) -> None:
    """`yggdryl xmla serve` over the folder: the Iceberg table is listed and read.

    The staged `yggdryl` is built with the `iceberg` feature; a binary without it
    lists the table and refuses its columns by name, which is what this
    catches before a wheel ships.
    """
    process = subprocess.Popen(
        [command(), "xmla", "serve", "--bind", "127.0.0.1:0", f"smoke={warehouse}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        encoding="utf-8",
        env={**os.environ, "NO_COLOR": "1"},
    )
    try:
        assert process.stdout is not None
        endpoint = process.stdout.readline().strip()
        assert endpoint.startswith("http://127.0.0.1:"), endpoint
        columns = post(
            endpoint,
            "Discover",
            f'<Discover xmlns="{XMLA}"><RequestType>DBSCHEMA_COLUMNS</RequestType>'
            "<Restrictions/><Properties/></Discover>",
        )
        assert "<COLUMN_NAME>id</COLUMN_NAME>" in columns, columns
        rows = post(
            endpoint,
            "Execute",
            f'<Execute xmlns="{XMLA}"><Command><Statement>select id from smoke.smoke order by id'
            "</Statement></Command><Properties/></Execute>",
        )
        assert rows.count("<row>") == 2 and "<id>2</id>" in rows, rows
    finally:
        process.kill()
        process.wait()


def interpreter_tag() -> str:
    """`cp314t`, `cp315`: the interpreter as a wheel tag names it."""
    threading = "t" if free_threaded() else ""
    return f"cp{sys.version_info.major}{sys.version_info.minor}{threading}"


def free_threaded() -> bool:
    return bool(sysconfig.get_config_var("Py_GIL_DISABLED") == 1)


def gil_stays_off() -> None:
    """On a free-threaded interpreter, the extension left the GIL disabled."""
    if free_threaded():
        assert not sys._is_gil_enabled(), (
            f"loading the extension enabled the GIL on {interpreter_tag()}"
        )
        print(f"the GIL is off on {interpreter_tag()}")


def extension_file() -> pathlib.Path:
    """The installed distribution's one `yggdryl/_native*` extension module.

    A wheel records it; an editable `maturin develop` install records nothing,
    so the package folder is found without importing it (which would import
    PyArrow) and its one extension taken from there.
    """
    files = importlib.metadata.files("yggdryl") or []
    found = [
        pathlib.Path(str(entry.locate()))
        for entry in files
        if fnmatch.fnmatch(entry.as_posix(), "yggdryl/_native*")
        and pathlib.PurePosixPath(entry.as_posix()).suffix in (".so", ".pyd")
    ]
    if not found:
        spec = importlib.util.find_spec("yggdryl")
        for folder in spec.submodule_search_locations or [] if spec is not None else []:
            found.extend(
                candidate
                for candidate in pathlib.Path(folder).glob("_native*")
                if candidate.suffix in (".so", ".pyd")
            )
    assert len(found) == 1, f"expected one yggdryl/_native extension, found {found}"
    return found[0]


def smoke_extension(reason: str) -> None:
    """Load the extension module alone, parse a datatype, find the command."""
    path = extension_file()
    print(f"yggdryl {importlib.metadata.version('yggdryl')} extension {path}")
    # The module initializes under the name it was built as, so it is loaded
    # bare rather than as `yggdryl._native`, whose package imports PyArrow.
    spec = importlib.util.spec_from_file_location("_native", path)
    assert spec is not None and spec.loader is not None, path
    native = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(native)
    gil_stays_off()
    assert hasattr(native, "DataType"), f"{path} exposes no DataType"
    parsed = native.DataType("int64")
    assert str(parsed) == "int64", str(parsed)
    print(f"the command is {command()}")
    print(f"SKIPPED: the Iceberg round trip and the XMLA serve - {reason}")
    print("wheel smoke: extension only")


def smoke_package() -> None:
    """Create a table, append a batch, read it back, serve it, and say where from."""
    import pyarrow as pa

    import yggdryl
    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergTable

    print(f"yggdryl {yggdryl.__version__} from {yggdryl.__file__}")
    gil_stays_off()

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-release-"))
    table = IcebergTable.create(IOBase(warehouse / "smoke"), columns)
    table.append(pa.record_batch({"id": [1, 2]}, schema=columns))
    assert table.scan().read_all().column("id").to_pylist() == [1, 2]
    serve_over_xmla(warehouse)
    print("wheel smoke: ok")


def main(argv: list[str]) -> None:
    """The whole smoke, or under `--extension-only` the extension alone.

    The half is the caller's choice, never the environment's: a lane that owes
    the whole smoke fails when PyArrow is missing or broken rather than passing
    on the extension-only half, and a lane that passes the flag says why that
    half is all it can run.
    """
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument(
        "--extension-only",
        action="store_true",
        help="load the extension module alone: for an interpreter PyArrow publishes no wheel for",
    )
    if parser.parse_args(argv).extension_only:
        smoke_extension(
            f"PyArrow publishes no wheel for {interpreter_tag()} on {sysconfig.get_platform()}"
        )
    else:
        smoke_package()


if __name__ == "__main__":
    main(sys.argv[1:])
