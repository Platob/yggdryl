#!/usr/bin/env python3
"""Exchange S3 objects with boto3 against a real server, both directions.

Self-consistency proves nothing about a wire protocol. The fake store the unit
tests run against was written from the same reading of the S3 API as the
client, so the two can agree and both be wrong; and a signature, a percent
escape, or a continuation token that a hand-written store accepts is not
evidence that Amazon would. This driver runs the exchange against a real
implementation, cross-checked by the reference client:

1. a server is provisioned - MinIO by default, or whatever
   ``YGGDRYL_S3_ENDPOINT`` already points at - and the exchange bucket made;
2. ``boto3`` writes objects under ``from-boto3/``, including the keys that a
   URL, a signature, and a key each spell differently;
3. ``cargo test --features s3 --test interop s3::`` writes its own objects
   under ``from-rust/`` and reads back what ``boto3`` wrote. Its reading half
   prints ``SKIPPED`` when the external objects are missing, and this driver
   fails on that word, so a skipped half can never read as a pass;
4. ``boto3`` reads back every object the Rust side wrote and asserts the bytes,
   the sizes, and the exact key names - including the multipart upload, whose
   assembled content and part count are checked from the outside.

boto3 and MinIO are checking tools of this script only, never dependencies of
the crate.

Run it directly::

    python scripts/check_s3_interop.py

Point it at a store you already have, and nothing is provisioned::

    YGGDRYL_S3_ENDPOINT=http://localhost:9000 python scripts/check_s3_interop.py
"""

from __future__ import annotations

import os
import platform
import shutil
import stat
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MINIO_DIR = REPO / "rust" / "target" / "s3-interop"
BUCKET = "yggdryl-interop"
FROM_RUST = "from-rust"
FROM_BOTO = "from-boto3"
ACCESS_KEY = "minioadmin"
SECRET_KEY = "minioadmin"
REGION = "us-east-1"
PORT = 9123

# The keys boto3 writes: each one spells something differently as a URL
# component, as a signed canonical path, and as a raw key.
BOTO_KEYS = [
    "plain.txt",
    "nested/deep/part.parquet",
    "year=2026/month=02/part-1.parquet",
    "a b/spaced.txt",
    "a+b/plus.txt",
    "100%/percent.txt",
    "données/prix.txt",
    "tilde~and.dots..txt",
]

# What the Rust half writes, mirrored here so this side can assert it.
RUST_KEYS = [
    "plain.txt",
    "nested/deep/part.parquet",
    "year=2026/month=01/part-0.parquet",
    "a b/spaced.txt",
    "a+b/plus.txt",
    "100%/percent.txt",
    "données/prix.txt",
    "tilde~and.dots..txt",
]


def minio_url() -> str:
    """The MinIO release for this platform."""
    system = {"Linux": "linux", "Darwin": "darwin", "Windows": "windows"}[platform.system()]
    machine = {"x86_64": "amd64", "AMD64": "amd64", "arm64": "arm64", "aarch64": "arm64"}[
        platform.machine()
    ]
    suffix = ".exe" if system == "windows" else ""
    return f"https://dl.min.io/server/minio/release/{system}-{machine}/minio{suffix}"


