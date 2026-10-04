# Charsets

Bytes become text once, at intake, in the charset a handle's media type declares; everything past that point is UTF-8, and a writer encodes back out the same way.

## Overview

| | |
| --- | --- |
| Declared by | the `charset` parameter of a media type - `text/csv;charset=windows-1252` - else UTF-8; where none is declared, a structured-text document (JSON, YAML, TOML, XML) also reads a byte-order mark, and XML its own declaration |
| Build | default |
| Charsets | thirteen, `Charset::ALL`: `utf-8`, `utf-16le`, `utf-16be`, `us-ascii`, `iso-8859-1`, `iso-8859-2`, `iso-8859-15`, `windows-1250`, `windows-1251`, `windows-1252`, `ibm437`, `ibm850`, `macintosh` - each spelled by its common aliases too, in any case |
| Rust | `Charset`: `from_str`, `decode`, `decode_lossy`, `transcribe`, `encode`, `encoded_len`, `bom`, `from_bom`, `from_media_type`, `reader`, `writer` and `decoder`; `yggdryl::charset::Transcoded` a handle presenting decoded text |
| Python | `yggdryl.charset`: `CHARSETS`, `canonical_name`, `decode`, `decode_lossy`, `encode`, `bom`, `from_bom` |
| JavaScript | `charset`: `CHARSETS`, `canonicalName`, `decode`, `decodeLossy`, `encode`, `bom`, `fromBom` |
| Refused | bare `utf-16`, which RFC 2781 and the WHATWG Encoding Standard read differently - `utf-16le`, `utf-16be` and a byte-order mark are the unambiguous answers - and `iso-8859-1` is ISO 8859-1, never `windows-1252` |

A text column that keeps the charset it is written in - the US-ASCII and windows-1252 string leaves - is a datatype of its own, on [Strings](../types/text/string.md).

## Read

Where the caller did not say, an explicit argument wins, then the charset the media type declares. Only a structured-text document reads further where none is declared: one bounded read of a byte-order mark, which it takes off, and for XML the encoding its declaration states. A record reader - CSV, plain text - takes off only the mark of the form it was declared in and reads any other mark as data, so it finds no UTF-16 file by its mark. A mark is framing rather than content, so `from_bom` answers the charset and the mark's length, and the caller decides. `decode` refuses a byte the charset does not hold, naming the charset, the byte position and what it found there; `decode_lossy` marks each fault with U+FFFD, which is what a capture of arbitrary wire bytes needs; Rust's `transcribe` reads every byte it can, an unassigned one as its ISO 8859-1 scalar. Every charset but the UTF-16 pair agrees with US-ASCII below `0x80`, so an all-ASCII payload is borrowed rather than transcoded, and a line is split before anything is.

