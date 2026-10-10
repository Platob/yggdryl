import sys, re, pathlib, collections
S = pathlib.Path("/tmp/s")
sys.path.insert(0, str(S))
import s4_move as s4
root = pathlib.Path(sys.argv[1])
src = root / "rust/src/s3"
leaves = collections.defaultdict(set)
for p in sorted(src.rglob("*.rs")):
    rel = str(p.relative_to(root))
    text = p.read_text()
    code = s4.code_only(text)
    for m in s4.USE_START.finditer(code):
        start = m.end(); depth=0; i=start
        while i < len(code):
            c = code[i]
            if c=='{': depth+=1
            elif c=='}': depth-=1
            elif c==';' and depth==0: break
            i+=1
        tree = re.sub(r"\s+"," ",text[start:i]).strip()
        try:
            for path, alias, kind in s4.parse_use(tree):
                if path and path[0]=="crate" and (len(path)<2 or path[1]!="s3"):
                    leaves["::".join(path[1:]) + ("::self" if kind=="self" else "")].add(rel.split("rust/src/")[1])
        except s4.UseParseError:
            print("PARSE", rel, tree)
    for m in re.finditer(r"(?<![\w$:])crate((?:::(?:r#)?[A-Za-z_]\w*)+)", code):
        segs = [s for s in m.group(1).split("::") if s]
        if segs and segs[0]=="s3": continue
        # skip those within use statements: rough check
        line_start = code.rfind("\n",0,m.start())+1
        if re.match(r"\s*(pub(\([^)]*\))?\s+)?use\b", code[line_start:m.start()+1]) : continue
        leaves["INLINE "+"::".join(segs)].add(rel.split("rust/src/")[1])
for k in sorted(leaves):
    print(k, "|", ", ".join(sorted(leaves[k])))
