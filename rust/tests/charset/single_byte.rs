//! `rust/src/charset/single_byte.rs`: one byte per scalar, and the ASCII run a
//! decode borrows rather than transcodes.

mod codecs {
    use std::borrow::Cow;

    use yggdryl::charset::Charset;

    use yggdryl::Error;

    /// Every byte a single-byte charset assigns, as that charset spells them.
    fn every_assigned_byte(charset: Charset) -> Vec<u8> {
        (0..=u8::MAX)
            .filter(|byte| charset.scalar_of(*byte).is_some())
            .collect()
    }

    #[test]
    fn ascii_input_is_borrowed_rather_than_transcoded() {
        let plain = b"symbol,price\nAAPL,187.23\n";
        for charset in Charset::ALL {
            if !charset.is_ascii_compatible() {
                continue;
            }
            assert!(
                matches!(charset.decode(plain).unwrap(), Cow::Borrowed(_)),
                "{charset} copied an all-ASCII payload"
            );
            assert!(
                matches!(charset.encode("symbol,price").unwrap(), Cow::Borrowed(_)),
                "{charset} copied all-ASCII text"
            );
        }
    }

    #[test]
    fn a_long_ascii_run_crossing_word_lanes_is_still_borrowed() {
        // The scan reads a machine word at a time and then finishes byte by byte,
        // so every length modulo the lane width has to answer the same.
        for length in 0..64 {
            let payload = vec![b'a'; length];
            assert!(matches!(
                Charset::Cp1252.decode(&payload).unwrap(),
                Cow::Borrowed(_)
            ));
            assert_eq!(
                Charset::Cp1252.decode(&payload).unwrap().len(),
                length,
                "{length}"
            );
        }
    }

    #[test]
    fn a_high_byte_anywhere_in_a_long_run_is_found() {
        for position in 0..40 {
            let mut payload = vec![b'a'; 40];
            payload[position] = 0xE9;
            let decoded = Charset::Latin1.decode(&payload).unwrap();
            assert!(matches!(decoded, Cow::Owned(_)));
            assert_eq!(decoded.chars().nth(position), Some('é'), "{position}");
            assert_eq!(decoded.chars().count(), 40);
        }
    }

    #[test]
    fn every_single_byte_charset_round_trips_its_whole_repertoire() {
        for charset in Charset::ALL {
            if !charset.is_single_byte() || charset == Charset::Ascii {
                continue;
            }
            let bytes = every_assigned_byte(charset);
            let decoded = charset.decode(&bytes).unwrap();
            assert_eq!(decoded.chars().count(), bytes.len(), "{charset}");
            assert_eq!(
                charset.encode(&decoded).unwrap().as_ref(),
                bytes,
                "{charset}"
            );
            for (scalar, byte) in decoded.chars().zip(&bytes) {
                assert_eq!(charset.byte_of(scalar), Some(*byte), "{charset} {scalar}");
                assert_eq!(
                    charset.scalar_of(*byte),
                    Some(scalar),
                    "{charset} {byte:#04x}"
                );
            }
        }
    }

    #[test]
    fn an_unassigned_byte_is_refused_with_its_position() {
        // Windows-1250, -1251 and -1252 are the charsets here that leave bytes
        // unassigned; every other one answers for all 256.
        let error = Charset::Cp1252.decode(b"ok\x81").unwrap_err();
        let Error::Codec {
            format,
            position,
            reason,
        } = error
        else {
            panic!("expected a codec refusal");
        };
        assert_eq!(format, "windows-1252");
        assert_eq!(position, 2);
        assert!(reason.contains("0x81"), "{reason}");
    }

    #[test]
    fn a_scalar_the_charset_has_no_byte_for_is_refused() {
        let error = Charset::Latin1.encode("prix 12€").unwrap_err();
        let Error::Codec {
            format,
            position,
            reason,
        } = error
        else {
            panic!("expected a codec refusal");
        };
        assert_eq!(format, "iso-8859-1");
        assert_eq!(position, 7);
        assert!(reason.contains("U+20AC"), "{reason}");
        // The same scalar is one byte in the charset that has it.
        assert_eq!(
            Charset::Cp1252.encode("prix 12€").unwrap().as_ref(),
            b"prix 12\x80"
        );
    }
}
