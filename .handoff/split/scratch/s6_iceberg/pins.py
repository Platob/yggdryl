"""Every #[test] fn's numeric literals before (HEAD) and after (worktree), by name."""
import re, subprocess, collections, pathlib, sys
sys.path.insert(0, '/tmp/claude-0/-home-user-yggdryl/09ea5bac-ef2f-52ce-b6c3-cfd3a196ec17/scratchpad')
import s4_move as s4
root = pathlib.Path(sys.argv[1]); base = sys.argv[2] if len(sys.argv) > 2 else 'HEAD'
def fns(text):
    code = s4.code_only(text)
    out = {}
    for m in re.finditer(r'#\[test\]', code):
        fn = re.compile(r'\bfn\s+(\w+)').search(code, m.end())
        if not fn: continue
        brace = code.find('{', code.find('(', fn.end()))
        # skip to the body brace after the signature
        paren = code.find('(', fn.end()); close = s4.matching_brace(code, paren); brace = code.find('{', close)
        end = s4.matching_brace(code, brace)
        body = text[brace:end]
        nums = re.findall(r'(?<![\w.])\d[\d_]*(?:\.\d+)?(?:e\d+)?(?:_?[ui]\d+|_?usize|_?f64)?\b', s4.code_only(body))
        out.setdefault(fn.group(1), []).append(collections.Counter(nums))
    return out
files = subprocess.run(['git','-C',str(root),'ls-tree','-r','--name-only',base,'rust'],capture_output=True,text=True).stdout.split()
before = {}
for f in files:
    if f.endswith('.rs') and '/tests/' in f or f.startswith('rust/tests/'):
        if not f.endswith('.rs'): continue
        t = subprocess.run(['git','-C',str(root),'show',f'{base}:{f}'],capture_output=True,text=True).stdout
        for k, v in fns(t).items(): before.setdefault(k, []).extend(v)
after = {}
for p in root.glob('rust/**/tests/**/*.rs'):
    if 'target' in p.parts: continue
    for k, v in fns(p.read_text()).items(): after.setdefault(k, []).extend(v)
diff = []
for name, counts in before.items():
    if name not in after: continue
    a = after[name]
    for c in counts:
        if c not in a:
            diff.append(name)
print('compared', len([n for n in before if n in after]), 'test fns; literal multisets differing:', len(diff))
for n in diff[:20]: print(' ', n)
