# The text and FIX read path through the charset layer: calls and allocations

One piece of work from `main` (PR #107 merged at `aa6566f6`, carrying
decisions 10, 11 and 12 and passing the four gates), on a new branch, after
`FIX_DIRECTION_MESSAGES_PROMPT.md` has landed and its baselines are re-taken;
the read path has not yet been made cheaper than the tree it merged. The
piece is to take the per-row cost of a text line and of a FIX
message down where it is counted - allocations and `IOBase` calls - and to pin
every step, with wall time as the confirmation and never as the proof.

**Order.** Attribution first, because three of the six targets below are
unattributed until a counter names them. Then the largest pinned count. Then
the next. One change per commit, each with its pin moved in the same commit,
so a reviewer can say which change moved which number.

## Outcome

- `read_text_lines` costs two allocations per unframed line - the assembled
  vector and the `Arc` box around it, the page type's floor - each named beside
  its pin, and a framed record one vector per physical line plus one `Arc`
  plus regrowth; or a written statement of why not.
- `FIX_TEXT_LINE_COSTS` and `FIX_LINE_COSTS` fall, each allocation that stays
  named, or the same statement.
- `row_size` and `column_size` through the record door ask the handle once
  per fact, pinned; the record write path gets the call pin it lacks.
- `fix/pipeline/{text_read,parse_text_arrow_reader,parse_lines}` back to
  within noise of `origin/main` (`91454596`) on a quiet Linux box, or a
  written statement of what is irreducible and why.
- Every decision written in `DECISIONS.md` before the code that keeps it; the
  equivalence snapshot unmoved.

## Where it stands, exactly

The branch's local head carries the merge of `origin/main` (`91454596`, PR #106:
decisions 10 and 11) and decision 12 in three commits - `a2755db1` (the merge,
decision 12 written, the charset option deleted), `32370d96` (the stray-byte
reading owned by the charset layer), `76040cc7` (a declared charset applied at
the transport) - plus the regenerated Node declarations and this prompt. The
four gates were run locally on that head; confirm `git log --oneline -5` shows
it and that the PR's CI has seen it before starting. Compare against
`origin/main`, never a local `main` ref, which may be stale.

Pinned, exact, on the branch (`rust/tests/allocations.rs`,
`rust/tests/iobase_calls.rs`):

| pin | value | what it counts |
| --- | --- | --- |
| `reading_text_lines_costs_a_constant_and_four_a_line` | `copy + 8 + 4·N` (95 at 16 rows, 4 130 at 1 024) | `read_text_lines` over N UTF-8 lines with no row header; per line: the assembled vector, the retained-body copy, the record's copy, the `Arc` |
| `a_declared_charset_costs_its_transport_and_nothing_a_line` | +3 under one window, +4 past it | two 64 KiB buffers and a `Box` per declared reader; `Reader::decoded` regrows once past one window |
| `FIX_TEXT_LINE_COSTS` | `[(4, 24), (16, 39), (64, 89)]` | a FIX message parsed from a `TextLine` of 4/16/64 pairs; the page is built outside the count |
| `FIX_LINE_COSTS` | `[(4, 26), (16, 41), (64, 91)]` | the same through the byte door |
| `WIDE_VALUE_COSTS` | `[(4, (26, 29)), (16, (41, 56))]` | a wide value below and above the inline width |
| `PACKED_MEMBER_COSTS` | `[(4, 59), (16, 93)]` | a packed group member |
| `iobase_calls::records::text_costs` | full read `pstream_bytes=1 url=1 bound_location=3 mtime=1 media_type=1 is_container=1 parent=1`; row count `pstream_bytes=1 url=1 media_type=2 is_container=2`; column count `size=1 media_type=1 is_container=2` | one text read, one `row_size`, one `column_size` through the record door |

Not pinned today, and needed before target 1 moves anything: a read with a
row header (captures), and a framed read. `FIX_TEXT_LINE_COSTS` builds its
page outside the counted closure, so target 1 cannot move the FIX pins; they
are the tripwire that must not move.

