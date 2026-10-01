//! `rust/src/csv/options.rs`: the dialect a CSV read or write takes, its
//! defaults, its builders and every refusal a setter makes.

use yggdryl::csv::CsvOptions;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::text::LineSep;
use yggdryl::{Error, MimeType};

/// The path and the reason of an `InvalidRecord` refusal.
fn refusal(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other:?}"),
    }
}

#[test]
fn a_line_break_plays_no_role() {
    for (path, refused) in [
        ("$.separator", CsvOptions::new().with_separator(b'\n')),
        ("$.quote", CsvOptions::new().with_quote(Some(b'\r'))),
        ("$.escape", CsvOptions::new().with_escape(Some(b'\n'))),
        ("$.comment", CsvOptions::new().with_comment(Some(b'\r'))),
    ] {
        let (at, reason) = refusal(refused.unwrap_err());
        assert_eq!(at, path);
        assert!(reason.contains("neither a line break"), "{reason}");
    }
}

#[test]
fn a_byte_past_ascii_plays_no_role() {
    let (path, reason) = refusal(CsvOptions::new().with_separator(b'\xE9').unwrap_err());
    assert_eq!(path, "$.separator");
    assert!(reason.starts_with("expected an ASCII byte"), "{reason}");
    assert!(reason.ends_with("got 0xe9"), "{reason}");
}

#[test]
fn two_roles_never_share_a_byte_and_a_refusal_leaves_the_options_as_they_were() {
    let mut options = CsvOptions::new();
    let (path, reason) = refusal(options.set_separator(b'"').unwrap_err());
    assert_eq!(path, "$.separator");
    assert!(reason.ends_with("got '\"', which is the quote"), "{reason}");
    assert_eq!(options.separator(), b',');

    let (path, reason) = refusal(options.set_quote(Some(b',')).unwrap_err());
    assert_eq!(path, "$.quote");
    assert!(
        reason.ends_with("got ',', which is the separator"),
        "{reason}"
    );
    assert_eq!(options.quote(), Some(b'"'));

    options.set_escape(Some(b'\\')).unwrap();
    let (path, reason) = refusal(options.set_comment(Some(b'\\')).unwrap_err());
    assert_eq!(path, "$.comment");
    assert!(
        reason.ends_with("got '\\\\', which is the escape"),
        "{reason}"
    );
    assert_eq!(options.comment(), None);

    options.set_comment(Some(b'#')).unwrap();
    let (path, reason) = refusal(options.set_separator(b'#').unwrap_err());
    assert_eq!(path, "$.separator");
    assert!(
        reason.ends_with("got '#', which is the comment byte"),
        "{reason}"
    );

    // The same byte may move to the role it already holds.
    options.set_separator(b',').unwrap();
    options.set_quote(Some(b'"')).unwrap();
    assert_eq!(
        options,
        CsvOptions::new()
            .with_escape(Some(b'\\'))
            .unwrap()
            .with_comment(Some(b'#'))
            .unwrap()
    );
}

#[test]
fn a_null_spelling_listed_twice_is_refused() {
    let mut options = CsvOptions::new();
    let (path, reason) = refusal(options.set_null_values(["", "NA", ""]).unwrap_err());
    assert_eq!(path, "$.null_values");
    assert_eq!(reason, "expected each null spelling once, got \"\" twice");
    assert_eq!(options.null_values(), [""]);
    options.set_null_values(["NA", "NULL", ""]).unwrap();
    assert_eq!(options.null_values(), ["NA", "NULL", ""]);
    let none = CsvOptions::new()
        .with_null_values::<[&str; 0], &str>([])
        .unwrap();
    assert!(none.null_values().is_empty());
}

#[test]
fn a_sample_of_no_record_is_refused() {
    let mut options = CsvOptions::new();
    let (path, reason) = refusal(options.set_infer_row_size(0).unwrap_err());
    assert_eq!(path, "$.infer_row_size");
    assert_eq!(
        reason,
        "expected at least one record to infer a column's datatype from, got 0"
    );
    assert_eq!(options.infer_row_size(), 1024);
    assert_eq!(options.with_infer_row_size(8).unwrap().infer_row_size(), 8);
}

