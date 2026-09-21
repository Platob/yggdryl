//! `rust/src/utf8.rs`: the transcriber no caller can name.
//!
//! `transcribe_into` is the reading every legacy-charset door goes through,
//! and what it counts is the whole contract, so it is reached through
//! `yggdryl::internals`. Everything a caller can observe is beside it here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::utf8::transcribe_into;

    #[test]
    fn a_transcription_counts_the_bytes_it_read_as_windows_1252() {
        fn read(input: &[u8]) -> (String, usize) {
            let mut text = String::new();
            let count = transcribe_into(input, &mut text);
            (text, count)
        }
        assert_eq!(read(b"caf\xe9"), (String::from("caf\u{e9}"), 1));
        // A character a byte limit cut in two: the two bytes that are left, not
        // a pending sequence and not `U+FFFD`.
        assert_eq!(read(b"58=\xE2\x82"), (String::from("58=\u{e2}\u{201A}"), 2));
        assert_eq!(read(b"symbol,price"), (String::from("symbol,price"), 0));
        // Valid UTF-8 above US-ASCII is kept, and counts nothing.
        assert_eq!(
            read("Gr\u{fc}\u{df}e \u{20AC}".as_bytes()),
            (String::from("Gr\u{fc}\u{df}e \u{20AC}"), 0)
        );
        assert_eq!(read(b""), (String::new(), 0));
        // It appends: what the target held stays in front of the reading.
        let mut text = String::from("58=");
        assert_eq!(transcribe_into(b"caf\xe9", &mut text), 1);
        assert_eq!(text, "58=caf\u{e9}");
    }
}

mod codecs {
    use std::borrow::Cow;

    use yggdryl::charset::Charset;

    use yggdryl::Error;

    #[test]
    fn utf8_is_validated_and_located() {
        assert_eq!(Charset::Utf8.decode("café".as_bytes()).unwrap(), "café");
        let error = Charset::Utf8.decode(b"caf\xc3").unwrap_err();
        assert!(
            matches!(&error, Error::Codec { format, position, reason }
                if *format == "utf-8" && *position == 3 && reason.contains("end of the input")),
            "{error:?}"
        );
        let error = Charset::Utf8.decode(b"caf\xff!").unwrap_err();
        assert!(
            matches!(error, Error::Codec { position: 3, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn valid_utf8_is_borrowed_under_utf8_and_under_us_ascii() {
        // A US-ASCII declaration is a UTF-8 declaration with a narrower promise,
        // so the borrow `transcribe` takes for UTF-8 it takes for US-ASCII too,
        // where `decode` still refuses the byte above `0x7F`.
        for charset in [Charset::Utf8, Charset::Ascii] {
            assert!(
                matches!(
                    charset.transcribe("caf\u{e9}".as_bytes()),
                    Cow::Borrowed("caf\u{e9}")
                ),
                "{charset}"
            );
            assert!(
                matches!(charset.transcribe(b"caf\xe9"), Cow::Owned(text) if text == "caf\u{e9}"),
                "{charset}"
            );
        }
        assert!(Charset::Ascii.decode("caf\u{e9}".as_bytes()).is_err());
    }
}
