"""Every #[test] fn of the before tree against the after tree: names as a
multiset, and per test the integer and string literals of its body, the
install line and the path spellings normalized."""
import collections, pathlib, re, sys
sys.path.insert(0, "/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad")
import s4_move as s4

before, after = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
dirs = ["rust/tests", "rust/s3/tests", "rust/parquet/tests", "rust/avro/tests"]

def tests(root):
    out = collections.defaultdict(list)
    for d in dirs:
        base = root / d
        if not base.exists():
            continue
        for p in sorted(base.rglob("*.rs")):
            text = p.read_text()
            code = s4.code_only(text)
            for m in re.finditer(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([a-z]+\))? )?fn (\w+)\s*\(", text):
                i = code.index("{", m.end())
                depth, j = 0, i
                while True:
                    c = code[j]
                    if c == "{": depth += 1
                    elif c == "}":
                        depth -= 1
                        if depth == 0: break
                    j += 1
                body = text[i:j + 1]
                body = re.sub(r"\n[ \t]*crate::install::installed\(\);", "", body)
                body = re.sub(r'\n[ \t]*yggdryl_\w+::install\(\)\.expect\("[^"]*"\);', "", body)
                out[m.group(1)].append((str(p.relative_to(root)), body))
    return out

def numbers(body):
    return re.findall(r"(?<![\w.])\d[\d_]*(?:\.\d+)?(?![\w])", body)

b, a = tests(before), tests(after)
bn = collections.Counter({k: len(v) for k, v in b.items()})
an = collections.Counter({k: len(v) for k, v in a.items()})
print("before", sum(bn.values()), "after", sum(an.values()))
print("lost", sorted((bn - an).elements()))
print("added", sorted((an - bn).elements()))
moved = changed_num = changed_body = 0
for name, olds in b.items():
    news = a.get(name, [])
    if len(olds) != 1 or len(news) != 1:
        # ambiguous names: compare as sorted lists of number tuples
        if sorted(tuple(numbers(x[1])) for x in olds) != sorted(tuple(numbers(x[1])) for x in news) and news:
            print("NUM DIFF (multi)", name)
            changed_num += 1
        continue
    (op, ob), (np_, nb) = olds[0], news[0]
    if op != np_:
        moved += 1
    if numbers(ob) != numbers(nb):
        print("NUM DIFF", name, op, "->", np_)
        changed_num += 1
    if ob != nb:
        changed_body += 1
print("moved", moved, "number diffs", changed_num, "bodies changed (paths/strings)", changed_body)

if len(sys.argv) > 3:
    import difflib
    for name, olds in b.items():
        news = a.get(name, [])
        if len(olds) == 1 and len(news) == 1 and olds[0][1] != news[0][1]:
            d = [l for l in difflib.unified_diff(olds[0][1].split("\n"), news[0][1].split("\n"), lineterm="", n=0) if l[:1] in "+-" and not l.startswith(("+++", "---"))]
            print("==", name, olds[0][0], "->", news[0][0])
            for l in d:
                print("  ", l.strip()[:150])
