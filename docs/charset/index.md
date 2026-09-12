# Charset

`Charset` names a character encoding; `charset::Transcoded` applies it over any `IOBase` handle so downstream code sees UTF-8.

## Pages

| Page | Purpose |
| --- | --- |
| [Encodings](encodings.md) | Every charset, its aliases, and what it holds |
| [Transcoded](transcoded.md) | The decoding handle, and random access through it |
| [Streams](streams.md) | `Decoder`, `Reader`, `Writer` over chunks |

## Contract

| | |
| --- | --- |
| Owns | `Charset`, `charset::Decoder`, `charset::Reader`, `charset::Writer`, `charset::Transcoded` |
| Charsets | `utf-8`, `utf-16le`, `utf-16be`, `us-ascii`, `iso-8859-1`, `iso-8859-2`, `iso-8859-15`, `windows-1250`, `windows-1251`, `windows-1252`, `ibm437`, `ibm850`, `macintosh` |
| Select | `Charset::from_str` for a name or alias; `Charset::from_media_type` and `Charset::from_url` for what a resource declares; `Charset::from_bom` for what a payload declares about itself |
| Operations | `decode`/`decode_lossy`/`transcribe`/`encode` for whole buffers, `encoded_len` for the stored length alone, `reader`/`writer` for streams, `decoder` for chunks and `transcriber` for chunks read as `transcribe` reads them |
| Borrow | An all-ASCII payload is already UTF-8, so `decode` and `encode` borrow it; asserted in the counting allocator |
| Composes | Any [`IOBase`](../holder/index.md) through `Transcoded`, over or under a [`Coded`](../coding/index.md) handle; `Transcoded` is itself an `IOBase` |
| Seek | Through `Transcoded`, by resume points recorded in one pass; the value is materialized only by an explicit `open` or a positional write |
| Declared by | [`MediaType`](../uri/path.md) carries `Option<Charset>`, parsed from `charset=` and rendered back |
| Refuses | A byte the charset leaves unassigned, a scalar it has no byte for, a broken sequence, and bare `utf-16` |
| Python | `yggdryl.charset` - `decode`, `decode_lossy`, `encode`, `from_bom`, `bom`, `canonical_name`, `CHARSETS` |
| JavaScript | `charset` - `decode`, `decodeLossy`, `encode`, `fromBom`, `bom`, `canonicalName`, `CHARSETS` |

## Use

