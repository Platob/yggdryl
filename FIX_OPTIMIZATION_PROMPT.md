# Optimizing the FIX read path, and making an entry text

Two pieces of work on `claude/fix-text-line-parsing-d89lw6`, which is green and
merged-ready but measurably slower than the commit it began from. The first
piece is to give that speed back. The second is a design change to `TextEntry`
that the first piece must not be tangled with.

**Order.** The regression first, because it is measured, attributed and has
vetted fixes waiting. The `TextEntry` change second, because it moves a decided
semantic and needs its own argument. Do not interleave them: a shared commit
would make it impossible to say which one moved a number.

## Outcome

- `fix/pipeline/parse_lines` and `text_read` back to within noise of `1ccca72`,
  or a written statement of what is irreducible and why.
- `TextEntry` answers text, not bytes, with a stated rule for what happens to
  bytes that are not text - and the rule written down before the code.
- Every number in `docs/fix/arrow.md` regenerated from a clean run.
- What is deleted is named. What is decided is written in `DECISIONS.md`.

## What is measured, and what it means

Measured on one Linux x86_64 container, Intel Xeon @ 2.80 GHz, 4 cores,
15 GiB, rustc 1.94.1, release with thin LTO and one codegen unit. Base is
`1ccca72`, the merge-base; head is `30f6626`. Same corpus, same harness,
same container - verified, not assumed.

| stage | `1ccca72` | `30f6626` | change |
| --- | ---: | ---: | ---: |
| `fix/pipeline/text_read` | 138.40 ms | 160.33 ms | +15.8% |
| `fix/pipeline/parse_text_arrow_reader` | 1.6683 s | 1.8168 s | +8.9% |
| `fix/pipeline/parse_lines` | 658.49 ms | 859.15 ms | +30.5% |

All at p = 0.00. The noise floor on this container is ~1.5%, established by
running the identical binary twice. An earlier run measured +14% on
`parse_lines` while a rustdoc build and an 18 GB delete ran alongside it; that
number was contamination, and the clean re-run is what the table holds. **Do
not benchmark while anything else runs on this box.**

Three things were checked before trusting the comparison, and all three hold:
the corpus fixture `rust/tests/fix/ulbridge.log` is byte-identical at both
commits; the benchmark harness is byte-identical; and the only change to
`rust/benchmarks/fix/pipeline.rs` is additive - it appends
`parse_text_lines_pluginid` after the three stages above. `fa6d7e2`, whose
subject reads "Regenerate the pipeline benchmark over the bridge's log",
touches `docs/fix/arrow.md` only.

## Read first

- `AGENTS.md` in full. Gate 1 is at §2 and includes MSRV checks that are easy
  to skip: `cargo +1.85.0 check` at `--all-targets`, at
  `--no-default-features --lib`, at `--no-default-features --features object
  --lib`, and `cargo +1.94.0 check --all-targets --features iceberg`. All four
  pass at `30f6626` - keep them passing.
- `DECISIONS.md` at the repository root: nine decisions and three amendments
  settled while this branch was built. The second amendment to decision 1 is
  the one that matters most here - it explains why `for_line` ranks candidates
  rather than stopping at the first, and a naive short-circuit is exactly the
  fix it rejected.
- `rust/src/mime_type/line.rs`, the whole file. It is where the regression is.
- `rust/tests/fix/equivalence.snapshot`: 264 messages over 8 groups, 11,775
  lines, pinning `.type`, `.digest`, `.wire`, every `.entry[i]` and every
  `.field.<column>`. **Not one line of it may move.** It is regenerated only
  under `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`, and regenerating it to make a test
  pass is the one thing that would make this work unreviewable.

# Part 1 - the regression

## Where it comes from

`text_read` executes no FIX code, so its +15.8% is the scanner alone. An exact
byte-visit simulation of the real corpus predicts +19.8 ms against a measured
+21.9 ms - about 90% of it accounted for by one function.

**`LineSeparator::for_line` (`rust/src/mime_type/line.rs:259`) lost a fast
path.** At `1ccca72` it took a `numeric` flag, and for a non-numeric line - every
bridge row in this corpus - it returned straight out of `ullink_separator`: a
single `memchr2(0x01, b'|')` that stops at the first hit. 57 of the 71 framed
corpus lines took that path, at about 20 byte-visits each. That path is gone.
Every line now evaluates all four candidates through `separates()`.

On one representative bridge line (3,799-byte body, 3,538-byte frame tail) the
cost went from **22 byte-visits to 24,856**. Over the 113-line corpus, per pass:

