//! The subset of Azure Blob Storage's XML this dialect reads and writes.
//!
//! Azure answers a listing as `EnumerationResults` and a refusal as `Error`,
//! and takes one document as a request body: the block list that commits a
//! staged upload. The reader is [`crate::object::xml`]; this module
//! names the elements and nothing else.

use super::super::answer::{ListPage, S3Summary};
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
            page.objects.push(S3Summary {
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/object/azure/xml.rs` pins and a caller cannot reach.
    //!
    //! Azure's document vocabulary is read and written here and nowhere else.
    //! Both items forward, so nothing changes visibility.
    use crate::object::answer::ListPage;
    use crate::object::xml::XmlError;

    /// Read one page of `List Blobs`.
    ///
    /// # Errors
    ///
    /// Malformed XML, another root, or a blob without a name.
    pub fn parse_list(xml: &[u8]) -> Result<ListPage, XmlError> {
        super::parse_list(xml)
    }

    /// The document that commits a staged upload, in the order the blocks go.
    pub fn render_block_list(ids: &[String]) -> String {
        super::render_block_list(ids)
    }
}