def provision_minio() -> subprocess.Popen[bytes]:
    """Download MinIO if it is not here, start it, and wait for it to answer."""
    MINIO_DIR.mkdir(parents=True, exist_ok=True)
    binary = MINIO_DIR / ("minio.exe" if platform.system() == "Windows" else "minio")
    if not binary.exists():
        print(f"downloading {minio_url()}")
        with urllib.request.urlopen(minio_url()) as response, binary.open("wb") as out:
            shutil.copyfileobj(response, out)
        binary.chmod(binary.stat().st_mode | stat.S_IEXEC)

    data = MINIO_DIR / "data"
    shutil.rmtree(data, ignore_errors=True)
    data.mkdir(parents=True)
    environment = dict(
        os.environ,
        MINIO_ROOT_USER=ACCESS_KEY,
        MINIO_ROOT_PASSWORD=SECRET_KEY,
    )
    server = subprocess.Popen(
        [str(binary), "server", str(data), "--address", f"127.0.0.1:{PORT}"],
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(120):
        if server.poll() is not None:
            raise SystemExit("MinIO exited before it was ready")
        try:
            with urllib.request.urlopen(
                f"http://127.0.0.1:{PORT}/minio/health/live", timeout=1
            ) as health:
                if health.status == 200:
                    return server
        except Exception:  # noqa: BLE001 - any failure here just means "not yet"
            time.sleep(0.5)
    server.terminate()
    raise SystemExit("MinIO did not become ready")


def credentials(provisioned: bool) -> tuple[str, str, str]:
    """The keys and region both sides sign with.

    A store this script started answers only to the root keys it was given, so
    ambient AWS credentials must not win there - they are for the case where a
    caller pointed the run at a store of their own.
    """
    if provisioned:
        return ACCESS_KEY, SECRET_KEY, REGION
    return (
        os.environ.get("AWS_ACCESS_KEY_ID", ACCESS_KEY),
        os.environ.get("AWS_SECRET_ACCESS_KEY", SECRET_KEY),
        os.environ.get("AWS_REGION", REGION),
    )


def client(endpoint: str, provisioned: bool):
    """A boto3 S3 client for the exchange endpoint."""
    import boto3
    from botocore.config import Config

    access, secret, region = credentials(provisioned)
    return boto3.client(
        "s3",
        endpoint_url=endpoint,
        aws_access_key_id=access,
        aws_secret_access_key=secret,
        region_name=region,
        config=Config(signature_version="s3v4", s3={"addressing_style": "path"}),
    )


def write_with_boto3(s3) -> None:
    """Create the bucket and write the objects the Rust half reads back."""
    try:
        s3.create_bucket(Bucket=BUCKET)
    except s3.exceptions.BucketAlreadyOwnedByYou:
        pass
    except s3.exceptions.BucketAlreadyExists:
        pass

    # Clear the prefix so a re-run is not confused by an earlier one.
    for key in listed(s3, f"{FROM_BOTO}/"):
        s3.delete_object(Bucket=BUCKET, Key=key)

    for name in BOTO_KEYS:
        key = f"{FROM_BOTO}/{name}"
        # The content is the key, so the Rust side proves the name survived
        # the listing, the URL, and the signature by reading it.
        s3.put_object(Bucket=BUCKET, Key=key, Body=key.encode("utf-8"))
    print(f"boto3 wrote {len(BOTO_KEYS)} objects under {FROM_BOTO}/")


def listed(s3, prefix: str) -> list[str]:
    """Every key under ``prefix``, paged the way the API pages."""
    keys: list[str] = []
    token = None
    while True:
        arguments = {"Bucket": BUCKET, "Prefix": prefix}
        if token:
            arguments["ContinuationToken"] = token
        page = s3.list_objects_v2(**arguments)
        keys.extend(item["Key"] for item in page.get("Contents", []))
        if not page.get("IsTruncated"):
            return keys
        token = page["NextContinuationToken"]


def read_with_boto3(s3) -> None:
    """Assert every object the Rust half wrote, from the outside."""
    found = set(listed(s3, f"{FROM_RUST}/"))
    expected = {f"{FROM_RUST}/{name}" for name in RUST_KEYS}
    missing = expected - found
    if missing:
        raise SystemExit(f"boto3 cannot see objects the Rust half wrote: {sorted(missing)}")

    for key in sorted(expected):
        body = s3.get_object(Bucket=BUCKET, Key=key)["Body"].read()
        wanted = f"yggdryl wrote {key}".encode("utf-8")
        if body != wanted:
            raise SystemExit(f"{key}: boto3 read {body!r}, expected {wanted!r}")
        head = s3.head_object(Bucket=BUCKET, Key=key)
        if head["ContentLength"] != len(wanted):
            raise SystemExit(f"{key}: length {head['ContentLength']}, expected {len(wanted)}")
        # A ranged read from the outside, on the very bytes the Rust side
        # ranged over.
        ranged = s3.get_object(Bucket=BUCKET, Key=key, Range="bytes=0-3")["Body"].read()
        if ranged != wanted[:4]:
            raise SystemExit(f"{key}: range read {ranged!r}, expected {wanted[:4]!r}")

    multipart = f"{FROM_RUST}/multipart.bin"
    if multipart not in found:
        raise SystemExit(f"boto3 cannot see the multipart object at {multipart}")
    head = s3.head_object(Bucket=BUCKET, Key=multipart)
    if head["ContentLength"] != 12 * 1024 * 1024:
        raise SystemExit(f"{multipart}: length {head['ContentLength']}, expected 12 MiB")
    # A multipart ETag ends in `-{parts}`, which is how the outside sees that
    # the upload really was assembled from parts rather than sent whole.
    etag = head["ETag"].strip('"')
    if "-" not in etag:
        raise SystemExit(f"{multipart}: ETag {etag!r} is not a multipart tag")
    body = s3.get_object(Bucket=BUCKET, Key=multipart)["Body"].read()
    if body != b"y" * (12 * 1024 * 1024):
        raise SystemExit(f"{multipart}: the assembled bytes are not what was uploaded")
    print(f"boto3 verified {len(expected)} objects and the multipart upload")


def run_cargo(endpoint: str, provisioned: bool) -> str:
    """Run the Rust half against `endpoint`, failing on a skipped half."""
    command = [
        "cargo",
        "test",
        "--locked",
        "--manifest-path",
        str(REPO / "rust" / "Cargo.toml"),
        "--features",
        "s3",
        "--test",
        "interop",
        "s3::",
        "--",
        "--nocapture",
        "--test-threads=1",
    ]
    access, secret, region = credentials(provisioned)
    environment = dict(
        os.environ,
        YGGDRYL_S3_ENDPOINT=endpoint,
        AWS_ACCESS_KEY_ID=access,
        AWS_SECRET_ACCESS_KEY=secret,
        AWS_REGION=region,
    )
    print(" ".join(command))
    result = subprocess.run(command, env=environment, capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    if result.returncode != 0:
        raise SystemExit(f"the Rust half failed with {result.returncode}")
    if "SKIPPED" in result.stdout:
        raise SystemExit("the Rust half skipped; it must run against the external objects")
    return result.stdout


def main() -> int:
    endpoint = os.environ.get("YGGDRYL_S3_ENDPOINT", "").strip()
    server = None
    provisioned = not endpoint
    if endpoint:
        print(f"using the store already at {endpoint}")
    else:
        server = provision_minio()
        endpoint = f"http://127.0.0.1:{PORT}"
        print(f"provisioned MinIO at {endpoint}")

    try:
        s3 = client(endpoint, provisioned)
        write_with_boto3(s3)
        run_cargo(endpoint, provisioned)
        read_with_boto3(s3)
    finally:
        if server is not None:
            server.terminate()
            server.wait(timeout=30)

    print("s3 interop: both directions agree")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
