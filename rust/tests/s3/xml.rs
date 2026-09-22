//! `rust/src/s3/xml.rs`: the scanner no caller can name.
//!
//! The refusal document is the one shape both XML stores answer with, so it is
//! read here rather than in either dialect. What each dialect reads on top of
//! this scanner is pinned in `rust/tests/s3/aws/xml.rs` and
//! `rust/tests/s3/azure/xml.rs`.

use yggdryl::internals::s3_answer::ErrorBody;
use yggdryl::internals::s3_xml::parse_error;

/// The namespace an S3 document declares; Azure's differs and neither is
/// read, which is the point.
const NS: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

#[test]
fn an_error_document_is_read_with_its_ids() {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
         <Error xmlns=\"{NS}\"><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message>
         <Key>missing.txt</Key><RequestId>4442587FB7D0A2F9</RequestId><HostId>abc=</HostId></Error>"
    );
    assert_eq!(
        parse_error(xml.as_bytes()),
        Some(ErrorBody {
            code: "NoSuchKey".to_owned(),
            message: "The specified key does not exist.".to_owned(),
            request_id: Some("4442587FB7D0A2F9".to_owned()),
            resource: None,
        })
    );
    let xml = "<Error><Code>AccessDenied</Code><Message>Access Denied</Message><Resource>/bucket/key</Resource></Error>";
    assert_eq!(
        parse_error(xml.as_bytes()).unwrap().resource.as_deref(),
        Some("/bucket/key")
    );
    assert_eq!(parse_error(b"<Error/>").unwrap().code, "");
}

#[test]
fn bytes_that_are_not_an_error_document_answer_none() {
    assert_eq!(parse_error(b""), None);
    assert_eq!(
        parse_error(b"<html><body>502 Bad Gateway</body></html>"),
        None
    );
    assert_eq!(parse_error(b"not xml at all"), None);
    assert_eq!(parse_error(b"<ListBucketResult />"), None);
    assert_eq!(parse_error(b"<Error><Code>Unclosed</Error>"), None);
}