| candidate | byte-visits at `30f6626` | at `1ccca72` |
| --- | ---: | ---: |
| `Marker` (four SOH spellings) | 613,961 SIMD | one pinned spelling |
| `Byte(0x01)` | 153,381 SIMD | - |
| `Whitespace` | 272,404 scalar | not a candidate at all |
| `Byte(b'|')` | 3,895 SIMD + 561 scalar | one `memchr2`, both bytes |
| total | 773,669 SIMD + 272,967 scalar | 94,772 SIMD + 0 scalar |

Two corrections to what an earlier reading of this assumed, both load-bearing:

1. **Absence is the expensive case, not the cheap one.** `separates()` does exit
   on its first iteration when the byte is absent - but that iteration calls
   `segment()` → `find()`, which scans the whole tail. For `Marker` that is
   four full-tail `memmem::find` calls, each building a fresh `Finder`. 79% of
   all SIMD byte traffic per pass is spent proving `^A`, `\x01`, `<SOH>` and
   `{SOH}` are *not* in a line. The corpus carries a marker on 2 lines of 113.
2. **Whitespace is the largest weighted term.** Its arm returned `None` at base,
   so it never ran during separator selection. It now runs a scalar,
   unvectorised `position(is_ascii_whitespace)` walk - 272,404 of the 272,967
   scalar byte-visits per pass, roughly 87% of weighted cycles.

`for_line` runs twice per line in `text_read` (through `payload()` and through
`inspect()`), and about 5.7 times per line on the `parse_lines` path, because
`entry_spans` recurses into every nested value.

## What is already attributed

The branch measured itself mid-flight, which collapses the search. `fa6d7e2`
re-published the pipeline table after the branch's first five commits:
`parse_lines` 710 ms against a 714 ms base - **flat**. So essentially all of the
+201 ms lands after `fa6d7e2`, and of the commits after it only these six touch
code `parse_lines` executes:

`a169bd6`, `760aade`, `0c906a1`, `fd76e05`, `d6537d2`, `aac6f0d`

`760aade` is the commit that deleted the `for_line` fast path - not `a169bd6`,
which is where an earlier reading put it. `a169bd6` is separately responsible
for `entry_spans` calling `locate_frame` at all; at base it was five lines over
`pairs(line)` with no framing.

On the FIX side, one top-level pair now costs **11 `Arc` refcount increments**
(and 11 matching decrements) between the page and the built `FixMsg`, across six
sites in `rust/src/media/text/entry.rs`, `rust/src/fix/codec.rs` and
`rust/src/fix/build.rs`. At `1ccca72`
that pair cost zero copies: `split_fix_with` and `ullink_pairs` carried
`(&[u8], &[u8])` slices end to end, and the only copy anywhere was the two
`SmolStr` constructions in `FixEntry::new` at record time - where 96.2% of this
corpus's keys and 86.2% of its values are ≤22 bytes and so were inline memcpys
with no allocation. The change is not `SmolStr` → `TextBytes`; it is **borrowed
slice → refcounted range**.

## Vetted safe - start here

Each was read adversarially by a separate reviewer instructed to refute it, with
the decisions file and the snapshot in hand. These survived.

Sites are under `rust/src/`: `line.rs` is `mime_type/line.rs`, `entry.rs` is
`media/text/entry.rs`, and `codec.rs`/`build.rs` are `fix/`.

| site | change | expected |
| --- | --- | --- |
| `line.rs:259` `for_line` | Walk the tail once over the union of the candidates' first bytes, pruning any candidate that cannot beat the best-so-far position | 773,669+272,967 byte-visits → 4,906+3,397; below the `1ccca72` level |
| `line.rs:291` `separates` | Carry the already-computed segment forward instead of recomputing it each iteration | scalar byte-visits −43%; lands on its own |
| `line.rs:372` `find`, `Marker` arm | One combined first-occurrence scan over the four distinct opener bytes instead of four independent `memmem::find` | Marker term 613,961 → ~150-300k |
| `line.rs:749` `span`/`segment_span` | Stop computing `PairSpan::nested` on `inspect`'s walk | −42,487 byte-visits/pass; small, land it with the others |
| `entry.rs:387` `read_entries_at` | Make the nested descent opt-in (depth limit, or a shallow constructor) so FIX's four call sites ask for depth 1 | ~48 increments, ~9 `Vec`s and ~9 re-entries into `entry_spans`/`for_line` per framed line |
| `build.rs:1414` `arrived` | Take `&mut self` and `self.arrival.take()` instead of cloning two `TextBytes` out | 2 of the 11 atomics per pair |
| `codec.rs:1107` | Iterate `kept.into_iter()` and move the judged key instead of `key.clone()` | 1 of 11 |
| `codec.rs:1877-1885` `arrival`/`arrivals` | Borrowed form for the bridge path | 2 of 11, plus one ~3.6 KB `Vec` per line |
| `codec.rs:2057-2073` `split_members` | Return offsets into the value, not `Vec<TextBytes>` | ~380k atomics over the corpus |