#[test]
fn the_defaults_and_the_builders() {
    let options = CsvOptions::new();
    assert_eq!(options, CsvOptions::default());
    assert_eq!(options.separator(), b',');
    assert_eq!(options.quote(), Some(b'"'));
    assert_eq!(options.escape(), None);
    assert_eq!(options.comment(), None);
    assert!(options.header());
    assert_eq!(options.null_values(), [""]);
    assert!(!options.trim());
    assert_eq!(options.infer_row_size(), 1024);
    assert_eq!(options.linesep(), &LineSep::LF);
    assert!(options.safe());
    assert_eq!(options.batch_row_size(), Some(65_536));
    assert_eq!(options.batch_byte_size(), Some(64 * 1024 * 1024));
    assert_eq!(options.name(), "row");
    assert_eq!(options.field(), None);

    let tsv = CsvOptions::tsv();
    assert_eq!(tsv.separator(), b'\t');
    assert_eq!(tsv.with_separator(b',').unwrap(), CsvOptions::new());

    let built = CsvOptions::new()
        .with_separator(b';')
        .unwrap()
        .with_quote(Some(b'\''))
        .unwrap()
        .with_escape(Some(b'\\'))
        .unwrap()
        .with_comment(Some(b'#'))
        .unwrap()
        .with_header(false)
        .with_null_values(["NA"])
        .unwrap()
        .with_trim(true)
        .with_infer_row_size(2)
        .unwrap()
        .with_linesep(LineSep::CRLF);
    assert_eq!(built.separator(), b';');
    assert_eq!(built.quote(), Some(b'\''));
    assert_eq!(built.escape(), Some(b'\\'));
    assert_eq!(built.comment(), Some(b'#'));
    assert!(!built.header());
    assert_eq!(built.null_values(), ["NA"]);
    assert!(built.trim());
    assert_eq!(built.infer_row_size(), 2);
    assert_eq!(built.linesep(), &LineSep::CRLF);
    let mut set = CsvOptions::new();
    set.set_header(false);
    set.set_trim(true);
    set.set_linesep(LineSep::CRLF);
    assert!(!set.header());
    assert!(set.trim());
    assert_eq!(set.linesep(), &LineSep::CRLF);
    // No quote at all is a dialect of its own.
    assert_eq!(CsvOptions::new().with_quote(None).unwrap().quote(), None);
}

#[test]
fn record_options_name_the_encoding_by_the_separator() {
    let csv = RecordOptions::for_mime_type(&MimeType::CSV).unwrap();
    assert!(matches!(&csv, RecordOptions::Csv(options) if options.separator() == b','));
    assert_eq!(csv.mime_type(), MimeType::CSV);
    let tsv = RecordOptions::for_mime_type(&MimeType::TSV).unwrap();
    assert!(matches!(&tsv, RecordOptions::Csv(options) if options.separator() == b'\t'));
    assert_eq!(tsv.mime_type(), MimeType::TSV);
    let mut moved = csv.clone();
    moved.set_csv_separator(b'\t').unwrap();
    assert_eq!(moved.mime_type(), MimeType::TSV);
    assert_ne!(moved.stable_hash(), csv.stable_hash());
    assert_eq!(RecordOptions::from(CsvOptions::tsv()), tsv);
}

