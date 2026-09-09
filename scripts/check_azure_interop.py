#!/usr/bin/env python3
"""Exchange blobs with azure-storage-blob against a real server, both directions.

Self-consistency proves nothing about a wire protocol. The fake store the unit
tests run against answers Azure's REST API, but it was written from the same
reading of that API as the client, so the two can agree and both be wrong. This
driver runs the exchange against a real implementation, cross-checked by the
reference client.

Azurite is the right server for this store because it is the one that checks
the *signature*: it recomputes the Shared Key ``StringToSign`` exactly as the
service does and answers ``403`` for anything that does not match. The order of
the thirteen signed lines, the empty ``Content-Length`` for a zero-length body,
the account appearing twice in the canonicalized resource of a path-style
endpoint, and the rule that every sub-request of a batch is authorized on its
own - each is checked here by a server that did not read this crate's source.

1. a server is provisioned - Azurite by default, or whatever
   ``YGGDRYL_AZURE_ENDPOINT`` already points at - and the exchange container
   made;
2. ``azure-storage-blob`` writes blobs under ``from-azure/``, including the
   names that a URL, a signature, and a raw name each spell differently;
3. ``cargo test --features object --test interop object::azure::`` writes its
   own blobs under ``from-rust/`` and reads back what the reference client
   wrote. Its reading half prints ``SKIPPED`` when the external blobs are
   missing, and this driver fails on that word, so a skipped half can never
   read as a pass;
4. the reference client reads back every blob the Rust side wrote and asserts
   the bytes, the sizes, and the exact names - including the block-list upload,
   whose assembled content is checked from the outside.

Azurite and azure-storage-blob are checking tools of this script only, never
dependencies of the crate.

Run it directly::

    python scripts/check_azure_interop.py

Point it at a store you already have, and nothing is provisioned::

    YGGDRYL_AZURE_ENDPOINT=http://127.0.0.1:10000 python scripts/check_azure_interop.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
AZURITE_DIR = REPO / "rust" / "target" / "azure-interop"
CONTAINER = "yggdryl-interop"
FROM_RUST = "from-rust"
FROM_REFERENCE = "from-azure"
PORT = 10123
# Published by Microsoft as the development account: identical in every
# emulator, and therefore a constant rather than a secret.
ACCOUNT = "devstoreaccount1"
KEY = (
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/"
    "KBHBeksoGMGw=="
)

# The names the reference client writes: each one spells something different as
# a URL component, as a signed canonical path, and as a raw name.
REFERENCE_NAMES = ["a b/spaced.txt", "plus+sign.txt", "unicode-é.txt"]
# What the Rust half writes, mirrored here so this side can assert it.
RUST_BLOBS = {
    f"{FROM_RUST}/quotes.csv": b"symbol,price\nAAPL,187.23\n",
    f"{FROM_RUST}/a b/spaced.txt": "a b/spaced.txt".encode(),
    f"{FROM_RUST}/plus+sign.txt": "plus+sign.txt".encode(),
    f"{FROM_RUST}/equals=sign.txt": "equals=sign.txt".encode(),
    f"{FROM_RUST}/unicode-é.txt": "unicode-é.txt".encode(),
}
LARGE = f"{FROM_RUST}/large.bin"
LARGE_SIZE = 9 * 1024 * 1024


def azurite_url() -> str:
    """Where the provisioned emulator listens."""
    return f"http://127.0.0.1:{PORT}"


def connection_string(endpoint: str) -> str:
    """The one intake most Azure configuration arrives as."""
    return (
        f"DefaultEndpointsProtocol=http;AccountName={ACCOUNT};AccountKey={KEY};"
        f"BlobEndpoint={endpoint}/{ACCOUNT};"
    )


def provision_azurite() -> subprocess.Popen[bytes]:
    """Start Azurite, installing it with npm if it is not already there."""
    binary = AZURITE_DIR / "node_modules" / ".bin" / "azurite-blob"
    if not binary.exists():
        AZURITE_DIR.mkdir(parents=True, exist_ok=True)
        npm = shutil.which("npm")
        if npm is None:
            raise SystemExit("npm is needed to install Azurite, and is not on PATH")
        print("installing azurite")
        subprocess.run(
            [npm, "install", "--silent", "--no-fund", "--no-audit", "azurite@3"],
            cwd=AZURITE_DIR,
            check=True,
        )
    data = AZURITE_DIR / "data"
    shutil.rmtree(data, ignore_errors=True)
    data.mkdir(parents=True, exist_ok=True)
    print(f"starting azurite on {azurite_url()}")
    process = subprocess.Popen(
        [
            str(binary),
            "--blobHost",
            "127.0.0.1",
            "--blobPort",
            str(PORT),
            "--location",
            str(data),
            "--silent",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(60):
        if process.poll() is not None:
            raise SystemExit("azurite exited before it was ready")
        try:
            # An unsigned request is refused, and a refusal is proof it is up.
            urllib.request.urlopen(f"{azurite_url()}/{ACCOUNT}?comp=list", timeout=1)
        except urllib.error.HTTPError:
            return process
        except OSError:
            time.sleep(0.25)
    process.terminate()
    raise SystemExit("azurite did not become ready")


def client(endpoint: str):
    """The reference client, pointed at `endpoint`."""
    from azure.storage.blob import BlobServiceClient

    return BlobServiceClient.from_connection_string(connection_string(endpoint))


def write_with_reference(service) -> None:
    """Write the exchange blobs the Rust half will read."""
    container = service.get_container_client(CONTAINER)
    try:
        container.create_container()
    except Exception:  # noqa: BLE001 - already there is the ordinary case
        pass
    container.upload_blob(
        f"{FROM_REFERENCE}/quotes.csv",
        b"symbol,price\nMSFT,411.10\n",
        overwrite=True,
    )
    for name in REFERENCE_NAMES:
        container.upload_blob(
            f"{FROM_REFERENCE}/{name}", name.encode(), overwrite=True
        )
    print(f"wrote {len(REFERENCE_NAMES) + 1} blobs under {FROM_REFERENCE}/")


def read_with_reference(service) -> None:
    """Assert every blob the Rust half wrote, from the outside."""
    container = service.get_container_client(CONTAINER)
    for name, expected in RUST_BLOBS.items():
        held = container.download_blob(name).readall()
        if held != expected:
            raise SystemExit(f"{name}: expected {expected!r}, got {held!r}")
    print(f"read back {len(RUST_BLOBS)} blobs written by the Rust half")

    # The block-list upload is the one worth checking from the outside: its
    # blocks were staged under ids this client chose, and a wrong id length is
    # something only the service enforces.
    assembled = container.download_blob(LARGE).readall()
    if len(assembled) != LARGE_SIZE:
        raise SystemExit(f"{LARGE}: expected {LARGE_SIZE} bytes, got {len(assembled)}")
    expected = bytes((index % 251) for index in range(LARGE_SIZE))
    if assembled != expected:
        raise SystemExit(f"{LARGE}: the assembled blocks are not what was written")
    blocks = container.get_blob_client(LARGE).get_block_list("committed")[0]
    widths = {len(block.id) for block in blocks}
    if len(widths) != 1:
        raise SystemExit(f"{LARGE}: block ids of differing width: {widths}")
    print(f"the {len(blocks)}-block upload assembled to {LARGE_SIZE} bytes")

    # And the names, exactly as the Rust half spelled them.
    listed = {blob.name for blob in container.list_blobs(name_starts_with=FROM_RUST)}
    for name in RUST_BLOBS:
        if name not in listed:
            raise SystemExit(f"{name} is not in the listing: {sorted(listed)}")
    print(f"the listing names all {len(RUST_BLOBS)} of them")


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
        "object::azure::",
        "--",
        "--nocapture",
        "--test-threads=1",
    ]
    environment = dict(
        os.environ,
        YGGDRYL_AZURE_ENDPOINT=endpoint,
        AZURE_STORAGE_ACCOUNT_NAME=ACCOUNT,
        AZURE_STORAGE_ACCOUNT_KEY=KEY,
    )
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
    endpoint = os.environ.get("YGGDRYL_AZURE_ENDPOINT", "").strip()
    process = None
    if not endpoint:
        process = provision_azurite()
        endpoint = azurite_url()
    try:
        service = client(endpoint)
        write_with_reference(service)
        printed = run_cargo(endpoint)
        # A half that decided there was nothing to read is not a pass.
        if "SKIPPED" in printed:
            raise SystemExit("the Rust half skipped a check")
        read_with_reference(service)
    finally:
        if process is not None:
            process.terminate()
            process.wait(timeout=10)
    print("azure interop: both directions agree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