Take `line.rs:259` and `line.rs:291` first. They are most of `text_read`, and
`text_read` is the cleanest signal you have.

## Refuted - do not retry as specified

These were proposed and broken with concrete counterexamples. The *ideas* behind
the first two are sound; the specs were not, and the difference is the whole
point.

- **Restoring a short-circuit in `for_line` by probing each candidate's first
  occurrence.** Broken by `8=FIX.4.4<x|35=D|58=a^A10=123`: a first-byte probe
  finds a marker opener that does not begin a marker. A first *occurrence* is
  not a first *separation*. The pruning version in the table above is the one
  that survives, because it prunes on position without trusting a raw byte hit.
- **The Marker combined scan as first written.** The corrected version is
  provably equivalent, but the spec omitted the correction, and its `find_back`
  half is a pessimization. Re-derive it; do not copy it.
- **Hoisting the separator decision out of the per-line loop.** The premise
  inverts on real input - `ulbridge.log` lines 7, 9 and 57 carry nested values
  whose frame differs from their parent's.
- **Sharing one page across a `parse_lines` batch.** `fix/codec.rs` reaches the
  raw `Arc<Vec<u8>>` behind a `TextBytes`; two lines in one page change what a
  data field's stated length re-slices. Counterexample verified by byte
  arithmetic: `8=FIX.4.4|35=D|95=21|96=ABC|10=...` followed by any second line.
- **Collapsing the `Vec` stages between the entry tree and the builder.** The
  reader's answers do not move, but a pin named in decision 7 breaks on a
  concrete input. Worth revisiting only with that pin re-derived first.

## How to measure

Three failures this session are worth not repeating.

**The baseline is on disk. Rescue it before it dies.** `1ccca72` is stored as
the criterion baseline `prebranch`, holding exactly 138399817.61 /
1668250167.23 / 658488607.79 ns. It does not need re-measuring - but it lives
under `target/`, where any `cargo clean` or a disk-pressure `rm -rf` destroys
it. criterion honours `CRITERION_HOME`; move the store outside `target/` first,
before running anything.

**Never run the fix bench unnamed.** criterion rolls `new/` into `base/` on
every run, so a second unnamed run overwrites the baseline you wanted. This
session lost the original before/after exactly that way and had to re-measure
`1ccca72` from scratch. Name every run after the commit it measures with
`--save-baseline`, and compare with `--load-baseline X --baseline Y`, which
re-derives any comparison with zero re-runs.

**Build a micro-benchmark for `for_line`.** Measuring it through the whole
`text_read` stage costs ~6 minutes a side and attributes nothing. A group in
`rust/benchmarks/text/line.rs`, registered in `rust/benchmarks/text.rs` beside
`text_line_benchmarks`, turns the argument into a per-shape number in under a
minute a side. Shapes worth separating: a pipe-framed bridge row, a raw-SOH
numeric frame, an escaped-SOH frame, and prose that names no separator.

For exact attribution with no noise floor at all, callgrind works - but the
bench profile inherits `strip = "symbols"` from `[profile.release]`, so build
with `CARGO_PROFILE_BENCH_STRIP=none CARGO_PROFILE_BENCH_DEBUG=line-tables-only`
and use `--profile-time 1` against the micro group, never the 11 MB corpus.

Disk discipline: 4 cores, release artifacts ~1.7 GB, and this session hit
`No space left on device` mid-build. Check `df` before a release build;
`target/debug/incremental` and stale test binaries are the safe things to drop.

# Part 2 - `TextEntry` becomes text

## The requirement

`TextEntry` should be text, not bytes: its key and value answer `str`, UTF-8 is
forced, and a value that fails to decode is sanitized automatically rather than
refused.

## What it collides with

Three things this branch decided. None of them makes the request wrong; all
three have to be re-decided in the open rather than discovered halfway through.

