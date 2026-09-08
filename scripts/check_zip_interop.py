#!/usr/bin/env python3
"""Exchange ZIP archives with Python's ``zipfile``, both directions.

Self-consistency proves nothing about an exchange format, so this driver runs
the same archives through the standard library's implementation:

1. ``cargo test --test interop zip::`` writes ``target/zip-interop/from-rust.zip``.
2. ``zipfile`` reads it, verifies every member and every CRC, and asserts the
   directory records - names, compression methods, sizes, comment.
3. ``zipfile`` writes ``target/zip-interop/from-python.zip`` with the same
   members, mixing stored and deflated ones, and
   ``target/zip-interop/from-python-streamed.zip`` through an unseekable
   stream, so every member carries a trailing data descriptor instead of
   sizes in its local header.
4. The same cargo target runs again; its reading half decodes the external
   archive and asserts the members, and its updating half rewrites one member
   of a copy. That half prints ``SKIPPED`` when the file is absent, and this
   driver fails on that word, so a skipped half can never read as a pass.
5. ``zipfile`` reads the updated copy, proving an archive this crate rewrote
   is still one the reference implementation accepts.

``zipfile`` is the standard library, so this adds no dependency to anything.
"""

from __future__ import annotations

import io
import subprocess
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXCHANGE = REPO / "rust" / "target" / "zip-interop"

# The members both sides assert, matching rust/tests/interop/zip.rs.
MEMBERS: list[tuple[str, bytes]] = [
    ("empty.bin", b""),
    ("notes/read me.txt", b"symbol,price\n"),
    ("trades/2024/eu.csv", b"symbol,price\nAAPL,187.23\n" * 64),
    ("trades/2024/us.csv", b"symbol,price\nMSFT,412.10\n"),
    # Longer than one write window and many restart strides, so the Rust
    # writer settles its local header afterwards and its DEFLATE stream
    # carries full flushes. Both must be invisible to the reference reader.
    ("trades/2024/big.csv", b"symbol,price\nNVDA,131.14\n" * 60_000),
]
# The one member the Rust writer stores rather than deflates.
STORED = "trades/2024/us.csv"


