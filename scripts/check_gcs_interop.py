#!/usr/bin/env python3
"""Exchange objects with google-cloud-storage against a real server, both ways.

Self-consistency proves nothing about a wire protocol. The fake store the unit
tests run against answers Google's JSON API, but it was written from the same
reading of that API as the client, so the two can agree and both be wrong. This
driver runs the exchange against a real implementation, cross-checked by the
reference client.

What ``fake-gcs-server`` checks is the *shape* of every request: the
``/storage/v1`` and ``/upload/storage/v1`` paths, the object name escaped into
one path segment, ``alt=media`` switching metadata for bytes, the
``multipart/related`` framing of a small write, and the resumable protocol's
``Content-Range`` and 308 answers. What it does not check is the bearer token,
because it accepts any - so this driver proves the dialect and not the identity,
and says so rather than implying more.

1. a server is provisioned - ``fake-gcs-server`` by default, or whatever
   ``YGGDRYL_GCS_ENDPOINT`` already points at - and the exchange bucket made;
2. ``google-cloud-storage`` writes objects under ``from-google/``, including
   the names whose separators the JSON API escapes into one segment;
3. ``cargo test --features object --test interop object::gcs::`` writes its own
   objects under ``from-rust/`` and reads back what the reference client wrote.
   Its reading half prints ``SKIPPED`` when the external objects are missing,
   and this driver fails on that word, so a skipped half can never read as a
   pass;
4. the reference client reads back every object the Rust side wrote and asserts
   the bytes, the sizes, and the exact names - including the resumable upload,
   whose assembled content is checked from the outside.

``fake-gcs-server`` and ``google-cloud-storage`` are checking tools of this
script only, never dependencies of the crate.

Run it directly::

    python scripts/check_gcs_interop.py

Point it at a store you already have, and nothing is provisioned::

    YGGDRYL_GCS_ENDPOINT=http://127.0.0.1:4443 python scripts/check_gcs_interop.py
"""

from __future__ import annotations

import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EMULATOR_DIR = REPO / "rust" / "target" / "gcs-interop"
BUCKET = "yggdryl-interop"
FROM_RUST = "from-rust"
FROM_REFERENCE = "from-google"
PORT = 4499
# Pinned: the emulator is a checking tool, and a moving one would make a
# failure ambiguous between this crate and its next release.
EMULATOR_VERSION = "1.52.2"

# The names the reference client writes: the JSON API escapes every separator
# into one path segment, which is where a hand-written client addresses the
# wrong object.
REFERENCE_NAMES = ["a b/spaced.txt", "plus+sign.txt", "unicode-é.txt"]
# What the Rust half writes, mirrored here so this side can assert it.
RUST_OBJECTS = {
    f"{FROM_RUST}/quotes.csv": b"symbol,price\nAAPL,187.23\n",
    f"{FROM_RUST}/a b/spaced.txt": "a b/spaced.txt".encode(),
    f"{FROM_RUST}/plus+sign.txt": "plus+sign.txt".encode(),
    f"{FROM_RUST}/equals=sign.txt": "equals=sign.txt".encode(),
    f"{FROM_RUST}/unicode-é.txt": "unicode-é.txt".encode(),
}
LARGE = f"{FROM_RUST}/large.bin"
LARGE_SIZE = 9 * 1024 * 1024


def emulator_url() -> str:
    """Where the provisioned emulator listens."""
    return f"http://127.0.0.1:{PORT}"


def emulator_release_url() -> str:
    """The published `fake-gcs-server` build for this platform."""
    system = {"Linux": "Linux", "Darwin": "Darwin", "Windows": "Windows"}[platform.system()]
    machine = {"x86_64": "amd64", "AMD64": "amd64", "arm64": "arm64", "aarch64": "arm64"}[
        platform.machine()
    ]
    return (
        "https://github.com/fsouza/fake-gcs-server/releases/download/"
        f"v{EMULATOR_VERSION}/fake-gcs-server_{EMULATOR_VERSION}_{system}_{machine}.tar.gz"
    )


