#[path = "text/format.rs"]
mod format;
#[path = "text/placeholder.rs"]
mod placeholder;
#[path = "text/value.rs"]
mod value;

use std::io::Cursor;
use yggdryl::Value;
use yggdryl::text::{self, Format, Limits};

#[test]
fn borrowed_text_dispatches_without_owned_utf8_staging() {
    let json = "{\"message\":\"héllo\"}";
    assert_eq!(
        text::from_utf8(json, Format::Json).unwrap(),
        text::from_bytes(json.as_bytes(), Format::Json).unwrap()
    );
    assert_eq!(
        text::from_utf8_with_limits(json, Format::Json, Limits::default()).unwrap(),
        text::from_bytes(json.as_bytes(), Format::Json).unwrap()
    );

    let lines = "1\n2\n";
    assert_eq!(
        text::from_utf8(lines, Format::JsonLines).unwrap(),
        Value::from_sequence([Value::from(1_u64), Value::from(2_u64)])
    );
    assert_eq!(
        text::from_utf8_all_with_limits(lines, Format::JsonLines, Limits::default()).unwrap(),
        vec![Value::from(1_u64), Value::from(2_u64)]
    );

    let yaml = "label: café\n---\nlabel: second\n";
    assert_eq!(
        text::from_utf8_all(yaml, Format::Yaml).unwrap(),
        text::from_bytes_all(yaml.as_bytes(), Format::Yaml).unwrap()
    );
}

#[test]
fn all_dispatch_paths_share_exact_document_limits() {
    for (format, input) in [
        (Format::Json, b"1 2".as_slice()),
        (Format::JsonLines, b"1\r\n2".as_slice()),
        (Format::Yaml, b"1\n---\n2\n".as_slice()),
    ] {
        let exact = Limits::new(8, input.len(), 8, 2);
        assert_eq!(
            text::from_bytes_all_with_limits(input, format, exact).unwrap(),
            vec![Value::from(1_u64), Value::from(2_u64)]
        );
        assert_eq!(
            text::from_utf8_all_with_limits(std::str::from_utf8(input).unwrap(), format, exact)
                .unwrap(),
            vec![Value::from(1_u64), Value::from(2_u64)]
        );
        assert_eq!(
            text::from_reader_all_with_limits(Cursor::new(input), format, exact).unwrap(),
            vec![Value::from(1_u64), Value::from(2_u64)]
        );

        let one = Limits::new(8, input.len(), 8, 1);
        assert!(text::from_bytes_all_with_limits(input, format, one).is_err());
        assert!(
            text::from_utf8_all_with_limits(std::str::from_utf8(input).unwrap(), format, one)
                .is_err()
        );
        assert!(text::from_reader_all_with_limits(Cursor::new(input), format, one).is_err());
    }
}
