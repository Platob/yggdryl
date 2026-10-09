

# ---------------------------------------------------------------------------
# Step 9: tooling
# ---------------------------------------------------------------------------

OWN_INTERNALS_RULE = '''

def own_internals(text: str) -> bool:
    """Whether a crate root declares its own `internals` outside the block this
    script writes - a crate whose root is the implementation, a medium split
    off the core: the block then holds the re-exports alone, written inside
    that module between the two markers, so the root has one `internals`."""
    outside = text
    if START in text:
        start = text.index(START)
        outside = text[:start] + text[text.index(END, start) + len(END):]
    return bool(DECLARATION.search(outside))
'''


def edit_tooling(root: pathlib.Path) -> None:
    """Where S4 (or an earlier S6 move) has not made them so, the two tools
    per crate, read off `s4_move.py` itself (the Parquet move's edits)."""
    path = root / "scripts/generate_internals.py"
    text = read(path)
    if "def crates()" not in text:
        source = read(SCRATCH / "s4_move.py")
        m = re.search(r"            tail = ('''def crates\(\).*?''')\n", source, re.S)
        start = text.find("def block() -> str:")
        end = text.find('if __name__ == "__main__":')
        if not m or start < 0 or end < 0:
            residue("scripts/generate_internals.py", "S4's per-crate generator was not found: make it per crate by hand")
        else:
            text = text[:start] + ast.literal_eval(m.group(1)) + text[end:]
    if "def own_internals(" not in text:
        text = replace_once(
            text,
            "def block() -> str:\n",
            OWN_INTERNALS_RULE.lstrip("\n") + "\n\ndef block(nested: bool = False) -> str:\n",
            "scripts/generate_internals.py",
        )
        text = replace_once(
            text,
            "    tests = SRC.parent.relative_to(ROOT).as_posix() + \"/tests/\"\n",
            "    if nested:\n"
            "        return START + \"\".join(f\"{line}\\n\" for line in body) + \"    \" + END\n"
            "    tests = SRC.parent.relative_to(ROOT).as_posix() + \"/tests/\"\n",
            "scripts/generate_internals.py",
        )
        text = replace_once(
            text,
            "        text = LIB.read_text(encoding=\"utf-8\")\n        generated = block()\n",
            "        text = LIB.read_text(encoding=\"utf-8\")\n"
            "        nested = own_internals(text)\n"
            "        if nested and START not in text:\n"
            "            print(f\"{LIB.relative_to(ROOT)} declares its own `internals`: put the markers at its end\", file=sys.stderr)\n"
            "            return 1\n"
            "        generated = block(nested)\n",
            "scripts/generate_internals.py",
        )
    write(path, text)
    path = root / "scripts/check_api_inventory.py"
    text = read(path)
    if 'glob("*/src")' not in text:
        text = replace_once(
            text,
            'for path in sorted((ROOT / "rust" / "src").rglob("*.rs")):',
            'for path in sorted(p for src in [ROOT / "rust" / "src", *sorted((ROOT / "rust").glob("*/src"))] for p in src.rglob("*.rs")):',
            "scripts/check_api_inventory.py",
        )
        text = replace_once(
            text,
            '        for path in crate.rglob("*.rs"):\n',
            "        # A leaf's surface holds what the core's macros write in it.\n"
            '        core = ROOT / "rust" / "src"\n'
            '        for path in [p for tree in {crate, core} for p in tree.rglob("*.rs")]:\n',
            "scripts/check_api_inventory.py",
        )
        write(path, text)
    result = subprocess.run([sys.executable, "-I", str(root / "scripts/generate_internals.py")],
                            capture_output=True, text=True, cwd=root)
    done(f"generate_internals.py: {(result.stdout or result.stderr).strip()}")
    if result.returncode:
        residue("scripts/generate_internals.py", f"the generator failed: {result.stderr.strip()[:200]}")
    # The docs runner compiles the pages' Rust blocks with features the core
    # no longer has, in whichever member S4 left it.
    rel = "scripts/check_docs_examples.py"
    runner = read(root / rel)
    m = re.search(r'"--features", "([^"]*)"', runner)
    if m:
        words = [w for w in m.group(1).split() if w not in ("s3", "yggdryl/s3")]
        runner = runner[:m.start(1)] + " ".join(words) + runner[m.end(1):]
        write(root / rel, runner)
    if '"--test", "docs_examples"' in runner and '"-p", "yggdryl-cli"' not in runner:
        residue(rel, "the Rust pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner "
                     "to `cli/tests/` (D13): a block naming `yggdryl_s3` compiles only there")


# ---------------------------------------------------------------------------
# Step 10: CI
# ---------------------------------------------------------------------------

S3_LEAF = 's3 = { package = "yggdryl-s3", jobs = ["object-interop", "azure-interop", "gcs-interop"] }\n'
NUMBERS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]
S3_EXCHANGES = ("object-interop", "azure-interop", "gcs-interop")