def provision_emulator() -> subprocess.Popen[bytes]:
    """Start `fake-gcs-server`, fetching the published build if it is not here."""
    name = "fake-gcs-server.exe" if platform.system() == "Windows" else "fake-gcs-server"
    binary = EMULATOR_DIR / "bin" / name
    if not binary.exists():
        # The published build, not `go install`: building it from source pulls
        # the whole Google Cloud and OpenTelemetry module graph over the network
        # on every run, and a single stream the Go proxy drops fails the lane
        # for a reason that has nothing to do with this crate. One archive is
        # one request, and it is the same binary.
        binary.parent.mkdir(parents=True, exist_ok=True)
        archive = EMULATOR_DIR / "fake-gcs-server.tar.gz"
        print(f"downloading {emulator_release_url()}")
        with urllib.request.urlopen(emulator_release_url()) as response, archive.open("wb") as out:
            shutil.copyfileobj(response, out)
        with tarfile.open(archive) as tar:
            member = tar.getmember(name)
            member.name = name
            tar.extract(member, binary.parent, filter="data")
        archive.unlink()
        binary.chmod(binary.stat().st_mode | stat.S_IEXEC)
    print(f"starting fake-gcs-server on {emulator_url()}")
    process = subprocess.Popen(
        [
            str(binary),
            "-scheme",
            "http",
            "-host",
            "127.0.0.1",
            "-port",
            str(PORT),
            "-backend",
            "memory",
            "-external-url",
            emulator_url(),
            "-public-host",
            f"127.0.0.1:{PORT}",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(60):
        if process.poll() is not None:
            raise SystemExit("fake-gcs-server exited before it was ready")
        try:
            urllib.request.urlopen(f"{emulator_url()}/storage/v1/b", timeout=1)
            return process
        except urllib.error.HTTPError:
            return process
        except OSError:
            time.sleep(0.25)
    process.terminate()
    raise SystemExit("fake-gcs-server did not become ready")


def client(endpoint: str):
    """The reference client, pointed at `endpoint` with no identity."""
    os.environ["STORAGE_EMULATOR_HOST"] = endpoint
    from google.auth.credentials import AnonymousCredentials
    from google.cloud import storage

    return storage.Client(project=BUCKET, credentials=AnonymousCredentials())


def write_with_reference(service) -> None:
    """Write the exchange objects the Rust half will read."""
    bucket = service.bucket(BUCKET)
    if not bucket.exists():
        bucket = service.create_bucket(BUCKET)
    bucket.blob(f"{FROM_REFERENCE}/quotes.csv").upload_from_string(
        b"symbol,price\nMSFT,411.10\n"
    )
    for name in REFERENCE_NAMES:
        bucket.blob(f"{FROM_REFERENCE}/{name}").upload_from_string(name.encode())
    print(f"wrote {len(REFERENCE_NAMES) + 1} objects under {FROM_REFERENCE}/")


def read_with_reference(service) -> None:
    """Assert every object the Rust half wrote, from the outside."""
    bucket = service.bucket(BUCKET)
    for name, expected in RUST_OBJECTS.items():
        held = bucket.blob(name).download_as_bytes()
        if held != expected:
            raise SystemExit(f"{name}: expected {expected!r}, got {held!r}")
    print(f"read back {len(RUST_OBJECTS)} objects written by the Rust half")

    # The resumable upload is the one worth checking from the outside: its
    # chunks each stated the byte range they carried, and an off-by-one there
    # assembles to something that is the right length and the wrong bytes.
    assembled = bucket.blob(LARGE).download_as_bytes()
    if len(assembled) != LARGE_SIZE:
        raise SystemExit(f"{LARGE}: expected {LARGE_SIZE} bytes, got {len(assembled)}")
    expected = bytes((index % 251) for index in range(LARGE_SIZE))
    if assembled != expected:
        raise SystemExit(f"{LARGE}: the assembled chunks are not what was written")
    print(f"the resumable upload assembled to {LARGE_SIZE} bytes")

    listed = {blob.name for blob in service.list_blobs(BUCKET, prefix=FROM_RUST)}
    for name in RUST_OBJECTS:
        if name not in listed:
            raise SystemExit(f"{name} is not in the listing: {sorted(listed)}")
    print(f"the listing names all {len(RUST_OBJECTS)} of them")


def run_cargo(endpoint: str) -> str:
    """Run the Rust half against `endpoint`, failing on a skipped half."""
    command = [
        "cargo",
        "test",
        "--locked",
        "--manifest-path",
        str(REPO / "rust" / "Cargo.toml"),
        "--features",
        "object",
        "--test",
        "interop",
        "object::gcs::",
        "--",
        "--nocapture",
        "--test-threads=1",
    ]
    environment = dict(os.environ, YGGDRYL_GCS_ENDPOINT=endpoint)
    print(" ".join(command))
    finished = subprocess.run(
        command, cwd=REPO, env=environment, capture_output=True, text=True
    )
    print(finished.stdout)
    print(finished.stderr, file=sys.stderr)
    if finished.returncode != 0:
        raise SystemExit("the Rust half failed")
    return finished.stdout


def main() -> int:
    endpoint = os.environ.get("YGGDRYL_GCS_ENDPOINT", "").strip()
    process = None
    if not endpoint:
        process = provision_emulator()
        endpoint = emulator_url()
    try:
        service = client(endpoint)
        write_with_reference(service)
        printed = run_cargo(endpoint)
        if "SKIPPED" in printed:
            raise SystemExit("the Rust half skipped a check")
        read_with_reference(service)
    finally:
        if process is not None:
            process.terminate()
            process.wait(timeout=10)
    print("gcs interop: both directions agree")
    print("note: the emulator accepts any bearer token, so this proves the")
    print("dialect and not the identity - a token chain is not checked here")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