Measured, on a Windows box that was not quiet enough to trust past the first
digit, and named so they are not mistaken for a proof: the merge commit
against `origin/main` in one session, `text_read` −1.7 %,
`parse_text_arrow_reader` −1.1 %, `parse_lines` +0.9 %, every
`text_lines/decode/*` within ±4 %, and every `fix/line/*/parse_line`
+0.5 … +4.8 % while every `fix/line/*/scan` beside it stayed flat. That last
row is the one unattributed signal: the scan never builds a value, the parse
builds one `Str` per field and casts it to its field. The declaration
commit's slot was contaminated (the controls moved +30 … +80 %) and was not
re-measured. `docs/fix/arrow.md` holds `main`'s numbers (this branch changes
four `DataType::utf8()` spellings in it), measured on a Windows 11 machine
(AMD Ryzen 5 150, rustc 1.96.1, per its own Performance paragraph);
regenerate it from a clean run when the work is done, not before.

## Read first

- `AGENTS.md` in full. Gate 1 is §2 and includes the four MSRV checks, each
  spelled `cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl`
  with `--all-targets`, with `--no-default-features --lib`, with
  `--no-default-features --features object --lib`, and
  `cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg`.
  All four pass at the branch head - keep them passing. The crate denies
  `unsafe_code`: no unchecked cast is an answer to anything below.
- `DECISIONS.md`: twelve decisions and three amendments. Decision 5 (an entry
  is a range of the line), decision 2 (a data field re-slices by its stated
  length, in page offsets), decision 7 (the pin that the third refuted item
  below breaks on) and decision 12 (no charset past the reader's door; the
  transport is where a declaration is applied) are the ones a page or buffer
  change collides with first.
- `docs/fix/arrow.md`, the Performance paragraph that states what stays
  (line 724): "That is the cost of decisions 2 and 5: every key and value a
  message records is a range of the line it came from … It goes only with a
  reader that builds the codec's pairs from the scanner's spans without the
  tree between them, which is a change to what an entry is and not to how
  fast it is read." The tree is what decisions 2 and 5 require; what target 2
  may remove is the stage each pair crosses between the tree and the builder,
  within what decision 7's pin allows. That is the claim to test, not to
  accept.
- `rust/src/media/text/arrow.rs`: `RawRows::next_line`, `header_match`,
  `parse_line`, `RawRecord::{new, append, finish}`, `TextLines::convert` -
  the four allocations per line live between them, at the lines named under
  target 1. `rust/src/media/text/bytes.rs` (`TextBytes`, `from_whole_page`,
  `from_page`, `slice`, and a module doc that describes a window-as-page
  reader the code does not implement - see Refuted). `rust/src/media/text/line.rs`
  (`decoded`, `body`, `capture`), `rust/src/media/text/batch.rs` (`value_of`),
  `rust/src/media/text/entry.rs` (`from_bytes_direct_located`,
  `collect_entries`, `key`/`value`), `rust/src/mime_type/line.rs` (the
  scanner: untouched by the merge and not a target), `rust/src/fix/codec.rs`
  (`parse_text_line`, `frame_arrivals`, `judged_keys`, `own_pairs`,
  `split_members`), `rust/src/fix/build.rs` (`Builder::push_pairs`, `finish`,
  `prepare_text`), `rust/src/types/string/scalars.rs` (`Str::new`,
  `from_bytes`, `from_storage`, `try_with_parameters`) and
  `rust/src/types/string/casts.rs`.
- `rust/tests/fix/equivalence.snapshot`: 264 messages, pinning `.type`,
  `.digest`, `.wire`, every `.entry[i]`, every `.field.<column>`. Regenerated
  only under `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`; regenerating it to make a test
  pass is the one thing that makes the work unreviewable.

## Targets, in the order of what a counter can prove

1. **The page is the line's vector.** Per unframed line today:
   `next_line` assembles the line into a `Vec<u8>` (`arrow.rs:830`),
   `parse_line` copies the retained body out of it (`:800`), `RawRecord::new`
   copies again (`:986`), `finish` wraps that copy in `Arc::new` (`:1019`);
   `convert` takes the page as it is (`from_whole_page`, `:1106`,
   allocation-free). Copies two and three are pure: `parse_line` already
   works in offsets and materializes only at the end, and `RawRecord::new`'s
   copy is a `truncate`. The floor is two - the vector and the `Arc` box, since
   the page is `Arc<Vec<u8>>` and the `RawRow` doc (`:520-527`) rules out an
   `Arc<[u8]>` realloc-and-copy - by turning both copies into offsets.
   Captures: `header_match` copies each with `to_vec()` (`:644`) and `convert`
   wraps each in `Arc::new` (`:1124`), beside three vectors of
   `Option<TextBytes>` per line (`parse_line` `:692`, `convert` `:1121`,
   `set_captures` `line.rs:279`); with the body a range of the page, a capture
   is `TextBytes::from_page(&page, start, end)` and the three vectors fold to
   one. Two collisions to settle first, in writing: both header paths remove
   the match with `bytes.drain(found.range)` (`:731`, `:880`) and the row
   header is compiled unanchored (`options.rs:50`; only the bridge's own
   header starts with `^`), so a match that starts past 0 leaves a body that
   is two disjoint ranges - keep the copy and count it there, or require an
   anchored header and say so in `set_rowheader`; and capture offsets come
   from `Regex::captures` over the assembled vector before `drain` shifts it,
   so `drain` and `truncate` become a body range in the same commit. Under
   framing, `append` (`:993-1009`) copies each physical line into the record's
   buffer, an allocation only on regrowth; the first line's vector can be that
   buffer; what stays is one vector per physical line, because the header is
   matched before the record exists. Decision 10's `decoded` still makes a
   page of its own for a line that was not UTF-8 - the rare line, unchanged.
   Pins to add before the change: a captured read (`ULBRIDGE_ROWHEADER` over
   `bridge_lines`) and a framed read, so the move is visible; then the
   absolute pin moves to `2` per line and the FIX pins do not.
