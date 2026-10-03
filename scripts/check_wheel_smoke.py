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
about to publish that a runner can import (``docs/testing.md`` names the three
it cannot), and ``ci.yml``'s Python lane, against the wheel that job already
builds and installs. A rename now breaks the pull request that makes
it, which is the only place the cost of fixing it is small.

It imports ``yggdryl`` and never ``python/yggdryl``: run it from anywhere,
and what it reports on is what the interpreter resolves - the installed
distribution, not the source tree beside it.

Usage:
    python scripts/check_wheel_smoke.py
"""

from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import urllib.request

import pyarrow as pa

import yggdryl
from yggdryl import IOBase
from yggdryl.iceberg import IcebergTable

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


def main() -> None:
    """Create a table, append a batch, read it back, serve it, and say where from."""
    print(f"yggdryl {yggdryl.__version__} from {yggdryl.__file__}")

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-release-"))
    table = IcebergTable.create(IOBase(warehouse / "smoke"), columns)
    table.append(pa.record_batch({"id": [1, 2]}, schema=columns))
    assert table.scan().read_all().column("id").to_pylist() == [1, 2]
    serve_over_xmla(warehouse)
    print("wheel smoke: ok")


if __name__ == "__main__":
    main()