#[test]
fn record_options_accessors_reach_every_dialect_setting() {
    let mut options = RecordOptions::for_mime_type(&MimeType::CSV).unwrap();
    assert_eq!(options.csv_separator(), Some(b','));
    assert_eq!(options.csv_quote(), Some(Some(b'"')));
    assert_eq!(options.csv_escape(), Some(None));
    assert_eq!(options.csv_comment(), Some(None));
    assert_eq!(options.header(), Some(yggdryl::RecordHeader::Source));
    assert_eq!(options.csv_null_values().map(<[_]>::len), Some(1));
    assert_eq!(options.csv_trim(), Some(false));
    assert_eq!(options.csv_infer_row_size(), Some(1024));

    options.set_csv_separator(b'|').unwrap();
    options.set_csv_quote(Some(b'\'')).unwrap();
    options.set_csv_escape(Some(b'\\')).unwrap();
    options.set_csv_comment(Some(b'#')).unwrap();
    options.set_header(false).unwrap();
    options.set_csv_null_values(["NA", ""]).unwrap();
    options.set_csv_trim(true).unwrap();
    options.set_csv_infer_row_size(16).unwrap();
    assert_eq!(options.csv_separator(), Some(b'|'));
    assert_eq!(options.csv_quote(), Some(Some(b'\'')));
    assert_eq!(options.csv_escape(), Some(Some(b'\\')));
    assert_eq!(options.csv_comment(), Some(Some(b'#')));
    assert_eq!(options.header(), Some(yggdryl::RecordHeader::None));
    assert_eq!(
        options
            .csv_null_values()
            .map(|values| values.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
        Some(vec!["NA", ""])
    );
    assert_eq!(options.csv_trim(), Some(true));
    assert_eq!(options.csv_infer_row_size(), Some(16));

    // The options' own refusals come through unchanged.
    let (path, _) = refusal(options.set_csv_separator(b'\'').unwrap_err());
    assert_eq!(path, "$.separator");
    let (path, _) = refusal(options.set_csv_infer_row_size(0).unwrap_err());
    assert_eq!(path, "$.infer_row_size");
}

#[test]
fn record_options_of_another_encoding_answer_none_and_refuse_a_setting() {
    let mut ipc = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).unwrap();
    assert_eq!(ipc.csv_separator(), None);
    assert_eq!(ipc.csv_quote(), None);
    assert_eq!(ipc.csv_escape(), None);
    assert_eq!(ipc.csv_comment(), None);
    assert_eq!(ipc.header(), None);
    assert_eq!(ipc.csv_null_values(), None);
    assert_eq!(ipc.csv_trim(), None);
    assert_eq!(ipc.csv_infer_row_size(), None);
    for (path, refused) in [
        ("$.separator", ipc.set_csv_separator(b';')),
        ("$.quote", ipc.set_csv_quote(None)),
        ("$.escape", ipc.set_csv_escape(Some(b'\\'))),
        ("$.comment", ipc.set_csv_comment(Some(b'#'))),
        ("$.null_values", ipc.set_csv_null_values(["NA"])),
        ("$.trim", ipc.set_csv_trim(true)),
        ("$.infer_row_size", ipc.set_csv_infer_row_size(2)),
    ] {
        let (at, reason) = refusal(refused.unwrap_err());
        assert_eq!(at, path);
        assert_eq!(
            reason,
            format!(
                "expected CSV options to set {}, got application/vnd.apache.arrow.stream options",
                match path {
                    "$.separator" => "a separator",
                    "$.quote" => "a quote",
                    "$.escape" => "an escape",
                    "$.comment" => "a comment byte",
                    "$.null_values" => "null spellings",
                    "$.trim" => "trimming",
                    _ => "a sample size",
                }
            )
        );
    }
    // The header is one knob a CSV and a workbook share, refused by both names.
    let (at, reason) = refusal(ipc.set_header(false).unwrap_err());
    assert_eq!(at, "$.header");
    assert_eq!(
        reason,
        "expected CSV or Excel options to set a header, got application/vnd.apache.arrow.stream options"
    );
    // And the other way round: a CSV value refuses another encoding's knob.
    let mut csv = RecordOptions::for_mime_type(&MimeType::CSV).unwrap();
    let (path, reason) = refusal(csv.set_avro_block_codec("null").unwrap_err());
    assert_eq!(path, "$.block_codec");
    assert_eq!(
        reason,
        "expected Avro options to set a block codec, got text/csv options"
    );
    assert_eq!(csv.timezone(), None);
    let (_, reason) = refusal(csv.set_timezone(None).unwrap_err());
    assert!(reason.ends_with("got text/csv options"), "{reason}");
}
