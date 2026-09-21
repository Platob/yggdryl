//! `rust/src/charset/decoder.rs`: a decode fed in whatever chunks a caller
//! already holds its bytes in.

mod codecs {

    use yggdryl::charset::Charset;

    use yggdryl::Error;

    #[test]
    fn a_chunked_decode_agrees_with_a_whole_one_at_every_split() {
        let text = "Grüße 😀 Ω tail";
        for charset in [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be] {
            let encoded = charset.encode(text).unwrap().into_owned();
            for split in 0..=encoded.len() {
                let mut decoder = charset.decoder();
                let mut decoded = String::new();
                decoder.push(&encoded[..split], &mut decoded).unwrap();
                decoder.push(&encoded[split..], &mut decoded).unwrap();
                decoder.finish().unwrap();
                assert_eq!(decoded, text, "{charset} split at {split}");
            }
        }

        // A wire with a fault refuses at the first fault either way: the same
        // position, and the same reason, whichever chunk the fault lands in. A
        // lead byte broken by another lead is the case a carry can mishandle,
        // since the join reads the held lead alone and keeps the breaker; a
        // sequence the wire cuts short at its end is the case `finish` answers,
        // and it names where that sequence began, not where the input ended.
        for (charset, wire) in [
            (Charset::Utf8, b"\xc3\xc3\xa9".to_vec()),
            (Charset::Utf8, b"\xe2\xe2\x82\xac".to_vec()),
            (Charset::Utf16Le, b"\x00\xd8\x00\xd8\x00\xdc".to_vec()),
            (Charset::Utf8, b"\xf0\x9f\x98\x80\xf0\x9f".to_vec()),
            (Charset::Utf16Le, b"\x00\xd8\x00\xdc\x00\xd8".to_vec()),
            (Charset::Utf16Be, b"\xd8\x00\xdc\x00\xd8".to_vec()),
        ] {
            let whole = charset.decode(&wire).unwrap_err().to_string();
            for split in 0..=wire.len() {
                let mut decoder = charset.decoder();
                let mut decoded = String::new();
                let chunked = decoder
                    .push(&wire[..split], &mut decoded)
                    .and_then(|()| decoder.push(&wire[split..], &mut decoded))
                    .and_then(|()| decoder.finish())
                    .unwrap_err()
                    .to_string();
                assert_eq!(chunked, whole, "{charset} split at {split}");
            }
        }
    }

    #[test]
    fn a_held_lead_byte_broken_by_the_next_chunk_is_refused_as_a_whole_decode_refuses_it() {
        // The held `C3` is refused at position 1, as it always was; the reason
        // named the end of the input, though the input went on. It names the
        // byte that broke the sequence now, as a decode of the whole does.
        let whole = Charset::Utf8.decode(b"a\xc3\xc3\xa9").unwrap_err();
        assert!(matches!(whole, Error::Codec { position: 1, .. }), "{whole}");
        let mut decoder = Charset::Utf8.decoder();
        let mut decoded = String::new();
        decoder.push(b"a\xc3", &mut decoded).unwrap();
        let chunked = decoder.push(b"\xc3\xa9", &mut decoded).unwrap_err();
        assert_eq!(chunked.to_string(), whole.to_string());
        assert!(
            chunked.to_string().ends_with("got 0xc3"),
            "the breaker, not the end of the input: {chunked}"
        );
    }

    #[test]
    fn a_chunked_decode_survives_one_byte_at_a_time() {
        let text = "Grüße 😀 Ω";
        for charset in [Charset::Utf8, Charset::Utf16Be, Charset::Cp1252] {
            let encoded = charset.encode("Gru\u{00df}e").unwrap().into_owned();
            let expected = charset.decode(&encoded).unwrap().into_owned();
            let mut decoder = charset.decoder();
            let mut decoded = String::new();
            for byte in &encoded {
                decoder.push(&[*byte], &mut decoded).unwrap();
            }
            decoder.finish().unwrap();
            assert_eq!(decoded, expected, "{charset}");
        }
        let mut decoder = Charset::Utf8.decoder();
        let mut decoded = String::new();
        decoder.push(text.as_bytes(), &mut decoded).unwrap();
        assert_eq!(decoded, text);
        assert_eq!(decoder.consumed(), text.len() as u64);
    }

    #[test]
    fn a_decode_that_stops_mid_sequence_is_a_refusal_not_silence() {
        let mut decoder = Charset::Utf8.decoder();
        let mut decoded = String::new();
        decoder.push(b"caf\xc3", &mut decoded).unwrap();
        assert_eq!(decoded, "caf");
        assert!(decoder.is_pending());
        assert!(decoder.finish().is_err());
    }

    #[test]
    fn a_chunked_decode_reports_a_broken_sequence_rather_than_holding_it() {
        let mut decoder = Charset::Utf16Le.decoder();
        let mut decoded = String::new();
        // A run of high surrogates never pairs, so it must refuse rather than
        // accumulate without bound.
        let error = (0..8)
            .map(|_| decoder.push(b"\x00\xd8", &mut decoded))
            .find_map(std::result::Result::err);
        assert!(
            error.is_some(),
            "an unpaired run was held rather than refused"
        );
    }

    #[test]
    fn a_chunked_refusal_is_measured_from_the_first_byte_the_decoder_was_fed() {
        // The doc's promise: a position is counted from the first byte fed, not
        // from the start of the chunk that held the byte.
        let mut decoder = Charset::Utf8.decoder();
        let mut decoded = String::new();
        decoder.push(b"abcd", &mut decoded).unwrap();
        let error = decoder.push(b"ef\xffgh", &mut decoded).unwrap_err();
        assert!(
            matches!(error, Error::Codec { position: 6, .. }),
            "4 bytes then index 2 of the next chunk: {error}"
        );

        // Through the carry too: a lead byte held from one chunk is refused with
        // the position it was fed at, once the next chunk shows it leads nothing.
        let mut decoder = Charset::Utf8.decoder();
        let mut decoded = String::new();
        decoder.push(b"abc\xe2", &mut decoded).unwrap();
        assert!(decoder.is_pending());
        let error = decoder.push(b"\x82Z", &mut decoded).unwrap_err();
        assert!(
            matches!(error, Error::Codec { position: 3, .. }),
            "the held lead byte was the fourth byte fed: {error}"
        );
    }
}
