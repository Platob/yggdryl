//! The subset of Amazon S3's XML this dialect reads and writes.
//!
//! S3 answers listings, bulk deletes, multipart uploads, and every failure
//! with small documents of a fixed shape, and takes three equally small ones
//! as request bodies. The reader is [`crate::holder::object::xml`]; this module
//! names the elements and nothing else.
//!
//! Keys and prefixes are percent-decoded only when the page carries
//! `<EncodingType>url</EncodingType>`. S3 then applies form-encoding rules -
//! a space arrives as `+`, everything outside the unreserved set as `%XX`
//! over its UTF-8 bytes - so both are undone, as botocore (`unquote_plus`)
//! and the Java SDK (`URLDecoder`) do.

use super::super::answer::{DeleteFailure, ListPage, ObjectSummary};
use super::super::xml::{XmlError, error_body, escape_text, parse_document, parse_root};

/// Read one `ListObjectsV2` page.
///
/// # Errors
///
/// Malformed XML, a root other than `<ListBucketResult>`, or a `<Contents>`
/// without a `<Key>` or a numeric `<Size>`.
pub(crate) fn parse_list_objects(xml: &[u8]) -> Result<ListPage, XmlError> {
    let root = parse_root(xml, "ListBucketResult")?;
    let encoded = root
        .child_text("EncodingType")
        .is_some_and(|value| value.trim() == "url");
    let decode = |text: &str| {
        if encoded {
            decode_url(text)
        } else {
            text.to_owned()
        }
    };
    let mut page = ListPage::default();
    for contents in root.children("Contents") {
        let size = contents.required("Size")?;
        let size = size
            .trim()
            .parse()
            .map_err(|_| XmlError(format!("<Size> is not a number: `{size}`")))?;
        page.objects.push(ObjectSummary {
            key: decode(contents.required("Key")?),
            size,
            etag: contents.child_text("ETag").map(str::to_owned),
            last_modified: contents.child_text("LastModified").map(str::to_owned),
        });
    }
    for common in root.children("CommonPrefixes") {
        page.prefixes.push(decode(common.required("Prefix")?));
    }
    page.is_truncated = root
        .child_text("IsTruncated")
        .is_some_and(|value| value.trim() == "true");
    page.next_continuation_token = root
        .child_text("NextContinuationToken")
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    Ok(page)
}

/// `<InitiateMultipartUploadResult><UploadId>`.
///
/// # Errors
///
/// Malformed XML, another root, or a missing or empty `<UploadId>`.
pub(crate) fn parse_upload_id(xml: &[u8]) -> Result<String, XmlError> {
    let root = parse_root(xml, "InitiateMultipartUploadResult")?;
    let id = root.required("UploadId")?;
    if id.is_empty() {
        return Err(XmlError("empty <UploadId>".to_owned()));
    }
    Ok(id.to_owned())
}

/// `<CompleteMultipartUploadResult><ETag>`.
///
/// # Errors
///
/// Malformed XML or another root. S3 can answer 200 and still fail late,
/// with an `<Error>` body: that is an `Err` carrying `code: message`.
pub(crate) fn parse_complete_multipart(xml: &[u8]) -> Result<Option<String>, XmlError> {
    let root = parse_document(xml)?;
    match root.name() {
        "CompleteMultipartUploadResult" => Ok(root.child_text("ETag").map(str::to_owned)),
        "Error" => {
            let error = error_body(&root);
            Err(XmlError(format!("{}: {}", error.code, error.message)))
        }
        other => Err(XmlError(format!(
            "expected <CompleteMultipartUploadResult>, found <{other}>"
        ))),
    }
}

