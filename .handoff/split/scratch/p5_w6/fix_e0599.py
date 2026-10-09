#!/usr/bin/env python3
"""Re-spell, from the compiler's own list, a graph trait call made on a
`FixMsg` onto the market message it holds.

D37 deletes `FixMsg`'s `Element`/`Event`/`Market`/`Operation` impls, so
`held.get_side()` on a `FixMsg` is `error[E0599]: no method named `get_side`
found for ... `FixMsg``. Feed this the short-format log of

    cargo check -p yggdryl --all-targets --keep-going --message-format=short

and it inserts `message().` (a reading) or `message_mut().` (a setter, an
insert, a removal, a derivation, a finalize) at each reported column, right
to left within a line, each insertion asserting the method name stands
there. Re-run the check and this script until it reports nothing: the
compiler reports what the phase it reached saw. Calls taking `self` by
value (`with_previous`, `merge_with`, `restating`) and a FixMsg passed where
a trait is required (E0277) are listed, never edited.

usage: fix_e0599.py CHECK_LOG [--dry-run]
"""
import collections
import re
import sys
from pathlib import Path

LINE = re.compile(
    r"^(?P<file>[^:\s]+\.rs):(?P<line>\d+):(?P<col>\d+): error\[E0599\]: no method named "
    r"`(?P<method>\w+)` found for (?:struct|reference|mutable reference) `(?:&(?:mut )?)?(?:yggdryl::)?FixMsg`"
)
OTHER = re.compile(r"^(?P<where>[^:\s]+\.rs:\d+:\d+): error\[E0277\]: .*FixMsg.*")
CONSUMING = {"with_previous", "merge_with", "restating"}
MUTATING_PREFIX = ("set_", "insert_", "remove_")
MUTATING = {"derive_securityid", "finalize", "fill_market", "sync_cross", "follow_identity", "note_conflict"}


def main(log, dry):
    edits = collections.defaultdict(list)
    manual = []
    for text in Path(log).read_text().splitlines():
        found = LINE.match(text)
        if not found:
            other = OTHER.match(text)
            if other:
                manual.append(f"{other.group('where')}: a FixMsg where a graph trait is required")
            continue
        method = found.group("method")
        where = f"{found.group('file')}:{found.group('line')}:{found.group('col')}"
        if method in CONSUMING:
            manual.append(f"{where}: {method} takes the message by value - into_message() first")
            continue
        door = "message_mut()." if method.startswith(MUTATING_PREFIX) or method in MUTATING else "message()."
        edits[found.group("file")].append((int(found.group("line")), int(found.group("col")), method, door))
    applied = 0
    for file, sites in edits.items():
        path = Path(file)
        lines = path.read_text().split("\n")
        for line, col, method, door in sorted(set(sites), reverse=True):
            text = lines[line - 1]
            at = col - 1
            if not text[at:].startswith(method):
                manual.append(f"{file}:{line}:{col}: expected `{method}` at the column")
                continue
            lines[line - 1] = text[:at] + door + text[at:]
            applied += 1
        if not dry:
            path.write_text("\n".join(lines))
    print(f"{applied} calls re-spelled in {len(edits)} files")
    for item in sorted(set(manual)):
        print("manual:", item)


if __name__ == "__main__":
    main(sys.argv[1], "--dry-run" in sys.argv[2:])