def run_cargo(allow_skip: bool) -> str:
    """Run the Rust half, failing on a skipped read unless it is expected."""
    result = subprocess.run(
        ["cargo", "test", "--locked", "--test", "interop", "zip::", "--", "--nocapture"],
        cwd=REPO / "rust",
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise SystemExit(f"cargo test failed:\n{output}")
    if not allow_skip and "SKIPPED" in output:
        raise SystemExit(f"the Rust reading half skipped:\n{output}")
    return output


def read_with_zipfile(path: Path) -> None:
    """Assert the archive this crate wrote, through the reference reader."""
    with zipfile.ZipFile(path) as archive:
        broken = archive.testzip()
        if broken is not None:
            raise SystemExit(f"zipfile rejected the member {broken!r} of {path}")
        if archive.comment != b"written by yggdryl":
            raise SystemExit(f"unexpected archive comment: {archive.comment!r}")

        listed = {info.filename: info for info in archive.infolist()}
        for name, payload in MEMBERS:
            if name not in listed:
                raise SystemExit(f"zipfile did not find {name!r} in {sorted(listed)}")
            info = listed[name]
            expected_method = (
                zipfile.ZIP_STORED if name == STORED else zipfile.ZIP_DEFLATED
            )
            if info.compress_type != expected_method:
                raise SystemExit(
                    f"{name!r}: expected method {expected_method}, got {info.compress_type}"
                )
            if info.file_size != len(payload):
                raise SystemExit(
                    f"{name!r}: expected {len(payload)} bytes, got {info.file_size}"
                )
            read = archive.read(name)
            if read != payload:
                raise SystemExit(f"{name!r}: member bytes differ")

        if "empty-directory/" not in listed:
            raise SystemExit(f"no directory record in {sorted(listed)}")
        if not listed["empty-directory/"].is_dir():
            raise SystemExit("the directory record does not read as a directory")

        # A nested archive is bytes to this reader and a resource to the other.
        nested = archive.read("inner.zip")
        with zipfile.ZipFile(io.BytesIO(nested)) as inner:
            if inner.read("deep/notes.txt") != b"nested":
                raise SystemExit("the nested archive does not read back")

        # The restart points the Rust writer emitted are a full flush, which
        # is an ordinary part of the stream: the reference decoder reads the
        # member whole without knowing they are there. Its map rides an extra
        # field this reader skips by length, which is what keeps it invisible.
        big = listed["trades/2024/big.csv"]
        if big.compress_type != zipfile.ZIP_DEFLATED:
            raise SystemExit("the streamed member is not deflated")
        if not any(
            identifier == 0x5967 for identifier, _ in _extras(big.extra)
        ):
            raise SystemExit("the streamed member carries no restart map")


def _extras(extra: bytes) -> list[tuple[int, bytes]]:
    """Split an extra field block into the records it holds."""
    fields: list[tuple[int, bytes]] = []
    at = 0
    while at + 4 <= len(extra):
        identifier = int.from_bytes(extra[at : at + 2], "little")
        length = int.from_bytes(extra[at + 2 : at + 4], "little")
        fields.append((identifier, extra[at + 4 : at + 4 + length]))
        at += 4 + length
    return fields


def write_with_zipfile(path: Path) -> None:
    """Write the archive the Rust reading half asserts."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as archive:
        archive.comment = b"written by python"
        for name, payload in MEMBERS:
            method = zipfile.ZIP_STORED if name == STORED else zipfile.ZIP_DEFLATED
            archive.writestr(name, payload, compress_type=method)
        # A placeholder for the code page name patched in below.
        archive.writestr(CODE_PAGE_PLACEHOLDER, b"symbol,price\n")
    _spell_in_code_page(path)


# `zipfile` encodes every name as ASCII or UTF-8, so the one spelling it
# cannot produce is the code page the format defaults to - which is exactly
# what a pre-Unicode writer leaves behind and what this reader has to survive.
# The placeholder is the same length as the name that replaces it, so only the
# one byte changes and every record stays exactly as long as it declared.
CODE_PAGE_PLACEHOLDER = "notes/cafX.txt"
# IBM 437 spells 0x82 as `e` with an acute accent.
CODE_PAGE_NAME = b"notes/caf\x82.txt"


def _spell_in_code_page(path: Path) -> None:
    """Rewrite the placeholder name as the bytes a code page would."""
    raw = path.read_bytes()
    placeholder = CODE_PAGE_PLACEHOLDER.encode("ascii")
    if placeholder not in raw:
        raise SystemExit(f"the placeholder name is not in {path}")
    path.write_bytes(raw.replace(placeholder, CODE_PAGE_NAME))


class _Unseekable:
    """A file object that cannot seek, which is what forces descriptors."""

    def __init__(self, target) -> None:
        self._target = target

    def write(self, data: bytes) -> int:
        return self._target.write(data)

    def flush(self) -> None:
        self._target.flush()

    def seekable(self) -> bool:
        return False


def write_streamed_with_zipfile(path: Path) -> None:
    """Write an archive whose members carry trailing data descriptors.

    ``zipfile`` normally seeks back to patch a local header. Denied a seek it
    sets the data-descriptor bit and writes the digest and both sizes after
    the member instead, which is the shape a streaming writer produces and the
    one compaction has to settle when it moves a record.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as raw:
        with zipfile.ZipFile(_Unseekable(raw), "w") as archive:
            for name, payload in MEMBERS:
                method = zipfile.ZIP_STORED if name == STORED else zipfile.ZIP_DEFLATED
                archive.writestr(name, payload, compress_type=method)
    with zipfile.ZipFile(path) as archive:
        descriptors = sum(
            1 for info in archive.infolist() if info.flag_bits & 0x08
        )
    if descriptors != len(MEMBERS):
        raise SystemExit(
            f"expected {len(MEMBERS)} streamed members, got {descriptors}"
        )


def read_streamed_round_trip(path: Path) -> None:
    """Assert the compacted copy of the streamed archive."""
    with zipfile.ZipFile(path) as archive:
        broken = archive.testzip()
        if broken is not None:
            raise SystemExit(f"zipfile rejected the member {broken!r} of {path}")
        for info in archive.infolist():
            if info.flag_bits & 0x08:
                raise SystemExit(
                    f"{info.filename!r} still claims a data descriptor after compaction"
                )
        for name, payload in MEMBERS:
            if name == "empty.bin":
                continue
            expected = (
                b"symbol,price\nAAPL,999.99\n" if name == "trades/2024/eu.csv" else payload
            )
            if archive.read(name) != expected:
                raise SystemExit(f"{name!r}: member bytes differ after compaction")


def read_round_trip(path: Path) -> None:
    """Assert the archive this crate rewrote, through the reference reader."""
    with zipfile.ZipFile(path) as archive:
        broken = archive.testzip()
        if broken is not None:
            raise SystemExit(f"zipfile rejected the member {broken!r} of {path}")
        names = set(archive.namelist())
        if "empty.bin" in names:
            raise SystemExit("the removed member is still in the directory")
        if "added/by-rust.txt" not in names:
            raise SystemExit(f"the appended member is missing from {sorted(names)}")
        if archive.read("added/by-rust.txt") != b"appended":
            raise SystemExit("the appended member's bytes differ")
        if archive.read("trades/2024/eu.csv") != b"symbol,price\nAAPL,999.99\n":
            raise SystemExit("the rewritten member's bytes differ")
        if archive.read("trades/2024/us.csv") != dict(MEMBERS)[STORED]:
            raise SystemExit("an untouched member changed")
        if archive.comment != b"written by python":
            raise SystemExit(f"the comment was lost: {archive.comment!r}")


def main() -> int:
    EXCHANGE.mkdir(parents=True, exist_ok=True)
    for stale in (
        "from-rust.zip",
        "from-python.zip",
        "from-python-streamed.zip",
        "round-trip.zip",
        "round-trip-streamed.zip",
    ):
        (EXCHANGE / stale).unlink(missing_ok=True)

    # The first run has nothing external to read, so its reading half skips.
    run_cargo(allow_skip=True)
    read_with_zipfile(EXCHANGE / "from-rust.zip")
    print("zipfile read the archive yggdryl wrote")

    write_with_zipfile(EXCHANGE / "from-python.zip")
    write_streamed_with_zipfile(EXCHANGE / "from-python-streamed.zip")
    run_cargo(allow_skip=False)
    print("yggdryl read the archive zipfile wrote")

    read_round_trip(EXCHANGE / "round-trip.zip")
    print("zipfile read the archive yggdryl rewrote")

    read_streamed_round_trip(EXCHANGE / "round-trip-streamed.zip")
    print("zipfile read the streamed archive yggdryl compacted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
