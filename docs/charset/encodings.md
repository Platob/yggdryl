# Encodings

Every charset this crate reads and writes, what it holds, and the names that resolve to it.

## Contract

| | |
| --- | --- |
| Canonical name | The IANA name, which is also what a `Content-Type` header and a `MediaType` carry |
| Aliases | Accepted at intake by `Charset::from_str`, case-insensitively and trimmed; never answered |
| Unassigned | A byte the charset gives no scalar; `decode` refuses it and `decode_lossy` replaces it |
| Tables | Generated from Python's codec registry by `scripts/generate_charset_tables.py` and checked against it both directions by `scripts/check_charset_interop.py` |

## The charsets

| Canonical name | Family | Holds | Unassigned bytes | Accepted aliases |
| --- | --- | --- | --- | --- |
| `utf-8` | Unicode | Every scalar, one to four bytes | none | utf8, utf_8, u8, csutf8 |
| `utf-16le` | Unicode | Every scalar, two or four bytes | none | utf16le, utf-16-le, csutf16le |
| `utf-16be` | Unicode | Every scalar, two or four bytes | none | utf16be, utf-16-be, csutf16be |
| `us-ascii` | ASCII | `U+0000`-`U+007F` | every byte above `0x7F` | ascii, us, iso646-us, ansi_x3.4-1968, cp367 |
| `iso-8859-1` | ISO 8859 | `U+0000`-`U+00FF` | none | latin1, latin-1, l1, iso8859-1, cp819 |
| `iso-8859-2` | ISO 8859 | Central European Latin | none | latin2, latin-2, l2, iso8859-2 |
| `iso-8859-15` | ISO 8859 | Latin-1 with the euro and seven others | none | latin9, latin-9, l9, iso8859-15 |
| `windows-1250` | Windows | Central European | `0x81`, `0x83`, `0x88`, `0x90`, `0x98` | cp1250, windows1250, x-cp1250 |
| `windows-1251` | Windows | Cyrillic | `0x98` | cp1251, windows1251, x-cp1251 |
| `windows-1252` | Windows | Western European | `0x81`, `0x8d`, `0x8f`, `0x90`, `0x9d` | cp1252, windows1252, x-cp1252 |
| `ibm437` | DOS | The original IBM PC set | none | cp437, 437, oem-us |
| `ibm850` | DOS | Western European DOS | none | cp850, 850 |
| `macintosh` | Mac OS | Western European, classic Mac OS | none | mac, macroman, mac-roman, csmacintosh |

Every charset but the UTF-16 pair agrees with US-ASCII on `0x00..=0x7F`, which is what lets a decode borrow an all-ASCII payload instead of transcoding it, and what lets a line scan split on `\n` before anything is decoded.

## Use

The whole repertoire of a single-byte charset round trips, and `scalar_of` and `byte_of` are the two halves of its table.

```rust
use yggdryl::Charset;

// Every byte `windows-1252` assigns, decoded and encoded back.
let assigned: Vec<u8> = (0..=u8::MAX)
    .filter(|byte| Charset::Cp1252.scalar_of(*byte).is_some())
    .collect();
let decoded = Charset::Cp1252.decode(&assigned)?;
assert_eq!(decoded.chars().count(), assigned.len());
assert_eq!(Charset::Cp1252.encode(&decoded)?.as_ref(), assigned);

// Five bytes are unassigned, so the repertoire is 251 rather than 256.
assert_eq!(assigned.len(), 251);
assert_eq!(Charset::Cp1252.scalar_of(0x80), Some('\u{20AC}'));
assert_eq!(Charset::Cp1252.scalar_of(0x81), None);
assert_eq!(Charset::Cp1252.byte_of('\u{20AC}'), Some(0x80));
```

## Edges

- `scalar_of` and `byte_of` on `utf-8`, `utf-16le` or `utf-16be` -> `None`. Their scalars are not one byte wide, so there is no per-byte answer to give.
- An alias -> resolved at intake and never answered: `Charset::as_str` and `Display` always spell the canonical name.
- A name with surrounding whitespace or in any case -> accepted; anything else -> `Err` naming the whole vocabulary.
- Adding a charset -> a row in `scripts/generate_charset_tables.py`, a regenerated `rust/src/charset/tables.rs`, a variant, and a row here. The enum is `#[non_exhaustive]` for exactly that reason.

## Commands

```bash
python scripts/generate_charset_tables.py
python scripts/generate_charset_tables.py --check
python scripts/check_charset_interop.py
cargo test --features "parquet iceberg" -p yggdryl --test charset vocabulary::
```
