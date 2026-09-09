//! The subset of Azure Blob Storage's XML this dialect reads and writes.
//!
//! Azure answers a listing as `EnumerationResults` and a refusal as `Error`,
//! and takes one document as a request body: the block list that commits a
//! staged upload. The reader is [`crate::holder::object::xml`]; this module
//! names the elements and nothing else.

use super::super::answer::{ListPage, ObjectSummary};
use super::super::xml::{XmlError, escape_text, parse_root};

/// Read one page of `List Blobs`.
///
/// A hierarchical listing rolls sub-prefixes up as `BlobPrefix`; a flat one
/// answers only `Blob`. `NextMarker` is present but empty on the last page,
/// which is Azure's way of saying there is no next page.
///
/// # Errors
///
/// Malformed XML, a root other than `EnumerationResults`, or a blob without a
/// name.
pub(crate) fn parse_list(xml: &[u8]) -> Result<ListPage, XmlError> {
    let root = parse_root(xml, "EnumerationResults")?;
    let mut page = ListPage::default();
    if let Some(blobs) = root.child("Blobs") {
        for blob in blobs.children("Blob") {
            let properties = blob.child("Properties");
            page.objects.push(ObjectSummary {
                key: blob.required("Name")?.to_owned(),
                size: properties
                    .and_then(|properties| properties.child_text("Content-Length"))
                    .and_then(|size| size.trim().parse().ok())
                    .unwrap_or(0),
                etag: properties
                    .and_then(|properties| properties.child_text("Etag"))
                    .map(str::to_owned),
                last_modified: properties
                    .and_then(|properties| properties.child_text("Last-Modified"))
                    .map(str::to_owned),
            });
        }
        for prefix in blobs.children("BlobPrefix") {
            page.prefixes.push(prefix.required("Name")?.to_owned());
        }
    }
    page.next_continuation_token = root
        .child_text("NextMarker")
        .map(str::trim)
        .filter(|marker| !marker.is_empty())
        .map(str::to_owned);
    page.is_truncated = page.next_continuation_token.is_some();
    Ok(page)
}

/// The document that commits a staged upload, in the order the blocks go.
///
/// Every id is `Latest`, because this client stages exactly the blocks it is
/// about to commit and never mixes them with what an earlier writer left.
pub(crate) fn render_block_list(ids: &[String]) -> String {
    let mut xml = String::with_capacity(64 + ids.len() * 48);
    xml.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>");
    for id in ids {
        xml.push_str("<Latest>");
        xml.push_str(&escape_text(id));
        xml.push_str("</Latest>");
    }
    xml.push_str("</BlockList>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let more = parse_list(listing("<Blobs /><NextMarker>2!MTIz</NextMarker>").as_bytes())
            .expect("a page");
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
}