2. **Attribute the 24.** `FIX_TEXT_LINE_COSTS` at four pairs is 24
   allocations, none named. The allocator is global and counts a closure, so
   attribute by subtraction of public doors - `TextEntries::from_bytes` alone,
   then `FixCodec::parse_text_line` minus it - and by callgrind for the
   remainder; the private stages (`frame_arrivals`, `judged_keys`,
   `own_pairs`, `Builder::push_pairs`/`finish`) have no door a test can
   count, and adding hidden doors is a decision to write first. From the
   code, expect about twenty constant (the tree's one `with_capacity`,
   `entry.rs:466`; `arrived`, `codec.rs:1789`; the judged, kept and resolved
   lists, `:1093`, `:1015-1021`; the builder's four `with_capacity`,
   `build.rs:547-550`; `finish`'s five, `:1472-1502`) plus one `vec![value]`
   per slot (`build.rs:1577`, `:1439`) - the slope the three widths show,
   `(39−24)/12` and `(89−39)/48`. None of the 24 is a `SmolStr`: keys and
   values are inline at this pin. Then remove what decisions 2 and 5 do not
   require, within decision 7. The 11 `Arc` refcount increments per pair the
   previous hand-off counted were cut by PR #106; refcount traffic is
   invisible to the allocator, so count what is left with a probe.
3. **`Str` on the parse path.** `Scalar::from(&str)` is `Str::new`
   (`scalars.rs:91-97`): one `SmolStr::new` beside an 8-byte `const`
   `StringParameters::default()`, no `try_with_parameters`, no validation;
   `from_storage` validates nothing either. Nothing to skip there. The door
   that validates per value on the parse path is the field cast:
   `prepare_text` (`fix/build.rs:1958-1965`) → `try_with_parameters`
   (`casts.rs:70-80`: `validate`, the ASCII repertoire walk on an `ascii`
   field, `encoded_len` on a bounded one). Measure that door, not the
   constructor: add a `SmolStr::new` control beside the existing
   `rust/benchmarks/types/datatype/string.rs` cases at 8, 22 and 64 bytes
   (`Str::from_storage` is `pub(crate)` - reach it through the file's `*_read`
   round trips), and a `prepare_text`-shaped case. If the cast is the cost,
   the fix is one check per field rather than per value, never a second
   string type; `Scalar` stays 48 bytes.
4. **A line proves it is text once.** `decoded` validates the body when the
   line is made; `body()` and `capture()` validate again on every call
   (`line.rs`), and `value_of` (`batch.rs:150`) asks `body()` per row for the
   body column, while `TextEntry::key()`/`value()` run `from_utf8_lossy` per
   lifted entry (`entry.rs:74-83`). `classify` takes bytes and validates
   nothing. The crate denies `unsafe_code`, so there is no unchecked cast;
   the proof cannot live on `TextBytes` either, since a slice of proven text
   is not proven (a byte-regex capture can split a character). What is left
   is a change to what a page is - a `str`-backed page beside the byte page,
   which decision 5 and the codec's byte door (`codec.rs:918-932`, "none of
   it validated") would have to agree to - and it is a decision to write
   before a line of it. Count the scans first: pin `text_batch/build/plain`
   and `text_records/record/plain`, where `value_of` runs per row;
   `text_lines/decode/*` cannot move, because the decode path never calls
   `body()`.
5. **Calls.** The full read already asks `media_type` and `is_container`
   once. It is `row_size` and `column_size` that pay two: `IOMedia::row_size`
   (`iomedia.rs:43`) asks `is_container` at `:46` after `dimension_options` →
   `record_options` (`:119`) asked it at `:121` and `media_type` at `:145`,
   and `text::arrow::row_size` (`media/text/arrow.rs:177`) asks `media_type`
   again; `column_size` (`:86`) repeats `record_options`' `is_container`.
   Carry the container bit and the media type the dimension already resolved
   into `leaf_row_size` (`iobase/transfer.rs:1001-1012`) and pin `row_size`
   at `media_type=1 is_container=1` and `column_size` at `is_container=1`.
   The handle write is pinned (`iobase_calls.rs:250`); the record write
   (`overwrite_arrow_batch`/append through the text media) is not - add it.
6. **Named and waiting.** `charset::Reader::new` reserves `decoded` at one
   window and regrows once past it (the `+4` above): reserve
   `2 × DEFAULT_STREAM_BATCH_SIZE` or amend the pin and §12 - decide, do not
   leave both. `Transcoded<File>` (`charset/transcoded.rs:474`),
   `Coding<File>` (`coding/mod.rs:478`) and the `Coded` enum
   (`coding/coded.rs:304`) delegate `bound_location`, and `owned_handle`
   (`media/text/arrow.rs:189-206`) re-opens that location raw under the
   wrapper's media type: a byte-changing wrapper answers no bound location;
   pin `Transcoded<File>` and `Coding<File>` through `read_text_lines`.
   `python/src/iomedia.rs` `Rows::fill` (`:707`) builds pyarrow batches with
   `from_pylist(chunk, schema=self.columns)`; if `str -> fixed_ascii` rows
   fail there before the core cast, pin it with a test first.

## Refuted before - do not retry as specified

- Sharing one page across a `parse_lines` batch: `fix/codec.rs` reaches the
  raw page behind a `TextBytes`, and two lines in one page change what a data
  field's stated length re-slices. Target 1 is one page per *line*, which is
  what decision 5 already says; a page per batch is not.
- The window as the page - what `bytes.rs`'s module doc describes and
  `arrow.rs` does not do: the data-field widening (`codec.rs:1800`) is bounded
  by the page, not the line, and one retained line would retain the 64 KiB
  window. Rewrite that doc paragraph when target 1 lands.
- Hoisting the separator decision out of the per-line loop: lines 7, 9 and 57
  of `ulbridge.log` carry nested values whose frame differs from their
  parent's.
- Collapsing the `Vec` stages between the entry tree and the builder: a pin
  named in decision 7 breaks on a concrete input; revisit only with that pin
  re-derived first.

## How to measure

- Pins first: `cargo test --locked -p yggdryl --test allocations --test iobase_calls`
  per candidate, and
  `cargo test --locked -p yggdryl --test fix the_codec_answers_what_it_answered`
  as the tripwire, per candidate rather than once at the end.
- Wall time as confirmation: named criterion baselines only
  (`--save-baseline <commit>`; compare from the saved `estimates.json`, since
  criterion refuses `--baseline` beside `--save-baseline`), `CRITERION_HOME`
  outside `target/`, on a quiet box, `text_scan` and `fix/line/*/scan` as the
  controls that never see a value or a charset - if the controls move more
  than noise, the run is contamination, not a result. Groups: `text_lines`,
  `text_batch`, `text_records`, `text_record_framing`, `text_scan`,
  `fix/line`, `fix/pipeline/{text_read,parse_text_arrow_reader,parse_lines}`.
- For attribution, callgrind on the `fix/line` and `text_scan` micro groups
  with `CARGO_PROFILE_BENCH_STRIP=none CARGO_PROFILE_BENCH_DEBUG=line-tables-only`
  and `--profile-time 1`; never the 11 MB corpus.
- Then Gate 1 whole, both bindings (the wheel and the addon rebuilt), Gate 4,
  and `docs/fix/arrow.md` regenerated from one clean run with the method
  named the way that page names it.

## Do not

- Do not regenerate `equivalence.snapshot` to make something pass. If a line
  moves, that is the finding; report it before touching it.
- Do not put a `Charset`, or a branch on one, past the reader's door
  (decision 12; `AGENTS.md` Charsets), and do not reach for `unsafe`.
- Do not benchmark while anything else runs on the box, and never unnamed.
- Do not land two targets in one commit.
- Do not take the refuted items above at face value because their ideas sound
  right; the counterexamples are in the detail.