=== "Rust"

    ```rust
    use yggdryl::Charset;

    // One byte per scalar on the wire, three in UTF-8.
    let wire = b"prix: 12\x80";
    assert_eq!(Charset::Cp1252.decode(wire)?, "prix: 12€");

    // The same bytes are a different document under a different charset.
    assert_eq!(Charset::Latin1.decode(wire)?, "prix: 12\u{0080}");
    assert_eq!(Charset::from_str("windows-1252")?, Charset::Cp1252);

    // A byte-order mark is framing: its charset, and its length.
    assert_eq!(Charset::from_bom(b"\xEF\xBB\xBFhello"), Some((Charset::Utf8, 3)));
    assert_eq!(Charset::from_bom(b"plain"), None);

    // `decode` refuses what the charset does not hold; `decode_lossy` marks it.
    let refused = Charset::Utf8.decode(b"caf\xE9 au lait").unwrap_err();
    assert!(refused.to_string().contains("at byte 3"), "{refused}");
    assert_eq!(Charset::Utf8.decode_lossy(b"caf\xE9 au lait"), "caf\u{FFFD} au lait");
    assert!(Charset::from_str("utf-16").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import charset

    # One byte per scalar on the wire, three in UTF-8.
    wire = b"prix: 12\x80"
    assert charset.decode("windows-1252", wire) == "prix: 12€"

    # The same bytes are a different document under a different charset.
    assert charset.decode("iso-8859-1", wire) == "prix: 12\x80"
    assert charset.canonical_name("cp1252") == "windows-1252"

    # A byte-order mark is framing: its charset, and its length.
    assert charset.from_bom(b"\xef\xbb\xbfhello") == ("utf-8", 3)
    assert charset.from_bom(b"plain") is None

    # `decode` refuses what the charset does not hold; `decode_lossy` marks it.
    with pytest.raises(ValueError, match="at byte 3"):
        charset.decode("utf-8", b"caf\xe9 au lait")
    assert charset.decode_lossy("utf-8", b"caf\xe9 au lait") == "caf� au lait"
    with pytest.raises(ValueError, match="utf-16le"):
        charset.canonical_name("utf-16")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { charset } = require('yggdryl')

    // One byte per scalar on the wire, three in UTF-8.
    const wire = Buffer.from('prix: 12\x80', 'latin1')
    assert.equal(charset.decode('windows-1252', wire), 'prix: 12€')

    // The same bytes are a different document under a different charset.
    assert.equal(charset.decode('iso-8859-1', wire), 'prix: 12\x80')
    assert.equal(charset.canonicalName('cp1252'), 'windows-1252')

    // A byte-order mark is framing: its charset, and its length.
    assert.deepEqual(charset.fromBom(Buffer.from([0xef, 0xbb, 0xbf, 0x68])), { charset: 'utf-8', length: 3 })
    assert.equal(charset.fromBom(Buffer.from('plain')), null)

    // `decode` refuses what the charset does not hold; `decodeLossy` marks it.
    const damaged = Buffer.from('caf\xe9 au lait', 'latin1')
    assert.throws(() => charset.decode('utf-8', damaged), /at byte 3/)
    assert.equal(charset.decodeLossy('utf-8', damaged), 'caf� au lait')
    assert.throws(() => charset.canonicalName('utf-16'), /utf-16le/)
    ```

## Write

`encode` writes text in a charset and refuses a scalar the charset cannot spell, naming the scalar and where it stands: there is no lossy encode, because a scalar with no byte is unrepresentable input. Rust's `encoded_len` answers the stored length without building the bytes - it counts rather than judges, so a length bound costs a walk and `encode` stays the one authority on whether the bytes can be written - and `writer` encodes a stream. A whole resource in one charset is a `Transcoded` handle, never an option on every reader.

=== "Rust"

    ```rust
    use yggdryl::Charset;

    // One byte for the euro in windows-1252, three in UTF-8.
    assert_eq!(Charset::Cp1252.encode("prix: 12€")?.as_ref(), b"prix: 12\x80");
    assert_eq!(Charset::Cp1252.encoded_len("prix: 12€"), 9);
    assert_eq!(Charset::Utf8.encoded_len("prix: 12€"), 11);

    // UTF-16 spends two bytes on every ASCII scalar, low byte first in `utf-16le`.
    assert_eq!(Charset::Utf16Le.encode("ok")?.as_ref(), b"o\0k\0");
    assert_eq!(Charset::Utf16Le.bom(), Some(&b"\xFF\xFE"[..]));

    // A scalar the charset cannot spell is refused, never replaced.
    let refused = Charset::Cp1252.encode("yen \u{5186}").unwrap_err();
    assert!(refused.to_string().contains("U+5186"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import charset

    # One byte for the euro in windows-1252, three in UTF-8.
    assert charset.encode("windows-1252", "prix: 12€") == b"prix: 12\x80"
    assert charset.encode("utf-8", "prix: 12€") == "prix: 12€".encode()

    # UTF-16 spends two bytes on every ASCII scalar, low byte first in `utf-16le`.
    assert charset.encode("utf-16le", "ok") == b"o\x00k\x00"
    assert charset.bom("utf-16le") == b"\xff\xfe"

    # A scalar the charset cannot spell is refused, never replaced.
    with pytest.raises(ValueError, match=r"U\+5186"):
        charset.encode("windows-1252", "yen 円")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { charset } = require('yggdryl')

    // One byte for the euro in windows-1252, three in UTF-8.
    assert.deepEqual(charset.encode('windows-1252', 'prix: 12€'), Buffer.from('prix: 12\x80', 'latin1'))
    assert.deepEqual(charset.encode('utf-8', 'prix: 12€'), Buffer.from('prix: 12€'))

    // UTF-16 spends two bytes on every ASCII scalar, low byte first in `utf-16le`.
    assert.deepEqual(charset.encode('utf-16le', 'ok'), Buffer.from([0x6f, 0x00, 0x6b, 0x00]))
    assert.deepEqual(charset.bom('utf-16le'), Buffer.from([0xff, 0xfe]))

    // A scalar the charset cannot spell is refused, never replaced.
    assert.throws(() => charset.encode('windows-1252', 'yen 円'), /U\+5186/)
    ```