1. **FIX's binary data fields.** `rust/src/fix/codec.rs:146` declares
   `DATA_TAGS: [i32; 21]` - `RawData(96)`, `XmlData(213)`, `Signature(89)` and
   eighteen more that FIX defines as *opaque bytes*. Decision 2 made the codec
   re-slice such a value forward to the length its `Len` field stated, precisely
   so a value carrying the frame separator survives whole. Sanitizing the page
   before FIX reads it would replace that binary with U+FFFD.
2. **Byte-for-byte re-emission.** `FixMsg::into_bytes` re-emits the received
   line, and `.wire` is pinned for all 264 snapshot messages. Sanitizing changes
   byte lengths - one invalid byte becomes a three-byte replacement - so every
   offset computed before sanitizing is invalid afterwards. Sanitization must
   therefore happen at page construction, before anything scans, or not in the
   page at all.
3. **The `Lossy` anomaly** (`rust/src/fix/anomaly.rs:66`) exists to report a
   value that is not text. Under automatic sanitize it would come to mean "this
   was sanitized", which is arguably the more useful fact - but it is a
   different fact, and its doc and tests say the current one.

The scope is smaller than it looks. **`rust/tests/fix/ulbridge.log` is entirely
valid UTF-8** - all 172,541 bytes - so the real capture cannot move the
snapshot. The collision is with deliberately-binary fixtures (the
`96=\xff\xfe A` case in `rust/tests/fix/message.rs`) and with the 21 data tags
in principle. The migration surface is ~107 `.as_bytes()` call sites across
`rust/src/fix` and `rust/src/media/text`.

## Three shapes - pick one and write it down

**A. Sanitize the page once, at decode.** Validate when the page is sealed; on
failure, sanitize into the page. Every range is then valid UTF-8 by
construction, `as_str()` is infallible and free, and zero-copy is preserved
because entries stay ranges into one page. This is the most literal reading of
"fully str based". It destroys binary data fields before FIX can see them, and
it must run before any scanning.

**B. Keep the page raw, sanitize per entry.** `TextEntry` exposes `&str` when
the range is valid and owns a sanitized string when it is not, allocating only
for the rare bad entry. FIX keeps its bytes. An entry stops being purely a
range, so `TextEntry` grows a storage enum and decision 5's "an entry is a range
of the line" needs restating.

**C. Storage unchanged, accessors force UTF-8.** `key()`/`value()` answer
`&str`/`Cow<str>` infallibly, sanitizing on the way out; `raw()` stays for the
data tags. Byte-exact re-emission and the snapshot survive untouched, and the
21 data tags keep working. Least invasive, and it is what "callers should not
juggle `Option<&str>`" actually needs.

**Recommended: C, unless the intent is specifically that a `TextLine`'s page
itself become text.** If it is, take A and accept that FIX's data fields need
their own byte path - which is a larger change than it sounds, and should be
decided before a line of it is written, not after.

Whichever is chosen: write it into `DECISIONS.md` as decision 10, with what it
costs, before changing code. That is the discipline that made the first four
steps reviewable.

## Proving it

For the regression: the equivalence snapshot must not move, `cargo test
--locked -p yggdryl --test fix the_codec_answers_what_it_answered` is the
fastest tripwire and should run per candidate rather than once at the end, and
`rust/tests/allocations.rs` must hold - `FIX_LINE_COSTS` at 27/42/92,
`FIX_TEXT_LINE_COSTS` at 25/40/90, `WIDE_VALUE_COSTS`, `PACKED_MEMBER_COSTS` at
61/95. Note what those pins cannot see: they count allocations, so atomic
refcount traffic and extra byte scans are invisible to them. Only a benchmark or
a profiler sees those.

For `TextEntry`: the same snapshot, plus a fixture for each of the three shapes'
answers to a non-UTF-8 data field, plus whatever `Lossy` comes to mean.

Then Gate 1 whole, both bindings, and regenerate `docs/fix/arrow.md` from a
clean run - the numbers there now are the slow ones, published deliberately so
they would not be mistaken for the old ones.

## Do not

- Do not regenerate `equivalence.snapshot` to make something pass. If a line
  moves, that is the finding; report it before touching it.
- Do not benchmark while anything else runs on this container.
- Do not run an unnamed criterion baseline.
- Do not take the refuted specs above at face value because their ideas sound
  right. Two of them are right in idea and broken in detail, and the detail is
  where the counterexamples live.
- Do not land the regression work and the `TextEntry` change in one commit.
