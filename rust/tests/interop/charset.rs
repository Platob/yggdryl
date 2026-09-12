//! The charset exchange with an external implementation.
//!
//! `scripts/check_charset_interop.py` drives this target twice around Python's
//! own `codecs` registry: the first run writes every charset's whole
//! repertoire for Python to decode, the second reads what Python encoded. The
//! reading half prints `SKIPPED` when the external corpus is absent - the
//! driver fails on that word - so a skipped half can never read as a pass.
//!
//! A charset is 128 facts per code page and no framing at all, so the exchange
//! is the facts themselves: one line per charset naming it, the bytes it
//! assigns, and the text those bytes are. Self-consistency would prove nothing
//! here - a table wrong in both directions round trips perfectly.

use yggdryl::Charset;

/// Where the exchange files live, shared with the Python driver.
fn exchange_dir() -> std::path::PathBuf {
    let mut path = std::env::current_dir().expect("a working directory");
    // Under `cargo test` the working directory is `rust/`.
    path.push("target");
    path.push("charset-interop");
    path
}

/// The charsets exchanged, and the name Python's registry knows each by.
///
/// The Unicode forms are here too: their repertoire is not a byte range, so
/// the corpus below hands them text rather than bytes, and the exchange is the
/// encoded form in both directions.
const EXCHANGED: [(Charset, &str); 13] = [
    (Charset::Utf8, "utf-8"),
    (Charset::Utf16Le, "utf-16-le"),
    (Charset::Utf16Be, "utf-16-be"),
    (Charset::Ascii, "ascii"),
    (Charset::Latin1, "iso-8859-1"),
    (Charset::Latin2, "iso-8859-2"),
    (Charset::Latin9, "iso-8859-15"),
    (Charset::Cp1250, "cp1250"),
    (Charset::Cp1251, "cp1251"),
    (Charset::Cp1252, "cp1252"),
    (Charset::Cp437, "cp437"),
    (Charset::Cp850, "cp850"),
    (Charset::MacRoman, "mac-roman"),
];

/// The text every charset is asked to carry, whatever its repertoire.
///
/// Only the ASCII part survives every charset; the accented and symbol parts
/// are what tell two Western code pages apart, and the astral scalar is what
/// makes a UTF-16 surrogate pair appear.
const SHARED: &str = "symbol,price\nAAPL,187.23\n";

/// Every byte a charset assigns, ascending, or the shared text for a Unicode
/// form, whose repertoire is not a byte range.
fn corpus(charset: Charset) -> Vec<u8> {
    if charset.is_unicode() {
        return charset
            .encode(&format!("{SHARED}Grüße 😀 Ω\n"))
            .expect("a Unicode form carries every scalar")
            .into_owned();
    }
    (0..=u8::MAX)
        .filter(|byte| charset.scalar_of(*byte).is_some())
        .collect()
}

/// Render one exchange line: the charset, the bytes in hex, and their text.
fn line(charset: Charset, bytes: &[u8], text: &str) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    // The text is JSON-escaped so a line stays one line whatever it holds.
    format!(
        "{}\t{hex}\t{}\n",
        charset.as_str(),
        serde_json::to_string(text).expect("text is always serializable")
    )
}

#[test]
fn writes_a_corpus_for_the_external_decoder() {
    let dir = exchange_dir();
    std::fs::create_dir_all(&dir).expect("the exchange directory");

    let mut rendered = String::new();
    for (charset, _) in EXCHANGED {
        let bytes = corpus(charset);
        let text = charset
            .decode(&bytes)
            .unwrap_or_else(|error| panic!("{charset} refused its own corpus: {error}"));
        // The claim Python checks: these bytes are that text in this charset.
        rendered.push_str(&line(charset, &bytes, &text));
        // And the reverse, here, so a one-directional table cannot pass.
        assert_eq!(
            charset.encode(&text).expect("its own text").as_ref(),
            bytes,
            "{charset} did not round trip its own corpus"
        );
    }
    std::fs::write(dir.join("from-rust.tsv"), rendered).expect("the exchange corpus");
}

#[test]
fn reads_the_corpus_the_external_encoder_wrote() {
    let path = exchange_dir().join("from-python.tsv");
    let Ok(rendered) = std::fs::read_to_string(&path) else {
        println!("SKIPPED: {} is absent", path.display());
        return;
    };

    let mut seen = 0;
    for entry in rendered.lines().filter(|line| !line.is_empty()) {
        let mut parts = entry.split('\t');
        let (Some(name), Some(hex), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            panic!("expected name, hex and text, got {entry:?}");
        };
        assert!(
            parts.next().is_none(),
            "expected three columns in {entry:?}"
        );

        let charset = Charset::from_str(name).expect("a name this crate knows");
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex bytes"))
            .collect();
        let expected: String = serde_json::from_str(text).expect("JSON-escaped text");

        assert_eq!(
            charset.decode(&bytes).expect("what Python encoded"),
            expected,
            "{charset} decoded what Python encoded differently"
        );
        assert_eq!(
            charset
                .encode(&expected)
                .expect("what Python decoded")
                .as_ref(),
            bytes,
            "{charset} encoded what Python decoded differently"
        );
        seen += 1;
    }
    assert_eq!(seen, EXCHANGED.len(), "the external corpus is incomplete");
}
