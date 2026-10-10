import json, re, sys, collections
S = sys.argv[1]
rows = collections.defaultdict(dict)
for line in open(S, encoding="utf-8"):
    if not line.startswith("lifecycle["): continue
    key, _, val = line.rstrip("\n").partition("\t")
    m = re.match(r"lifecycle\[(\d+)\]\.(.*)", key)
    n, field = int(m.group(1)), m.group(2)
    rows[n][field] = val
SIDED = {"ORDR", "EXEC"}
LIVE_NOT = {"FILLED", "CANCELED", "CANCELLED", "EXPIRED", "REJECTED", "DONE_FOR_DAY", "STOPPED", "SUSPENDED", "UNKNOWN", "TRADE"}
def j(v):
    try: return json.loads(v)
    except Exception: return v
alive = {}  # (kind, side) -> list of (n, ids dict)
out = []
for n in sorted(rows):
    r = rows[n]
    kind = j(r.get("field.marketdatakind", '""'))
    side = j(r.get("field.side", '"UKNW"')) if kind in SIDED else "-"
    ids = j(r.get("field.identifiers", "{}")) or {}
    state = j(r.get("field.state", '""'))
    prev = r.get("field.prevuuid")
    cc = j(r.get("field.crosscode", '""'))
    mt = j(r.get("field.msgtype", '""'))
    tu = j(r.get("field.transunix", "0"))
    shared = []
    for (k, s), lives in alive.items():
        if k != kind or s != side: continue
        for (m, mids, mcc) in lives:
            common = sorted({(t, v) for t, v in ids.items() if v in mids.values()})
            same_type = sorted(t for t, v in ids.items() if mids.get(t) == v)
            if common: shared.append((m, mcc, same_type, [f"{t}={v}" for t, v in common if t not in same_type]))
    out.append((n, mt, kind, side, cc, state, bool(prev), tu, shared))
    # settle: alive iff state live (approximation: not in LIVE_NOT and not EXPIRED)
    live = state not in LIVE_NOT and kind not in ("UKNW", "SESS", "")
    key = (kind, side)
    alive.setdefault(key, [])
    # retire same crosscode chain
    alive[key] = [x for x in alive[key] if x[2] != cc]
    if live: alive[key].append((n, ids, cc))
for n, mt, kind, side, cc, state, prev, tu, shared in out:
    flag = ""
    if shared and not prev: flag = "  <-- shares, did not join"
    if prev and not shared: flag = "  <-- joined by cross code alone"
    print(f"[{n:03}] 35={mt:<3} {kind:<4} {side:<4} {cc:<40} {state:<16} prev={int(prev)} tu={tu} shared={shared}{flag}")
