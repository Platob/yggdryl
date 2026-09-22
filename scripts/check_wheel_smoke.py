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
about to publish, and ``ci.yml``'s Python lane, against the wheel that job
already builds and installs. A rename now breaks the pull request that makes
it, which is the only place the cost of fixing it is small.

It imports ``yggdryl`` and never ``python/yggdryl``: run it from anywhere,
and what it reports on is what the interpreter resolves - the installed
distribution, not the source tree beside it.

Usage:
    python scripts/check_wheel_smoke.py
"""

from __future__ import annotations

import pathlib
import tempfile

import pyarrow as pa

import yggdryl
from yggdryl import IOBase
from yggdryl.iceberg import Table


def main() -> None:
    """Create a table, append a batch, read it back, and say where from."""
    print(f"yggdryl {yggdryl.__version__} from {yggdryl.__file__}")

    columns = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-release-")) / "smoke"
    table = Table.create(IOBase(root), columns)
    table.append(pa.record_batch({"id": [1, 2]}, schema=columns))
    assert table.scan().read_all().column("id").to_pylist() == [1, 2]
    print("wheel smoke: ok")


if __name__ == "__main__":
    main()
