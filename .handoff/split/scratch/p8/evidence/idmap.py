import json, glob, sys
rows = []
for f in sorted(glob.glob(sys.argv[1] + "/*.json")):
    d = json.load(open(f))
    items = d if isinstance(d, list) else d.get("fields", [d])
    for it in items:
        if not isinstance(it, dict): continue
        m = it.get("metadata") or {}
        if "FIX:idmap" in m or "FIX:parents" in m:
            rows.append((int(it.get("tag", 0)), it.get("name"), m.get("FIX:idmap"), m.get("FIX:parents")))
for r in sorted(rows): print(r)
