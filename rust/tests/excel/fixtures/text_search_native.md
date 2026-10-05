# SEARCH native observations

`text_search_native.json` retains all 176 cases from four successful, cleanup-complete
Excel 16.0 build 20430 runs, both date systems and explicit source text compatibility
versions 1/2. Its 352 comparisons are native before-SaveAs/after-SaveAs versus the
saved XML cache; they are not Rust equality claims. Input/output hashes, raw COM,
saved formulas/cache, and exact version provenance remain in the fixture.

The fixture separates wildcard transitions from position counting. `?` consumes
one UTF-16 unit in both observed versions: `SEARCH("??", "😀")` returns 1 in both.
Version 2 exposes only Unicode-scalar start positions; `SEARCH("?Z", "A😀Z")`
returns #VALUE! there but 3 under version 1. `SEARCH("??Z", "A😀Z")` returns 2 in
both, while literal Z returns 4/3. A trailing unescaped tilde contributes no
literal suffix; bare `~` returns 1. These rules belong to the existing wildcard
parser/transition owner with explicit SEARCH intake, not a second matcher.

The portable adapter initially computes ASCII case-insensitive comparisons and
uncased Unicode. The 32 observations involving non-ASCII cased text remain explicit
held-cache controls; in particular native sigma/final-sigma equivalence differs
from the existing generic lowercase comparator. No locale is inferred from the
Excel UI or a filename, and no host collation is imported into Rust.

Microsoft documents SEARCH's wildcard syntax, default start, #VALUE! bounds and
version-2 surrogate position counting at
https://support.microsoft.com/en-us/excel/functions/search-function.
The observed UTF-16 wildcard edge above is retained separately from that prose.

The 48 discriminating follow-up observations prove that version 2 skips a
low-surrogate candidate and continues to a later scalar start: `?Z` in
`A😀Z aZ` returns 5, and in `😀Z bZ` returns 4. `*?Z` still returns 1.
Escaped ordinary characters (`~a`, `a~b`) and a trailing tilde retain their
observed grammar. The test runs primary and follow-up corpora separately because
the source workbooks reuse host coordinates; no observation overwrites another.
