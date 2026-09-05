"""Provision Apache Spark and the Iceberg Spark runtime for the interop suite.

The Iceberg interop tests in ``python/tests/media/test_spark_interop.py`` exchange
tables with Apache Spark, the format's reference implementation. Spark is a
heavyweight dependency - a JVM, a few hundred megabytes of packages, and one
runtime jar from Maven Central - so it is provisioned here, on demand, and
never by the default test run.

Usage, from the repository root::

    python scripts/setup_spark_interop.py           # provision
    python/.venv/bin/python -m pytest -m spark_interop  # run the suite

What this script does:

1. installs ``pyspark==PYSPARK_VERSION`` into ``python/.venv`` (or into the
   running interpreter's environment when no venv exists there);
2. downloads ``iceberg-spark-runtime-{SPARK_SERIES}_{SCALA}-{ICEBERG_VERSION}``
   from Maven Central into ``python/.spark-interop/``, where the test suite
   looks for it (override with ``YGGDRYL_ICEBERG_SPARK_JAR``);
3. verifies a Java runtime is on ``PATH``, because Spark cannot run without
   one - it is *not* installed here, since every CI image and most
   workstations already have one.

The suite skips itself, with a message naming what is missing, when any of
the three is absent - so this script is the one switch that turns it on.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

PYSPARK_VERSION = "4.1.3"
SPARK_SERIES = "4.1"
SCALA = "2.13"
ICEBERG_VERSION = "1.11.0"

REPO = Path(__file__).resolve().parent.parent
JAR_DIR = REPO / "python" / ".spark-interop"
JAR_NAME = f"iceberg-spark-runtime-{SPARK_SERIES}_{SCALA}-{ICEBERG_VERSION}.jar"
JAR_URL = (
    "https://repo1.maven.org/maven2/org/apache/iceberg/"
    f"iceberg-spark-runtime-{SPARK_SERIES}_{SCALA}/{ICEBERG_VERSION}/{JAR_NAME}"
)


def venv_python() -> Path:
    """The interpreter of ``python/.venv``, or the one running this script."""
    for candidate in (
        REPO / "python" / ".venv" / "bin" / "python",
        REPO / "python" / ".venv" / "Scripts" / "python.exe",
    ):
        if candidate.exists():
            return candidate
    return Path(sys.executable)


def install(python: Path) -> None:
    """Install pyspark with whatever installer the environment offers.

    A ``uv``-created venv carries no pip, so ``uv pip`` is preferred when uv
    is on PATH; otherwise pip is used, bootstrapped through ``ensurepip``
    when the venv lacks it.
    """
    requirement = f"pyspark=={PYSPARK_VERSION}"
    if shutil.which("uv") is not None:
        subprocess.run(
            ["uv", "pip", "install", "--quiet", "--python", str(python), requirement],
            check=True,
        )
        return
    if subprocess.run(
        [str(python), "-m", "pip", "--version"], capture_output=True, check=False
    ).returncode != 0:
        subprocess.run([str(python), "-m", "ensurepip", "--upgrade"], check=True)
    subprocess.run(
        [str(python), "-m", "pip", "install", "--quiet", requirement], check=True
    )


def main() -> int:
    if shutil.which("java") is None:
        print(
            "warning: no `java` on PATH; install a JRE (17 or 21) or the "
            "suite will skip",
            file=sys.stderr,
        )

    python = venv_python()
    print(f"installing pyspark=={PYSPARK_VERSION} into {python} ...")
    install(python)

    JAR_DIR.mkdir(parents=True, exist_ok=True)
    jar = JAR_DIR / JAR_NAME
    # A jar is a zip; a truncated download would fail Spark with an opaque
    # ClassNotFoundException, so validity is checked here and a bad file is
    # fetched again rather than trusted.
    if jar.exists() and zipfile.is_zipfile(jar):
        print(f"already present: {jar}")
    else:
        print(f"downloading {JAR_URL} ...")
        with urllib.request.urlopen(JAR_URL) as response, open(jar, "wb") as out:
            shutil.copyfileobj(response, out)
        if not zipfile.is_zipfile(jar):
            jar.unlink()
            raise SystemExit(f"downloaded jar is not a valid zip: {JAR_URL}")
        print(f"saved {jar} ({jar.stat().st_size} bytes)")

    print("done; run the suite with: python -m pytest -m spark_interop")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