/// `<DeleteResult>`: the `<Error>` entries (quiet mode omits `<Deleted>`).
///
/// # Errors
///
/// Malformed XML, another root, or an `<Error>` entry without a `<Key>`.
pub(crate) fn parse_delete_result(xml: &[u8]) -> Result<Vec<DeleteFailure>, XmlError> {
    let root = parse_root(xml, "DeleteResult")?;
    root.children("Error")
        .map(|error| {
            Ok(DeleteFailure {
                key: error.required("Key")?.to_owned(),
                code: error.child_text("Code").unwrap_or_default().to_owned(),
                message: error.child_text("Message").unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// `<AssumeRoleResponse><AssumeRoleResult><Credentials>`.
///
/// # Errors
///
/// Malformed XML, another root, or an answer without an access key or secret.
pub(crate) fn parse_assumed_credentials(xml: &[u8]) -> Result<AssumedCredentials, XmlError> {
    let root = parse_root(xml, "AssumeRoleResponse")?;
    let found = root
        .child("AssumeRoleResult")
        .and_then(|result| result.child("Credentials"))
        .ok_or_else(|| XmlError("missing <AssumeRoleResult><Credentials>".to_owned()))?;
    Ok(AssumedCredentials {
        access_key_id: found.required("AccessKeyId")?.to_owned(),
        secret_access_key: found.required("SecretAccessKey")?.to_owned(),
        session_token: found
            .child_text("SessionToken")
            .unwrap_or_default()
            .to_owned(),
        expiration: found.child_text("Expiration").map(str::to_owned),
    })
}

/// One credential set STS handed back.
pub(crate) struct AssumedCredentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: String,
    pub(crate) expiration: Option<String>,
}

/// `<CreateBucketConfiguration><LocationConstraint>{region}</LocationConstraint></CreateBucketConfiguration>`.
pub(crate) fn render_create_bucket(region: &str) -> String {
    format!(
        "<CreateBucketConfiguration><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
        escape_text(region)
    )
}

/// `<Delete><Quiet>true</Quiet><Object><Key>..</Key></Object>...</Delete>`
/// with escaping; `<Quiet>` is omitted when not quiet, its default.
pub(crate) fn render_delete_objects(keys: &[String], quiet: bool) -> String {
    let mut xml = String::from("<Delete>");
    if quiet {
        xml.push_str("<Quiet>true</Quiet>");
    }
    for key in keys {
        xml.push_str("<Object><Key>");
        xml.push_str(&escape_text(key));
        xml.push_str("</Key></Object>");
    }
    xml.push_str("</Delete>");
    xml
}

/// `<CompleteMultipartUpload><Part><PartNumber>n</PartNumber><ETag>..</ETag></Part>...</CompleteMultipartUpload>`,
/// parts in the order given (S3 requires ascending part numbers).
pub(crate) fn render_complete_multipart(parts: &[(u32, String)]) -> String {
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        xml.push_str("<Part><PartNumber>");
        xml.push_str(&number.to_string());
        xml.push_str("</PartNumber><ETag>");
        xml.push_str(&escape_text(etag));
        xml.push_str("</ETag></Part>");
    }
    xml.push_str("</CompleteMultipartUpload>");
    xml
}

/// `text` with form-encoding undone: `+` is a space and `%XX` a byte. A
/// malformed escape stays as spelled and a result that is not UTF-8 leaves
/// the whole text as it stands, so a key S3 gave us is never silently
/// mangled.
fn decode_url(text: &str) -> String {
    if !text.contains(['%', '+']) {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => match hex_byte(&bytes[index + 1..index + 3]) {
                Some(byte) => {
                    decoded.push(byte);
                    index += 3;
                }
                None => {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            },
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

/// One `%XX` pair as the byte it spells.
fn hex_byte(pair: &[u8]) -> Option<u8> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    Some(digit(*pair.first()?)? * 16 + digit(*pair.get(1)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XMLNS: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

    fn listing(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"{XMLNS}\">\n<Name>bucket</Name>\n{body}\n</ListBucketResult>\n"
        )
    }

    fn keys(page: &ListPage) -> Vec<&str> {
        page.objects
            .iter()
            .map(|object| object.key.as_str())
            .collect()
    }

    #[test]
    fn a_plain_listing_yields_objects_in_document_order() {
        let xml = listing(
            "<Prefix></Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>
             <Contents>
               <Key>my-image.jpg</Key>
               <LastModified>2009-10-12T17:50:30.000Z</LastModified>
               <ETag>\"fba9dede5f27731c9771645a39863328\"</ETag>
               <Size>434234</Size>
               <StorageClass>STANDARD</StorageClass>
             </Contents>
             <Contents>
               <Key>notes/</Key>
               <Size>0</Size>
             </Contents>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(
            page.objects,
            vec![
                ObjectSummary {
                    key: "my-image.jpg".to_owned(),
                    size: 434_234,
                    etag: Some("\"fba9dede5f27731c9771645a39863328\"".to_owned()),
                    last_modified: Some("2009-10-12T17:50:30.000Z".to_owned()),
                },
                ObjectSummary {
                    key: "notes/".to_owned(),
                    size: 0,
                    etag: None,
                    last_modified: None,
                },
            ]
        );
        assert!(page.prefixes.is_empty());
        assert!(!page.is_truncated);
        assert_eq!(page.next_continuation_token, None);
    }

    #[test]
    fn common_prefixes_and_the_continuation_token_are_read() {
        let xml = listing(
            "<Prefix>photos/</Prefix><Delimiter>/</Delimiter><KeyCount>3</KeyCount><MaxKeys>3</MaxKeys>
             <IsTruncated>true</IsTruncated>
             <NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken>
             <Contents><Key>photos/index.html</Key><Size>12</Size></Contents>
             <CommonPrefixes><Prefix>photos/2006/</Prefix></CommonPrefixes>
             <CommonPrefixes><Prefix>photos/2007/</Prefix></CommonPrefixes>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["photos/index.html"]);
        assert_eq!(page.prefixes, ["photos/2006/", "photos/2007/"]);
        assert!(page.is_truncated);
        assert_eq!(
            page.next_continuation_token.as_deref(),
            Some("1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=")
        );
    }

    #[test]
    fn an_empty_listing_is_an_empty_page() {
        let xml = listing(
            "<Prefix/><KeyCount>0</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated>",
        );
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap(),
            ListPage::default()
        );
    }

    #[test]
    fn url_encoding_type_decodes_keys_and_prefixes() {
        // EncodingType sits after the entries, where S3 puts it.
        let xml = listing(
            "<Contents><Key>photos%2F2006+January%2Fsample%281%29.jpg</Key><Size>1</Size></Contents>
             <Contents><Key>caf%C3%A9%2B%25.txt</Key><Size>2</Size></Contents>
             <CommonPrefixes><Prefix>a+b%2F</Prefix></CommonPrefixes>
             <EncodingType>url</EncodingType>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(
            keys(&page),
            ["photos/2006 January/sample(1).jpg", "café+%.txt"]
        );
        assert_eq!(page.prefixes, ["a b/"]);
    }

    #[test]
    fn without_encoding_type_keys_stay_verbatim() {
        let xml = listing(
            "<Contents><Key>a+b%2F</Key><Size>1</Size></Contents>
             <Contents><Key>tom &amp; jerry.txt</Key><Size>2</Size></Contents>
             <Contents><Key>café/日本.txt</Key><Size>3</Size></Contents>
             <CommonPrefixes><Prefix>a+b%2F</Prefix></CommonPrefixes>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["a+b%2F", "tom & jerry.txt", "café/日本.txt"]);
        assert_eq!(page.prefixes, ["a+b%2F"]);
    }

    #[test]
    fn url_decoding_leaves_malformed_escapes_and_non_utf8_alone() {
        assert_eq!(decode_url("100%25+%zz%"), "100% %zz%");
        assert_eq!(decode_url("%FF"), "%FF");
        assert_eq!(decode_url("plain"), "plain");
        assert_eq!(decode_url("%2"), "%2");
    }

    #[test]
    fn entities_and_character_references_decode() {
        let xml = listing(
            "<Contents><Key>&amp;&lt;&gt;&quot;&apos;&#65;&#x42;&#x1F600;&#x00e9;</Key><Size>1</Size></Contents>",
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["&<>\"'AB😀é"]);
    }

    #[test]
    fn cdata_comments_and_processing_instructions_are_handled() {
        let xml = format!(
            "\u{FEFF}<?xml version=\"1.0\"?><!-- header --><?pi data?>
             <s3:ListBucketResult xmlns:s3=\"{XMLNS}\">
               <s3:Contents><!-- inside --><s3:Key><![CDATA[a<b&c]]>.txt</s3:Key><?x?><s3:Size> 7 </s3:Size></s3:Contents>
               <s3:IsTruncated>false</s3:IsTruncated>
             </s3:ListBucketResult><!-- trailer -->
             "
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["a<b&c.txt"]);
        assert_eq!(page.objects[0].size, 7);
    }

    #[test]
    fn attributes_may_hold_the_closing_bracket() {
        let xml = format!(
            "<ListBucketResult xmlns=\"{XMLNS}\" note='a>b' other = \"c/>d\"><Contents><Key>k</Key><Size>1</Size></Contents></ListBucketResult>"
        );
        let page = parse_list_objects(xml.as_bytes()).unwrap();
        assert_eq!(keys(&page), ["k"]);
    }

    #[test]
    fn a_different_root_is_rejected() {
        let error = parse_list_objects(b"<Error><Code>NoSuchBucket</Code></Error>").unwrap_err();
        assert_eq!(error.0, "expected <ListBucketResult>, found <Error>");
    }

    #[test]
    fn contents_without_a_key_or_a_numeric_size_are_rejected() {
        let xml = listing("<Contents><Size>1</Size></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Contents> without <Key>"
        );
        let xml = listing("<Contents><Key>k</Key></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Contents> without <Size>"
        );
        let xml = listing("<Contents><Key>k</Key><Size>big</Size></Contents>");
        assert_eq!(
            parse_list_objects(xml.as_bytes()).unwrap_err().0,
            "<Size> is not a number: `big`"
        );
    }

    #[test]
    fn malformed_documents_are_rejected() {
        let malformed: [(&[u8], &str); 12] = [
            (b"", "expected the root element at byte 0"),
            (b"   ", "expected the root element at byte 3"),
            (b"<a><b></a>", "<b> closed by </a> at byte 9"),
            (b"<a>", "<a> never closed at byte 3"),
            (b"<a>&nope;</a>", "unknown reference `&nope;` at byte 3"),
            (b"<a>&amp</a>", "unterminated reference `&amp` at byte 3"),
            (b"<a>&#xD800;</a>", "unknown reference `&#xD800;` at byte 3"),
            (b"<a>&#0;</a>", "unknown reference `&#0;` at byte 3"),
            (b"<a>x</a>tail", "content after the root element at byte 8"),
            (b"<a><!-- open </a>", "unterminated comment at byte 7"),
            (
                b"<a x=1></a>",
                "expected a quoted attribute value at byte 5",
            ),
            (
                b"<a>\xFF</a>",
                "not UTF-8: invalid utf-8 sequence of 1 bytes from index 3",
            ),
        ];
        for (xml, message) in malformed {
            assert_eq!(parse_document(xml).unwrap_err().0, message);
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let open = "<a>".repeat(40);
        let close = "</a>".repeat(40);
        let error = parse_document(format!("{open}{close}").as_bytes()).unwrap_err();
        assert!(error.0.starts_with("elements nested deeper than 32"));
        assert!(
            parse_document(format!("{}{}", "<a>".repeat(30), "</a>".repeat(30)).as_bytes()).is_ok()
        );
    }

    #[test]
    fn the_upload_id_is_read() {
        let xml = format!(
            "<InitiateMultipartUploadResult xmlns=\"{XMLNS}\"><Bucket>b</Bucket><Key>k</Key>
             <UploadId>VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA</UploadId></InitiateMultipartUploadResult>"
        );
        assert_eq!(
            parse_upload_id(xml.as_bytes()).unwrap(),
            "VXBsb2FkIElEIGZvciA2aWWpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA"
        );
        let missing = parse_upload_id(b"<InitiateMultipartUploadResult/>").unwrap_err();
        assert_eq!(
            missing.0,
            "<InitiateMultipartUploadResult> without <UploadId>"
        );
        let empty = parse_upload_id(
            b"<InitiateMultipartUploadResult><UploadId/></InitiateMultipartUploadResult>",
        );
        assert_eq!(empty.unwrap_err().0, "empty <UploadId>");
        assert!(parse_upload_id(b"<Error><Code>NoSuchBucket</Code></Error>").is_err());
    }

    #[test]
    fn a_completed_upload_answers_its_etag_even_behind_keepalive_padding() {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n\n\n<CompleteMultipartUploadResult xmlns=\"{XMLNS}\">
             <Location>https://b.s3.amazonaws.com/k</Location><Bucket>b</Bucket><Key>k</Key>
             <ETag>\"3858f62230ac3c915f300c664312c11f-9\"</ETag></CompleteMultipartUploadResult>"
        );
        assert_eq!(
            parse_complete_multipart(xml.as_bytes()).unwrap().as_deref(),
            Some("\"3858f62230ac3c915f300c664312c11f-9\"")
        );
        assert_eq!(
            parse_complete_multipart(b"<CompleteMultipartUploadResult/>").unwrap(),
            None
        );
    }

    #[test]
    fn a_late_failure_in_a_200_body_is_an_error() {
        let xml = "\n\n<Error><Code>InternalError</Code><Message>We encountered an internal error. Please try again.</Message></Error>";
        let error = parse_complete_multipart(xml.as_bytes()).unwrap_err();
        assert_eq!(
            error.0,
            "InternalError: We encountered an internal error. Please try again."
        );
        assert_eq!(
            parse_complete_multipart(listing("").as_bytes())
                .unwrap_err()
                .0,
            "expected <CompleteMultipartUploadResult>, found <ListBucketResult>"
        );
    }

    #[test]
    fn delete_failures_are_read_and_deleted_entries_ignored() {
        let xml = format!(
            "<DeleteResult xmlns=\"{XMLNS}\"><Deleted><Key>a</Key></Deleted>
             <Error><Key>b &amp; c</Key><Code>AccessDenied</Code><Message>Access Denied</Message></Error>
             <Deleted><Key>d</Key></Deleted>
             <Error><Key>e</Key><Code>InternalError</Code><Message>Try again</Message></Error></DeleteResult>"
        );
        assert_eq!(
            parse_delete_result(xml.as_bytes()).unwrap(),
            vec![
                DeleteFailure {
                    key: "b & c".to_owned(),
                    code: "AccessDenied".to_owned(),
                    message: "Access Denied".to_owned(),
                },
                DeleteFailure {
                    key: "e".to_owned(),
                    code: "InternalError".to_owned(),
                    message: "Try again".to_owned(),
                },
            ]
        );
        assert!(parse_delete_result(b"<DeleteResult/>").unwrap().is_empty());
        assert_eq!(
            parse_delete_result(b"<DeleteResult><Error><Code>x</Code></Error></DeleteResult>")
                .unwrap_err()
                .0,
            "<Error> without <Key>"
        );
    }

    #[test]
    fn create_bucket_names_the_region() {
        assert_eq!(
            render_create_bucket("eu-west-1"),
            "<CreateBucketConfiguration><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>"
        );
        assert!(render_create_bucket("<x>").contains("&lt;x&gt;"));
    }

    #[test]
    fn delete_objects_escapes_keys_and_honors_quiet() {
        let keys = ["a&b<c>\"d'e".to_owned(), "plain".to_owned()];
        assert_eq!(
            render_delete_objects(&keys, true),
            "<Delete><Quiet>true</Quiet><Object><Key>a&amp;b&lt;c&gt;&quot;d&apos;e</Key></Object><Object><Key>plain</Key></Object></Delete>"
        );
        assert_eq!(render_delete_objects(&[], false), "<Delete></Delete>");
    }

    #[test]
    fn complete_multipart_lists_parts_with_escaped_etags() {
        let parts = [(1, "\"a\"".to_owned()), (2, "\"b&c\"".to_owned())];
        assert_eq!(
            render_complete_multipart(&parts),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;a&quot;</ETag></Part><Part><PartNumber>2</PartNumber><ETag>&quot;b&amp;c&quot;</ETag></Part></CompleteMultipartUpload>"
        );
    }

    #[test]
    fn escape_text_covers_the_five_characters() {
        assert_eq!(escape_text("&<>\"' ok é"), "&amp;&lt;&gt;&quot;&apos; ok é");
        assert_eq!(escape_text(""), "");
    }

    #[test]
    fn rendered_bodies_scan_back_to_the_same_text() {
        let keys = ["a&b<c>\"d'e/ф".to_owned(), " lead and trail ".to_owned()];
        let root = parse_document(render_delete_objects(&keys, true).as_bytes()).unwrap();
        let scanned: Vec<&str> = root
            .children("Object")
            .map(|object| object.child_text("Key").unwrap())
            .collect();
        assert_eq!(scanned, keys);
        assert_eq!(root.child_text("Quiet"), Some("true"));
        let root = parse_document(render_complete_multipart(&[(3, "\"e\"".to_owned())]).as_bytes())
            .unwrap();
        let part = root.child("Part").unwrap();
        assert_eq!(part.child_text("PartNumber"), Some("3"));
        assert_eq!(part.child_text("ETag"), Some("\"e\""));
    }
}
