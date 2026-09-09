//! What a request to Azure Blob Storage looks like.
//!
//! Azure addresses a blob at `/{container}/{blob}` on the account's own host,
//! selects an operation with `comp` and `restype` query parameters rather than
//! with a path, and assembles a large blob from staged blocks committed as a
//! list rather than from numbered parts. Those three facts are the whole of the
//! dialect.

use super::super::request::Request;

/// The longest a block id may be once decoded, and so the width every id is
/// padded to: for one blob every id must be the same length, which is where a
/// hand-written client usually goes wrong.
const BLOCK_ID_WIDTH: usize = 24;

/// Read one page of a listing.
///
/// The presence of `delimiter` is the only thing that separates a listing of
/// one level from a listing of a whole subtree.
pub(crate) fn list_request<'body>(
    container: &str,
    prefix: &str,
    delimiter: Option<&str>,
    marker: Option<&str>,
    max_results: u16,
) -> Request<'body> {
    let mut request = Request::new("GET", "ListBlobs", container, "")
        .query("restype", "container")
        .query("comp", "list")
        .query("maxresults", max_results.to_string());
    if !prefix.is_empty() {
        request = request.query("prefix", prefix);
    }
    if let Some(delimiter) = delimiter {
        request = request.query("delimiter", delimiter);
    }
    if let Some(marker) = marker {
        request = request.query("marker", marker);
    }
    request
}

/// Stage one block of a blob that is being assembled.
pub(crate) fn put_block_request<'body>(
    container: &str,
    key: &str,
    block: &str,
    body: &'body [u8],
) -> Request<'body> {
    Request::new("PUT", "PutBlock", container, key)
        .query("comp", "block")
        .query("blockid", block)
        .header("content-length", body.len().to_string())
        .body(body)
}

/// Commit the staged blocks, in order, as the blob's contents.
pub(crate) fn put_block_list_request<'body>(
    container: &str,
    key: &str,
    body: &'body [u8],
) -> Request<'body> {
    Request::new("PUT", "PutBlockList", container, key)
        .query("comp", "blocklist")
        .header("content-type", "application/xml")
        .header("content-length", body.len().to_string())
        .body(body)
}

/// Ask whether the container is there.
pub(crate) fn head_container_request<'body>(container: &str) -> Request<'body> {
    Request::new("GET", "GetContainerProperties", container, "").query("restype", "container")
}

/// Create the container.
pub(crate) fn create_container_request<'body>(container: &str) -> Request<'body> {
    Request::new("PUT", "CreateContainer", container, "")
        .query("restype", "container")
        .header("content-length", "0")
}

/// Delete the container itself.
pub(crate) fn delete_container_request<'body>(container: &str) -> Request<'body> {
    Request::new("DELETE", "DeleteContainer", container, "").query("restype", "container")
}

/// Delete several blobs in one round trip, as a batch of sub-requests.
pub(crate) fn batch_request<'body>(
    container: &str,
    body: &'body [u8],
    boundary: &str,
) -> Request<'body> {
    Request::new("POST", "BlobBatch", container, "")
        .query("restype", "container")
        .query("comp", "batch")
        .header(
            "content-type",
            format!("multipart/mixed; boundary={boundary}"),
        )
        .header("content-length", body.len().to_string())
        .body(body)
}

/// The body of a batch of deletes: one embedded HTTP request per blob.
///
/// Each sub-request is signed by the outer request rather than on its own, so
/// what a part carries is the verb, the path, and the version.
pub(crate) fn batch_body(
    account: &str,
    container: &str,
    keys: &[String],
    version: &str,
    boundary: &str,
    account_in_path: bool,
) -> Vec<u8> {
    let mut body = String::new();
    for (index, key) in keys.iter().enumerate() {
        body.push_str(&format!("--{boundary}\r\n"));
        body.push_str("Content-Type: application/http\r\n");
        body.push_str("Content-Transfer-Encoding: binary\r\n");
        body.push_str(&format!("Content-ID: {index}\r\n\r\n"));
        let prefix = if account_in_path {
            format!("/{account}")
        } else {
            String::new()
        };
        body.push_str(&format!(
            "DELETE {prefix}/{}/{} HTTP/1.1\r\n",
            super::super::sigv4::encode_key(container),
            super::super::sigv4::encode_key(key)
        ));
        body.push_str(&format!("x-ms-version: {version}\r\n"));
        body.push_str("Content-Length: 0\r\n\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    body.into_bytes()
}

/// The boundary a batch is framed with, which Azure requires to begin `batch_`.
pub(crate) fn batch_boundary(seed: &[u8]) -> String {
    format!("batch_{:032x}", u128::from(crate::xxhash::xxh3(seed)))
}

/// The id of block `index`, base64 as the wire spells one.
///
/// Every id of a blob has to be the same length, so the number is rendered to
/// a fixed width before it is encoded rather than however wide it happens to
/// be.
pub(crate) fn block_id(index: u32) -> String {
    use base64::Engine as _;
    let raw = format!("{index:0width$}", width = BLOCK_ID_WIDTH);
    base64::engine::general_purpose::STANDARD.encode(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_block_id_of_a_blob_is_the_same_length() {
        let first = block_id(1);
        let last = block_id(9_999_999);
        assert_eq!(first.len(), last.len());
        assert_ne!(first, last);
        // Ids sort the way the blocks are ordered, which is what makes a
        // committed list readable.
        assert!(block_id(2) > block_id(1));
    }

    #[test]
    fn a_batch_boundary_is_the_prefix_azure_demands() {
        assert!(batch_boundary(b"lake").starts_with("batch_"));
    }
}
