"""Recalculate an .xlsx with LibreOffice Calc and print every sheet's values.

A local cross-check for the formula engine: LibreOffice is told to always
recalculate OOXML on load (the profile's OOXMLRecalcMode), then each sheet is
exported to CSV. Usage: recalc.py <book.xlsx> [out-dir]
"""

import csv
import glob
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROFILE = os.path.join(HERE, "profile")
REGISTRY = """<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Calc/Formula/Load"><prop oor:name="OOXMLRecalcMode" oor:op="fuse"><value>0</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Formula/Load"><prop oor:name="ODFRecalcMode" oor:op="fuse"><value>0</value></prop></item>
</oor:items>
"""


def recalculated(book, out_dir):
    user = os.path.join(PROFILE, "user")
    os.makedirs(user, exist_ok=True)
    with open(os.path.join(user, "registrymodifications.xcu"), "w") as f:
        f.write(REGISTRY)
    os.makedirs(out_dir, exist_ok=True)
    env = dict(os.environ, HOME=PROFILE)
    subprocess.run(
        [
            "soffice", f"-env:UserInstallation=file://{PROFILE}", "--headless", "--norestore",
            "--convert-to", 'csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,false,false,false,-1',
            "--outdir", out_dir, book,
        ],
        check=True, capture_output=True, env=env, timeout=240,
    )
    stem = os.path.splitext(os.path.basename(book))[0]
    sheets = {}
    for path in sorted(glob.glob(os.path.join(out_dir, f"{stem}-*.csv"))):
        name = os.path.basename(path)[len(stem) + 1:-4]
        with open(path, newline="", encoding="utf-8") as f:
            sheets[name] = list(csv.reader(f))
    return sheets


if __name__ == "__main__":
    book = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "out")
    for name, rows in recalculated(book, out).items():
        print(f"== {name}")
        for row in rows:
            print(row)
