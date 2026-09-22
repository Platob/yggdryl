//! `rust/src/s3/azure/xml.rs`: the Azure vocabulary no caller can name.
//!
//! Azure answers a listing as `EnumerationResults` and says "no next page" by
//! answering an empty `NextMarker` rather than none at all, so both readings
//! are pinned against documents spelled the way the service spells them.

use yggdryl::internals::s3_azure_xml::{parse_list, render_block_list};

fn listing(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><EnumerationResults \
         ServiceEndpoint=\"https://trades.blob.core.windows.net/\" \
         ContainerName=\"lake\">{body}</EnumerationResults>"
    )
}

#[test]
fn a_hierarchical_page_reads_its_blobs_and_its_prefixes() {
    let page = parse_list(
        listing(
            "<Blobs><Blob><Name>lake/part.parquet</Name><Properties>\
             <Last-Modified>Wed, 01 Jan 2020 00:00:00 GMT</Last-Modified>\
             <Etag>0x8D8</Etag><Content-Length>512</Content-Length>\
             </Properties></Blob>\
             <BlobPrefix><Name>lake/year=2026/</Name></BlobPrefix></Blobs>\
             <NextMarker />",
        )
        .as_bytes(),
    )
    .expect("a page");
    assert_eq!(page.objects.len(), 1);
    assert_eq!(page.objects[0].key, "lake/part.parquet");
    assert_eq!(page.objects[0].size, 512);
    assert_eq!(page.objects[0].etag.as_deref(), Some("0x8D8"));
    assert_eq!(page.prefixes, ["lake/year=2026/"]);
    assert!(!page.is_truncated);
    assert_eq!(page.next_continuation_token, None);
}

#[test]
fn a_marker_says_another_page_follows_and_an_empty_one_says_it_does_not() {
    let more =
        parse_list(listing("<Blobs /><NextMarker>2!MTIz</NextMarker>").as_bytes()).expect("a page");
    assert!(more.is_truncated);
    assert_eq!(more.next_continuation_token.as_deref(), Some("2!MTIz"));

    let last =
        parse_list(listing("<Blobs /><NextMarker></NextMarker>").as_bytes()).expect("a page");
    assert!(!last.is_truncated);
}

#[test]
fn a_block_list_names_every_staged_block_in_order() {
    assert_eq!(
        render_block_list(&["AAAAAA==".to_owned(), "AAAAAQ==".to_owned()]),
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>\
         <Latest>AAAAAA==</Latest><Latest>AAAAAQ==</Latest></BlockList>"
    );
}

#[test]
fn a_blob_without_a_name_is_rejected() {
    let error = parse_list(listing("<Blobs><Blob><Properties /></Blob></Blobs>").as_bytes())
        .expect_err("a refusal");
    assert!(error.0.contains("Name"), "{}", error.0);
}