## Performance

One release run of the `charset` Criterion target on one Linux x86_64 container - Intel Xeon @ 2.80 GHz, 4 cores, 15 GiB; rustc 1.97.0, release profile (thin LTO, one codegen unit) - medians of 100 samples, on 2026-10-02. The payload repeats `symbol,desk,price\nAAPL,London,187.23\n`; where it must transcode, the stated share of its bytes is `0xE9` - `é` in every single-byte charset here - and an encode takes the text that decodes to.

| 1 MiB, one byte in twenty above US-ASCII | median | throughput |
| --- | ---: | ---: |
| `decode`, windows-1252 | 1.701 ms | 587.9 MiB/s |
| `decode`, iso-8859-1 | 1.662 ms | 601.5 MiB/s |
| `decode`, utf-16le, the same text in UTF-16 | 3.727 ms | 536.6 MiB/s |

| 1 MiB, all US-ASCII | median | throughput |
| --- | ---: | ---: |
| `decode`, utf-8 | 47.18 us | 20.70 GiB/s |
| `decode`, us-ascii | 109.6 us | 8.91 GiB/s |
| `decode`, iso-8859-1 | 133.8 us | 7.30 GiB/s |
| `decode`, windows-1252 | 132.4 us | 7.38 GiB/s |
| `std::str::from_utf8` (baseline) | 44.16 us | 22.12 GiB/s |

An all-ASCII payload is borrowed, never transcoded: UTF-8 checks it at `std::str::from_utf8`'s pace, and the single-byte charsets' scan for a byte above `0x7F` costs two and a half to three times that and still runs above 7 GiB/s - at least twelve times cheaper than transcoding the same megabyte, which runs at about 600 MiB/s whichever table it reads.

| `encode`, 1 MiB of text | all US-ASCII | 5% above | 50% above |
| --- | ---: | ---: | ---: |
| iso-8859-1 | 68.15 us (14.33 GiB/s) | 1.277 ms (821.9 MiB/s) | 10.86 ms (138.2 MiB/s) |
| windows-1252 | 73.37 us (13.31 GiB/s) | 1.244 ms (843.8 MiB/s) | 10.89 ms (137.7 MiB/s) |
| utf-16le | 2.823 ms (354.2 MiB/s) | 3.355 ms (313.0 MiB/s) | 3.405 ms (440.5 MiB/s) |

An encode borrows the same way and pays, per scalar above US-ASCII, for the reverse table it searches: a twentieth of the scalars cost about a millisecond a megabyte, half of them about eleven. UTF-16 spends two bytes on every scalar, so it neither borrows nor searches, and costs 2.8 to 3.4 ms at every mix.

| 16 MiB of windows-1252, one byte in twenty above US-ASCII | median | throughput |
| --- | ---: | ---: |
| `Charset::decode`, the whole buffer | 45.94 ms | 348.3 MiB/s |
| `decoder()`, pushed 8 KiB at a time | 45.60 ms | 350.9 MiB/s |
| `reader()`, `read_to_string` | 48.77 ms | 328.1 MiB/s |
| `Transcoded`, `read_all_bytes` | 40.41 ms | 396.0 MiB/s |
| `Transcoded`, eight 4 KiB `read_range_bytes` windows | 647.9 us | 48.2 MiB/s |
| the same eight windows over the decoded bytes in a `Buffer` (baseline) | 1.334 us | 22.87 GiB/s |

Over 16 MiB the four doors decode within about a tenth of one another, so a stream costs its carry check rather than its throughput. A `Transcoded` handle seeks by its resume index: eight scattered windows cost a seventieth of decoding the whole, though still far above the same windows over bytes already decoded.

```bash
cargo bench -p yggdryl --bench charset -- '^charset_(decode|borrow)/[^/]+/1MiB$|^charset_encode/[^/]+/(0|5|50)pct/1MiB$|^charset_streaming/[^/]+/16MiB$'
```
