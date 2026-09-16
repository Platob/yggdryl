"""Run documentation examples inside one long-lived interpreter.

A Python example spends a few milliseconds on what it demonstrates and the
better part of a second importing PyArrow and the extension behind it. Running
one process per example paid that import once per block; this worker pays it
once per core, then executes block after block in a fresh namespace.

Protocol, one JSON object per line in each direction:

* in  - ``{"path": "/tmp/.../types_index_0.py"}``
* out - ``{"ok": true}`` or ``{"ok": false, "detail": "..."}``

The requests arrive on stdin and the answers leave on the descriptor stdout
started as; the example's own stdout and stderr are pointed at a capture file
the parent named, so a block that prints - or an extension that writes straight
to the descriptor - cannot corrupt the protocol. The same file is what the
parent reads when a block takes the whole worker down with it.
"""

from __future__ import annotations

import json
import os
import pathlib
import sys
import traceback


def main() -> int:
    capture_path = sys.argv[1]
    warm = sys.argv[2:]

    # The protocol keeps the descriptor stdout started as; fd 1 and fd 2 become
    # the capture, so anything the example writes - at any level - lands there.
    protocol = os.fdopen(os.dup(1), "w", buffering=1, encoding="utf-8")
    capture = open(capture_path, "w+b")  # noqa: SIM115 - held for the process
    os.dup2(capture.fileno(), 1)
    os.dup2(capture.fileno(), 2)

    # The imports every block pays for, paid once. A warm name that will not
    # import is left to the block to report against its own code.
    for name in warm:
        try:
            __import__(name)
        except Exception:  # noqa: BLE001 - reported by the first block instead
            pass

    home = os.getcwd()
    namespaces: list[dict[str, object]] = []
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        path = pathlib.Path(json.loads(line)["path"])
        capture.seek(0)
        capture.truncate()
        # A bare namespace: `exec` fills in `__builtins__` itself, and the
        # block sees nothing the block before it left behind.
        namespace: dict[str, object] = {"__name__": "__main__", "__file__": str(path)}
        try:
            exec(compile(path.read_text(encoding="utf-8"), str(path), "exec"), namespace)  # noqa: S102
            answer = {"ok": True}
        except SystemExit as exit_code:
            code = exit_code.code or 0
            answer = {"ok": code == 0, "detail": f"sys.exit({code})"}
        except BaseException:  # noqa: BLE001 - every failure is this block's
            answer = {"ok": False, "detail": traceback.format_exc()}
        if not answer["ok"]:
            capture.flush()
            capture.seek(0)
            printed = capture.read().decode("utf-8", "replace").strip()
            if printed:
                answer["detail"] = f"{printed}\n{answer['detail']}"
        # A block that wandered leaves the next one where it started.
        os.chdir(home)
        # The namespace is never emptied. What a block installs process-wide -
        # a logging handler on `yggdryl`, say - outlives the block, and the
        # function it installed reads its own module globals: empty that dict
        # and the handler raises for every block after it.
        namespaces.append(namespace)
        protocol.write(json.dumps(answer) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
