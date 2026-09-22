//! `rust/src/s3/aws/xml.rs`: the S3 vocabulary no caller can name.
//!
//! Every listing, bulk delete and multipart upload the client performs is read
//! and written through this module, and a store's answer is the one input it
//! does not control - so the documents below are spelled the way S3 spells
//! them, malformed ones included.

use yggdryl::internals::s3_answer::{DeleteFailure, ListPage, S3Summary};
use yggdryl::internals::s3_aws_xml::{
    decode_url, parse_complete_multipart, parse_delete_result, parse_list_objects, parse_upload_id,
    render_complete_multipart, render_create_bucket, render_delete_objects,
};
use yggdryl::internals::s3_xml::{escape_text, parse_document};

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
            S3Summary {
                key: "my-image.jpg".to_owned(),
                size: 434_234,
                etag: Some("\"fba9dede5f27731c9771645a39863328\"".to_owned()),
                last_modified: Some("2009-10-12T17:50:30.000Z".to_owned()),
            },
            S3Summary {
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
    let root =
        parse_document(render_complete_multipart(&[(3, "\"e\"".to_owned())]).as_bytes()).unwrap();
    let part = root.child("Part").unwrap();
    assert_eq!(part.child_text("PartNumber"), Some("3"));
    assert_eq!(part.child_text("ETag"), Some("\"e\""));
}