def edit_ci(root: pathlib.Path, linking: list[str]) -> None:
    rel = ".github/ci/rows.toml"
    text = read(root / rel)
    if S3_LEAF not in text:
        # After the leaves already listed, before the ones still commented out.
        lines = text.split("\n")
        start = lines.index("[leaves]") if "[leaves]" in lines else -1
        if start < 0:
            residue(rel, "no `[leaves]` table to list `s3` in")
        else:
            k = start + 1
            while k < len(lines) and lines[k].strip() and not lines[k].startswith("["):
                k += 1
            lines.insert(k, S3_LEAF.rstrip("\n"))
            text = "\n".join(lines)
    # The Iceberg crate links the backend under its `s3tables` feature.
    m = re.search(r'(?m)^(# )?iceberg = \{ package = "yggdryl-iceberg", after = \[([^\]]*)\]', text)
    if m and '"s3"' not in m.group(2):
        text = text[:m.end(2)] + ', "s3"' + text[m.end(2):]
    elif not m:
        residue(rel, "no `iceberg` leaf line to say it is after `s3`")
    for row in ("x-s3", "x-azure", "x-gcs"):
        text = text.replace(f'[rows.{row}]\nextends = ["interop"]\n', f'[rows.{row}]\nextends = ["crate-s3"]\n', 1)
    m = re.search(r"can break the build of all (\w+)", text)
    if m and m.group(1) in NUMBERS and "crate's target, read through the leaf's row" not in text:
        left = NUMBERS[NUMBERS.index(m.group(1)) - 3]
        text = edit_text(
            text, rel,
            ("# One `interop` target holds every exchange's Rust half, so a change to any\n"
             f"# of them can break the build of all {m.group(1)}.\n",
             "# One `interop` target holds the core's exchanges' Rust half, so a change to\n"
             f"# any of them can break the build of all {left}; a leaf's exchange is its own\n"
             "# crate's target, read through the leaf's row.\n"),
        )
    elif m and m.group(1) in NUMBERS:
        left = NUMBERS[NUMBERS.index(m.group(1)) - 3]
        text = text.replace(f"can break the build of all {m.group(1)};", f"can break the build of all {left};", 1)
    write(root / rel, text)
    for leaf in linking:
        leaf_after_s3(root, leaf)
    edit_planner_tests(root)
    edit(
        root, ".github/workflows/ci.yml",
        ("    # with each driver's own invocation - `--features s3` for S3, Azure and\n    # Google,",
         "    # with each driver's own invocation - `yggdryl-s3`'s `interop` target for\n    # S3, Azure and Google,"),
        ("    # with `s3` and Iceberg compiled; this job only reads it.\n",
         "    # with the object stores and Iceberg compiled; this job only reads it.\n"),
        ("        run: cargo test --locked --manifest-path rust/Cargo.toml --features s3 --test interop --no-run\n",
         "        run: cargo test --locked --manifest-path rust/s3/Cargo.toml --test interop --no-run\n"),
        ("      # The driver runs the `interop` target with `--features s3`, which the\n",
         "      # The driver runs `yggdryl-s3`'s `interop` target, which the\n"),
        ("  # generates an integration target and compiles it against `parquet iceberg\n  # s3 s3tables http3`,",
         "  # generates an integration target and compiles it against `parquet iceberg\n  # s3tables http3`,"),
    )


def edit_planner_tests(root: pathlib.Path) -> None:
    rel = "scripts/tests/test_ci_plan.py"
    text = read(root / rel)
    text = text.replace(
        'LEAF_LINE = re.compile(r"^(?:# )?([a-z]+) = \\{ package = .*\\}$", re.MULTILINE)',
        'LEAF_LINE = re.compile(r"^(?:# )?([a-z][a-z0-9]*) = \\{ package = .*\\}$", re.MULTILINE)', 1)
    if '"s3": {"object-interop", "azure-interop", "gcs-interop"},' not in text:
        text = edit_text(
            text, rel,
            ('        "iceberg": {"pyiceberg-interop", "spark-interop", "iceberg-msrv"},\n    }\n',
             '        "iceberg": {"pyiceberg-interop", "spark-interop", "iceberg-msrv"},\n'
             '        "s3": {"object-interop", "azure-interop", "gcs-interop"},\n    }\n'),
            ("    # What no leaf-only change runs.\n",
             "    # What no leaf-only change runs but the leaf's own exchanges.\n"),
            ("                self.assertFalse(self.NEVER & result.jobs)\n",
             "                self.assertFalse((self.NEVER - exchanges) & result.jobs)\n"),
        )
    m = re.search(
        r'(    def test_an_exchange_half_runs_every_exchange\(self\) -> None:\n'
        r'        jobs = planned\(\["rust/tests/interop/zip.rs"\]\).jobs\n'
        r'        self.assertTrue\(\{)([^}]*)(\} <= jobs\)\n)', text)
    if not m:
        residue(rel, "the exchange-half test was not found: say the object stores' exchanges are not the core's by hand")
    elif "yggdryl-s3" not in text:
        jobs = [j for j in re.findall(r'"([\w-]+)"', m.group(2)) if j not in S3_EXCHANGES]
        listed = ", ".join(f'"{j}"' for j in jobs)
        body = (m.group(1) + listed + m.group(3)
                + "        # The object stores' exchanges' Rust half is `yggdryl-s3`'s own target.\n"
                + "        for job in (\"object-interop\", \"azure-interop\", \"gcs-interop\"):\n"
                + "            self.assertNotIn(job, jobs)\n")
        text = text[:m.start()] + body + text[m.end():]
    write(root / rel, text)
