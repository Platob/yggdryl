//! `rust/src/charset/writer.rs`: a writer that takes UTF-8 and emits one
//! charset's bytes.

mod codecs {

    use std::io::Write;
    use yggdryl::charset::Charset;

    #[test]
    fn a_writer_encodes_text_written_in_any_chunks() {
        let text = "symbol,désk\nAAPL,€1\n";
        for chunk in [1, 2, 3, 7, text.len()] {
            let mut target = Vec::new();
            {
                let mut writer = Charset::Cp1252.writer(&mut target);
                for piece in text.as_bytes().chunks(chunk) {
                    writer.write_all(piece).unwrap();
                }
                writer.finish().unwrap();
            }
            assert_eq!(
                target,
                Charset::Cp1252.encode(text).unwrap().as_ref(),
                "{chunk}"
            );
        }
    }

    #[test]
    fn a_writer_left_mid_scalar_refuses_to_finish() {
        let mut target = Vec::new();
        let mut writer = Charset::Cp1252.writer(&mut target);
        writer.write_all(b"caf\xc3").unwrap();
        assert!(writer.finish().is_err());
    }
}
