# Over the parsed messages of the snapshot (ulbridge[N]:i rows), in instant order: for each message of
# a chained kind, which live messages of its kind (and side where sided) it shares an identifier VALUE
# with, by type - against the one it joins today (same cross code, or a chain-identity name).
import json, re, sys, collections
S = sys.argv[1]
rows = collections.defaultdict(dict)
for line in open(S, encoding="utf-8"):
    m = re.match(r"ulbridge\[(\d+)\]:(\d+)\.field\.(\w+)\t(.*)", line.rstrip("\n"))
    if m: rows[(int(m.group(1)), int(m.group(2)))][m.group(3)] = m.group(4)
def j(v, d=None):
    try: return json.loads(v)
    except Exception: return d
CHAIN = {"orderid","clordid","secondaryorderid","secondaryclordid","quoteid","secondaryquoteid","tradeid","secondarytradeid","secondaryfirmtradeid","tradereportid"}
SIDED = {"ORDR","EXEC"}
msgs = []
for key in sorted(rows):
    r = rows[key]
    kind = j(r.get("marketdatakind",'""'), "")
    if kind in ("", "UKNW", "SESS"): continue
    msgs.append(dict(at=f"ulbridge[{key[0]:03}]:{key[1]}", mt=j(r.get("msgtype",'""'),""), kind=kind,
        side=j(r.get("side",'"UKNW"'),"UKNW") if kind in SIDED else "-", cc=j(r.get("crosscode",'""'),""),
        state=j(r.get("state",'""'),""), tu=j(r.get("transunix",'""'),""), ids=j(r.get("identifiers","{}"),{}) or {},
        sec=j(r.get("securityids","{}"),{}) or {}, uuid=j(r.get("uuid",'""'),"")))
msgs.sort(key=lambda m: m["tu"])
print(len(msgs), "chained-kind messages")
alive = collections.defaultdict(list)
kinds = collections.Counter(); cands = []
for m in msgs:
    k = (m["kind"], m["side"])
    same = [a for a in alive[k] if a["cc"] == m["cc"]]
    byname = [a for a in alive[k] if a["cc"] != m["cc"] and any(t.split(":")[-1] in CHAIN and v in a["ids"].values() for t, v in m["ids"].items())]
    byany = [a for a in alive[k] if a["cc"] != m["cc"] and a not in byname and any(v in a["ids"].values() for t, v in m["ids"].items())]
    kinds[(m["kind"], bool(same), bool(byname), bool(byany))] += 1
    if byany or len(byname) > 1 or (byname and same):
        cands.append((m, same, byname, byany))
    # settle
    live = m["state"] not in {"FILLED","CANCELED","EXPIRED","REJECTED","DONE_FOR_DAY","TRADE","UNKNOWN"}
    alive[k] = [a for a in alive[k] if a["cc"] != m["cc"]]
    if live: alive[k].append(m)
print("(kind, joins own code, joins by chain name, would join by another type):")
for k, v in sorted(kinds.items()): print("  ", k, v)
print("\ncandidates (shares a non-chain identifier value with a live message of another code, or cites two):")
for m, same, byname, byany in cands:
    common = lambda a: sorted({f"{t}={v}" for t, v in m["ids"].items() if v in a["ids"].values()})
    print(f"  {m['at']} 35={m['mt']} {m['kind']}/{m['side']} {m['cc']} {m['state']} own={len(same)} byname={[(a['at'],a['cc'],common(a)) for a in byname]} byany={[(a['at'],a['cc'],common(a)) for a in byany]}")
