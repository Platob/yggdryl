"""P12 evidence: walk the ULBridge capture and print every report touching fills."""
import pathlib, sys
from yggdryl import DataType, IOBase, TextOptions, Url
from yggdryl.fix import ULBRIDGE_ROWHEADER, FixCodec, FixRegistry

REPO = pathlib.Path("/home/user/yggdryl")
LOG = REPO / "rust/tests/support/ulbridge.log"
CLOCK = DataType('datetime64(ns,"UTC")').scalar(1_704_190_530_000_000_000)

options = TextOptions()
options.rowheader = ULBRIDGE_ROWHEADER
options.timezone = "Europe/Zurich"
options.start_rownum = 1
source = IOBase.from_bytes(LOG.read_bytes())
source.media_type = Url("file:///ulbridge.log").media_type
lines = list(source.read_text_lines(options=options))
registry = FixRegistry.from_handle(REPO / "config" / "fix")
codec = FixCodec(registry, default_sending_time=CLOCK, exclude_msgtypes=[], threads=1,
                 capture_names=list(options.capture_names))
messages = list(codec.parse_text_lines(lines))
mode = sys.argv[1] if len(sys.argv) > 1 else "walked"
if mode == "walked":
    seq = list(codec.lifecycle(messages))
elif mode == "every":
    seq = list(codec.with_dedup_window_ms(None).lifecycle(messages))
else:
    seq = messages
print(f"lines={len(lines)} parsed={len(messages)} {mode}={len(seq)}")

def A(m,n):
    v=getattr(m,n)
    return v() if callable(v) else v

def s(v):
    if v is None:
        return "-"
    try:
        return str(v.as_py()) if hasattr(v, "as_py") else str(v)
    except Exception:
        return str(v)

def tag(m, t):
    v = m.get_by_tag(t)
    return s(v)

def ident(m, key):
    ids = A(m,"identifiers")
    try:
        v = ids.get(key)
    except Exception:
        v = None
    return "-" if v is None else str(v)

rows = []
for i, m in enumerate(seq):
    kind = str(A(m,"marketdatakind"))
    if kind not in ("ORDR", "EXEC", "QUOT", "TRAD") and "order" not in kind.lower() and "exec" not in kind.lower():
        pass
    rows.append((i, m))
for i, m in rows:
    if str(A(m,'marketdatakind')) in ('SESS',) and tag(m,17)=='-' : continue
    if str(A(m,'marketdatakind'))=='UKNW' and tag(m,17)=='-' and tag(m,11)=='-': continue
    print("\t".join([
        str(i), A(m,"header").msgtype, str(A(m,"marketdatakind")), A(m,"crosscode"),
        "t=%s" % A(m,"transunix"),
        "orderid=" + tag(m, 37), "clordid=" + tag(m, 11), "execid17=" + tag(m, 17),
        "id.execid=" + ident(m, "execid"),
        "150=" + tag(m, 150), "39=" + tag(m, 39),
        "state=" + str(A(m,"state")),
        "38=" + tag(m, 38), "ordqty=" + s(A(m,"ordqty")),
        "32=" + tag(m, 32), "lastqty=" + s(A(m,"lastqty")), "31=" + tag(m, 31), "lastpx=" + s(A(m,"lastpx")),
        "14=" + tag(m, 14), "cumqty=" + s(A(m,"cumqty")),
        "151=" + tag(m, 151), "leavesqty=" + s(A(m,"leavesqty")),
        "6=" + tag(m, 6), "avgpx=" + s(A(m,"avgpx")),
        "side=" + str(A(m,"side")),
        "uuid=" + s(A(m,"uuid"))[-8:], "prev=" + (s(A(m,"prevuuid"))[-8:] if A(m,"prevuuid") is not None else "-"), "line=" + tag(m, 65001) if False else "",
    ]))
