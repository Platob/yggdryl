# Unicode compatibility evidence (Excel 16.0 build 20430.0)

`unicode_native.json` contains the 102 native observations from three owned runs,
17 formulas in each of the 1900 and 1904 date systems per run. All three reports
have `passed=true`, `cleanup_completed=true`, 34 observed cases, and 34/34
before-SaveAs and after-SaveAs comparisons equal to their saved cell caches.
The fixture records the source report and manifest hashes, each input and saved
workbook hash, input/saved compatibility marker, source and native formula,
typed COM `Value2` before and after SaveAs, and saved OOXML cache type/raw value.
Ordinary text values remain JSON strings. Each of the 12 lone-half native
values is a typed `str` with `text_transport.encoding="utf16_code_units"`
and `units_hex`; the corresponding decoded cache uses the same transport.
No JSON string contains an unpaired surrogate, so Rust `serde_json` can read
the fixture without dropping the native code unit.
This is native execution and cache transport evidence; no Rust evaluator
equivalence or reopen behavior was tested.

| Input marker | Native report | Key observation | Saved marker |
| --- | --- | --- | --- |
| absent | `p5-unicode-absent-decoded` | 34/34 same typed `Value2` as explicit v1 | absent |
| `setVersion="1"` | `p5-unicode-v1-decoded` | 34/34 same typed `Value2` as absent | absent (Excel removed the extension) |
| `setVersion="2"` | `p5-unicode-v2` | 18/34 differ from v1, nine formulas in each date system | `setVersion="2" warnBelowVersion="2"` |

The input extension is `{D14903EA-33C4-47F7-8F05-3474C54BE107}` with
`http://schemas.microsoft.com/office/spreadsheetml/2024/workbookCompatibilityVersion`.
The builder cites an Excel-produced workbook as marker provenance. The fixture
separately records authored input marker and Excel's saved marker; a saved v1
package cannot be treated as proof that the input marker was absent.

Examples from the 1900 pair (1904 matches exactly): `LEN("A😀Z")` is 4 under
absent/v1 and 3 under v2; `MID("A😀Z",2,1)` is a lone high UTF-16 surrogate
under absent/v1 and the full `😀` under v2; `FIND("Z","A😀Z")` is 4 vs 3;
`REPLACE("A😀Z",2,1,"X")` is `AX` followed by a lone low surrogate then `Z`
under absent/v1, and `AXZ` under v2. The `LEFT`/`RIGHT` controls return whole
emoji in both modes, so these observations do not establish one universal
indexing algorithm for all text functions. `LOWER("İ😀ABC")` yields `i😀abc`,
`UPPER("ß😀abc")` leaves `ß` unchanged, and the tested `PROPER` behavior is
also unchanged between modes; these are only the tested locale/build cases.

For the lone-half cases, COM `Value2` carries Python strings containing
`\ud83d` or `\ude00`; saved OOXML cells have `t="str"` and `<v>` text
`_xD83D_` or `_xDE00_` (or `AX_xDE00_Z`). `scripts/check_excel_desktop.py`
now decodes those OOXML UTF-16 escapes for comparison while retaining the raw
XML text. It uses one pass, so `_x005F_` protecting a literal `_xHHHH_` is not
decoded twice. A Rust UTF-8 `String` cannot represent a lone surrogate. Until
there is an explicit value/cache representation for these outputs, evaluation
must leave such formula results uncomputed and retain their source cache; it
must not substitute U+FFFD or an empty string.

The earlier `p5-unicode-absent/results.json` and `p5-unicode-v1/results.json`
remain failed evidence (12 comparisons each, cleanup succeeded). Their raw
COM and raw saved XML values already had the same six lone-half patterns per
run; the assertion failures came from comparing escaped XML text literally.
The corrected decoder passed its five pure regressions after observed red and
the full 85-test oracle suite. These reports were not rewritten or discarded.

Regenerate and validate without COM or Cargo:

```text
python .scratch/build-p5-unicode-evidence.py
```

The generated fixture has SHA-256
`00a225179fe1c3bcd6dc01f13530b7cd5e61ee0c5d477f390fbb5b827348bf61`.
