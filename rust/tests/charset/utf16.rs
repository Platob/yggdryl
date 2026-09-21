//! `rust/src/charset/utf16.rs`: UTF-16 in both byte orders, astral scalars and
//! the half units it refuses.

mod codecs {

    use yggdryl::charset::Charset;

    use yggdryl::Error;

    #[test]
    fn utf16_round_trips_both_orders_including_astral_scalars() {
        let text = "Grüße 😀 Ω";
        for charset in [Charset::Utf16Le, Charset::Utf16Be] {
            let encoded = charset.encode(text).unwrap();
            assert_eq!(encoded.len() % 2, 0);
            assert_eq!(charset.decode(&encoded).unwrap(), text, "{charset}");
            assert_eq!(charset.unit_size(), 2);
            assert!(!charset.is_ascii_compatible());
        }
        // The two orders are byte-swapped spellings of the same units.
        let little = Charset::Utf16Le.encode(text).unwrap().into_owned();
        let big = Charset::Utf16Be.encode(text).unwrap().into_owned();
        assert_eq!(little.len(), big.len());
        for pair in 0..little.len() / 2 {
            assert_eq!(little[pair * 2], big[pair * 2 + 1]);
        }
    }

    #[test]
    fn utf16_refuses_a_half_unit_and_a_lone_surrogate() {
        let error = Charset::Utf16Le.decode(b"A\x00B").unwrap_err();
        assert!(
            matches!(&error, Error::Codec { format, position, .. }
                if *format == "utf-16le" && *position == 2),
            "{error:?}"
        );
        let error = Charset::Utf16Le.decode(b"\x00\xd8\x00\x00").unwrap_err();
        assert!(
            matches!(&error, Error::Codec { reason, .. } if reason.contains("surrogate")),
            "{error:?}"
        );
    }
}
