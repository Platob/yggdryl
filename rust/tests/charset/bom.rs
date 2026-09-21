//! `rust/src/charset/bom.rs`: the byte-order mark - what it names, how long it
//! is, and why a declaration outranks it.

mod codecs {

    use yggdryl::charset::Charset;

    #[test]
    fn a_byte_order_mark_names_its_charset_and_its_own_length() {
        assert_eq!(
            Charset::from_bom(b"\xef\xbb\xbfid"),
            Some((Charset::Utf8, 3))
        );
        assert_eq!(
            Charset::from_bom(b"\xff\xfeA\x00"),
            Some((Charset::Utf16Le, 2))
        );
        assert_eq!(
            Charset::from_bom(b"\xfe\xff\x00A"),
            Some((Charset::Utf16Be, 2))
        );
        assert_eq!(Charset::from_bom(b"id,name"), None);
        assert_eq!(Charset::from_bom(b""), None);

        assert_eq!(Charset::Utf8.bom(), Some(b"\xef\xbb\xbf".as_slice()));
        assert_eq!(Charset::Cp1252.bom(), None);
        // Nothing is stripped on a caller's behalf: the mark is a scalar until a
        // caller decides it is framing.
        assert_eq!(
            Charset::Utf8.decode(b"\xef\xbb\xbfid").unwrap(),
            "\u{FEFF}id"
        );
    }
}

mod handles {

    use yggdryl::holder::Buffer;
    use yggdryl::{Charset, MediaType, Scalar, text};

    #[test]
    fn a_byte_order_mark_is_framing_rather_than_the_first_character() {
        // A UTF-16 document declares nothing and is recognized by its own mark.
        let mut bytes = Charset::Utf16Le.bom().expect("a marked form").to_vec();
        bytes.extend_from_slice(&Charset::Utf16Le.encode("{\"id\":1}").unwrap());
        let source = Buffer::from_bytes(bytes)
            .with_media_type(MediaType::from_str("application/json").unwrap());

        let value = text::from_io(&source).expect("the mark names the charset");
        assert_eq!(value.get_key_str("id").and_then(Scalar::as_i64), Some(1));
    }

    #[test]
    fn a_declared_charset_wins_over_a_mark_the_payload_carries() {
        // The mark is still framing and comes off, but it does not overrule what
        // the handle was told the bytes are.
        let mut bytes = Charset::Utf8.bom().expect("a marked form").to_vec();
        bytes.extend_from_slice(&Charset::Cp1252.encode("{\"fee\":\"€1\"}").unwrap());
        let source = Buffer::from_bytes(bytes).with_media_type(
            MediaType::from_str("application/json")
                .unwrap()
                .with_charset(Charset::Cp1252),
        );

        let value = text::from_io(&source).expect("the declaration decides");
        assert_eq!(
            value.get_key_str("fee").and_then(Scalar::as_str),
            Some("€1")
        );
    }
}