Text crosses this boundary once. A byte payload is decoded at intake - by a `Transcoded` handle, by the [structured plan](../text/index.md), by the [text record reader's](../media/text.md#declaring-a-charset) transport from the charset its handle's media type declares, or by a direct `decode` - and everything past that point is `str`, a `Scalar::String`, or an Arrow string array; nothing re-decodes, and no layer branches on a charset per row.

=== "Rust"

    ```rust
    use yggdryl::Charset;

    // One byte per scalar on the wire, three in UTF-8.
    let wire = b"prix: 12\x80";
    assert_eq!(Charset::Cp1252.decode(wire)?, "prix: 12€");
    assert_eq!(Charset::Cp1252.encode("prix: 12€")?.as_ref(), wire);

    // The same bytes are a different document under a different charset.
    assert_eq!(Charset::Latin1.decode(wire)?, "prix: 12\u{0080}");
    assert_eq!(Charset::from_str("windows-1252")?, Charset::Cp1252);
    ```

=== "Python"

    ```python
    from yggdryl import charset

    wire = b"prix: 12\x80"
    assert charset.decode("windows-1252", wire) == "prix: 12€"
    assert charset.encode("windows-1252", "prix: 12€") == wire

    assert charset.decode("iso-8859-1", wire) == "prix: 12"
    assert charset.canonical_name("cp1252") == "windows-1252"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { charset } = require('yggdryl')

    const wire = Buffer.from('prix: 12\x80', 'latin1')
    assert.equal(charset.decode('windows-1252', wire), 'prix: 12€')
    assert.deepEqual(charset.encode('windows-1252', 'prix: 12€'), wire)

    assert.equal(charset.decode('iso-8859-1', wire), 'prix: 12')
    assert.equal(charset.canonicalName('cp1252'), 'windows-1252')
    ```

## Declaring a charset

A resource declares its charset on its [media type](../uri/path.md), the way it declares its codings, so a reader needs no argument. The structured codec and the [text record reader](../media/text.md#declaring-a-charset) both read the declaration where they build their transport, coding first and charset second, and neither takes a charset of its own: the media type is the one owner of which charset the bytes are in.

=== "Rust"

    ```rust
    use yggdryl::{Charset, MediaType};

    let declared = MediaType::from_str("text/csv;charset=windows-1252")?;
    assert_eq!(declared.charset(), Some(Charset::Cp1252));
    assert_eq!(Charset::from_media_type(&declared), Charset::Cp1252);

    // Declaring nothing reads as UTF-8 without recording a declaration.
    let silent = MediaType::from_str("text/csv")?;
    assert_eq!(silent.charset(), None);
    assert_eq!(Charset::from_media_type(&silent), Charset::Utf8);
    ```

=== "Python"

    ```python
    from yggdryl import MediaType

    declared = MediaType.from_str("text/csv;charset=windows-1252")
    assert declared.charset == "windows-1252"
    assert str(declared) == "text/csv;charset=windows-1252"

    assert MediaType.from_str("text/csv").charset is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MediaType } = require('yggdryl')

    const declared = MediaType.fromString('text/csv;charset=windows-1252')
    assert.equal(declared.charset, 'windows-1252')
    assert.equal(declared.toString(), 'text/csv;charset=windows-1252')

    assert.equal(MediaType.fromString('text/csv').charset, null)
    ```

## Reading what cannot be read

Three doors, one verb. `decode` refuses a byte the charset leaves unassigned and names where it is. `decode_lossy` reads the same bytes and marks each fault with `U+FFFD`, which is what a capture of arbitrary wire lines wants and never what a stored column wants. `transcribe` reads every byte it can: an unassigned byte becomes the scalar ISO 8859-1 gives it - which for the five bytes `windows-1252` leaves unassigned is what the WHATWG Encoding Standard's own index answers - and bytes offered as UTF-8, or as US-ASCII, that are not what they were offered as are read by one rule, written once in this layer: every valid UTF-8 run is kept as it is, and every other byte reads as the character Windows-1252 gives it, as the WHATWG Encoding Standard tables it, with the five bytes the classic table leaves undefined as the C1 controls of their number. Per invalid run rather than per buffer, because a buffer is mostly UTF-8 with a stray byte far more often than it is wholly Windows-1252, and reading a valid `é` as `Ã©` because a lone `0xE9` stands elsewhere would destroy what was right to repair what was wrong. US-ASCII reads exactly as UTF-8, because a US-ASCII declaration is a UTF-8 declaration with a narrower promise and a broken promise about UTF-8-compatible text has one rule. The [text record reader](../media/text.md#a-line-is-text) makes every line text by this rule and holds no table of its own. A [string column](../types/text.md) reads its bytes through one door, `Str::from_bytes`, and the charset picks which of these it opens: `utf8` and `ascii` (and every bounded or fixed spelling of them) go through `decode`, so bytes that are not what they claim are refused and a US-ASCII value holds no NUL and no byte above `0x7F`; every other charset - `string(windows-1252)`, `fixed_string(iso-8859-1,8)` - goes through `transcribe`, because it declares legacy bytes that still have to be read.

=== "Rust"

    ```rust
    use yggdryl::{Charset, Error};

    let error = Charset::Cp1252.decode(b"ok\x81").unwrap_err();
    assert!(matches!(error, Error::Codec { format: "windows-1252", position: 2, .. }));
    assert_eq!(Charset::Cp1252.decode_lossy(b"ok\x81"), "ok\u{FFFD}");
    assert_eq!(Charset::Cp1252.transcribe(b"ok\x81"), "ok\u{0081}");
    // Bytes that are not the UTF-8 they claim to be still read.
    assert_eq!(Charset::Utf8.transcribe(b"caf\xe9"), "café");
    // Per run: the valid `é` is kept, and only the stray byte reads as Windows-1252.
    assert_eq!(Charset::Utf8.transcribe(b"caf\xC3\xA9 \xE9"), "café é");
    // US-ASCII reads exactly as UTF-8.
    assert_eq!(Charset::Ascii.transcribe(b"caf\xC3\xA9 \x80"), "café €");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import charset

    with pytest.raises(ValueError) as refusal:
        charset.decode("windows-1252", b"ok\x81")
    assert "windows-1252" in str(refusal.value)
    assert charset.decode_lossy("windows-1252", b"ok\x81") == "ok�"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { charset } = require('yggdryl')

    const broken = Buffer.from([0x6f, 0x6b, 0x81])
    assert.throws(() => charset.decode('windows-1252', broken), /windows-1252/)
    assert.equal(charset.decodeLossy('windows-1252', broken), 'ok�')
    ```

## Edges

- Bare `utf-16` -> `Err`. RFC 2781 reads an unmarked stream as big-endian and the WHATWG Encoding Standard reads it as little-endian, so the name is ambiguous rather than defaulted; `utf-16le`, `utf-16be`, and `Charset::from_bom` are the three unambiguous answers.
- `iso-8859-1` -> ISO 8859-1, never `windows-1252`. The two differ over `0x80..=0x9F`, so a value that answered one for the other would put a smart quote where a control character was written. A browser's `TextDecoder` resolves that label the other way.
- `Charset::from_bom` -> the charset and the mark's byte length; the mark is never stripped on a caller's behalf, so `decode` of a marked payload begins with `U+FEFF`. The [structured-text](../text/index.md) plan is the one place a mark is treated as framing.
- An unassigned byte -> `Err` naming the charset, the byte position, and the byte. Only `windows-1250`, `windows-1251` and `windows-1252` leave any unassigned; every other single-byte charset here answers for all 256, and for one that does `transcribe` and `decode_lossy` answer the same text. Under `utf-8` and `us-ascii` the two doors part: `transcribe` reads a stray byte as Windows-1252 where `decode_lossy` writes `U+FFFD`.
- A lone UTF-16 surrogate -> `U+FFFD` from `transcribe` too: it is not a scalar in any encoding, so there is nothing to transcribe it to.
- `encoded_len` -> the bytes the charset stores that text in, counted without building them. It counts rather than judges: a scalar the charset has no byte for still occupies the one it would occupy, because that is what a [length bound](../types/text.md) is asking. `encode` is the one door that refuses such a scalar, and it refuses it where the bytes are written.
- A scalar the charset has no byte for -> `Err` naming the scalar as `U+XXXX`. Every scalar has a UTF-16 form, so the Unicode forms never refuse an encode.
- `Charset::Utf8` through `reader`, `writer` or `Transcoded` -> the bytes pass through unchanged and are not revalidated, exactly as `Codec::Identity` leaves bytes alone. `decode` is the validating door.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib charset::
    cargo test --features "parquet iceberg" -p yggdryl --test charset
    cargo test -p yggdryl --test iobase_calls a_random_read
    cargo test -p yggdryl --test allocations charset
    python scripts/check_charset_interop.py
    python scripts/generate_charset_tables.py --check
    cargo bench -p yggdryl --bench charset -- charset_borrow
    cargo bench -p yggdryl --bench charset -- charset_decode
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/charset
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/charset/*.test.js"
    ```
