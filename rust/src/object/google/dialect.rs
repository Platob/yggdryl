//! What a request to Google Cloud Storage looks like.
//!
//! Google's JSON API puts every object below `/storage/v1`, names the object in
//! the path with every `/` escaped as `%2F`, and switches metadata for bytes
//! with `alt=media`. Uploads go to a second prefix, `/upload/storage/v1`, and a
//! large one is a resumable session rather than numbered parts. Those four
//! facts are the whole of the dialect; everything else is the backend's.

use super::super::options::ObjectOptions;
use super::super::request::Request;

/// Where the JSON API's object and bucket resources live.
const API: &str = "/storage/v1";
/// Where a media upload is posted.
const UPLOAD: &str = "/upload/storage/v1";
/// Where a batch of sub-requests is posted.
pub(crate) const BATCH: &str = "/batch/storage/v1";
/// The granularity every resumable chunk but the last is a multiple of.
pub(crate) const CHUNK_GRANULARITY: u64 = 256 * 1024;

/// The path of one object's JSON resource.
///
/// Every `/` in the name is escaped, because the name is one path segment: it
/// is `b/{bucket}/o/{name}`, not a path below `o`.
pub(crate) fn object_path(bucket: &str, key: &str) -> String {
    format!("{API}/b/{}/o/{}", segment(bucket), segment(key))
}

/// The path of one bucket's JSON resource.
pub(crate) fn bucket_path(bucket: &str) -> String {
    format!("{API}/b/{}", segment(bucket))
}

/// The path a listing is read from.
pub(crate) fn listing_path(bucket: &str) -> String {
    format!("{API}/b/{}/o", segment(bucket))
}

/// The path an upload is posted to.
pub(crate) fn upload_path(bucket: &str) -> String {
    format!("{UPLOAD}/b/{}/o", segment(bucket))
}

/// One path segment, with every reserved byte escaped - `/` included.
fn segment(value: &str) -> String {
    super::super::sigv4::encode_query_component(value)
}

/// Read one object's metadata: the JSON resource, not its bytes.
pub(crate) fn head_request<'body>(bucket: &str, key: &str) -> Request<'body> {
    Request::new("GET", "objects.get", bucket, key)
        .target(object_path(bucket, key))
        // Only the fields a handle caches; the rest is a larger body for
        // nothing.
        .query("fields", "size,updated,etag,contentType,generation")
        .query("prettyPrint", "false")
}

/// Read one object's bytes.
pub(crate) fn get_request<'body>(bucket: &str, key: &str) -> Request<'body> {
    Request::new("GET", "objects.get", bucket, key)
        .target(object_path(bucket, key))
        .query("alt", "media")
}

/// Remove one object.
pub(crate) fn delete_request<'body>(bucket: &str, key: &str) -> Request<'body> {
    Request::new("DELETE", "objects.delete", bucket, key).target(object_path(bucket, key))
}

/// Read one page of a listing.
pub(crate) fn list_request<'body>(
    bucket: &str,
    prefix: &str,
    delimiter: Option<&str>,
    page_token: Option<&str>,
    max_results: u16,
) -> Request<'body> {
    let mut request = Request::new("GET", "objects.list", bucket, "")
        .target(listing_path(bucket))
        .query("maxResults", max_results.to_string())
        .query("prettyPrint", "false");
    if !prefix.is_empty() {
        request = request.query("prefix", prefix);
    }
    if let Some(delimiter) = delimiter {
        request = request.query("delimiter", delimiter);
    }
    if let Some(token) = page_token {
        request = request.query("pageToken", token);
    }
    request
}

/// Write one object whole, carrying its metadata in the same request.
///
/// `multipart/related` is what the official clients send for a small write,
/// because it is the one shape that puts the bytes and the metadata in one
/// round trip.
pub(crate) fn put_request<'body>(
    bucket: &str,
    key: &str,
    body: &'body [u8],
    boundary: &str,
) -> Request<'body> {
    Request::new("POST", "objects.insert", bucket, key)
        .target(upload_path(bucket))
        .query("uploadType", "multipart")
        .query("name", key)
        .header(
            "content-type",
            format!("multipart/related; boundary=\"{boundary}\""),
        )
        .body(body)
}

/// The body of a `multipart/related` upload: the metadata, then the bytes.
///
/// The JSON part must come first, and the framing is exact - no leading and no
/// trailing newline beyond the separators themselves.
pub(crate) fn multipart_body(
    metadata: &str,
    content_type: &str,
    bytes: &[u8],
    boundary: &str,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(bytes.len() + metadata.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"content-type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(format!("content-type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
    body
}

/// A boundary no body can contain, derived from the body itself.
///
/// A random one would do; a digest of the payload is deterministic, which is
/// what lets a test assert the request bytes.
pub(crate) fn boundary(bytes: &[u8]) -> String {
    format!("yggdryl{:016x}", crate::xxhash::xxh3(bytes))
}

/// Begin a resumable upload, which answers a session to send chunks to.
pub(crate) fn initiate_request<'body>(
    bucket: &str,
    key: &str,
    metadata: &'body [u8],
    content_type: &str,
    total: u64,
) -> Request<'body> {
    Request::new("POST", "objects.insert", bucket, key)
        .target(upload_path(bucket))
        .query("uploadType", "resumable")
        .query("name", key)
        .header("content-type", "application/json; charset=UTF-8")
        .header("x-upload-content-type", content_type)
        .header("x-upload-content-length", total.to_string())
        .body(metadata)
}

/// Send one chunk of a resumable upload.
///
/// `start` and `last` are inclusive byte offsets, and every chunk but the last
/// is a multiple of [`CHUNK_GRANULARITY`].
pub(crate) fn chunk_request<'body>(
    session: &str,
    bucket: &str,
    key: &str,
    body: &'body [u8],
    start: u64,
    total: u64,
) -> Request<'body> {
    let last = start + body.len() as u64 - 1;
    Request::new("PUT", "objects.insert", bucket, key)
        .absolute(session)
        .header("content-range", format!("bytes {start}-{last}/{total}"))
        .body(body)
}

/// Abandon a resumable session, releasing what it holds.
pub(crate) fn cancel_request<'body>(session: &str, bucket: &str, key: &str) -> Request<'body> {
    Request::new("DELETE", "objects.insert", bucket, key).absolute(session)
}

/// Ask whether the bucket is there.
pub(crate) fn head_bucket_request<'body>(bucket: &str) -> Request<'body> {
    Request::new("GET", "buckets.get", bucket, "")
        .target(bucket_path(bucket))
        .query("fields", "name")
        .query("prettyPrint", "false")
}

/// Create the bucket, which needs the project that will own it.
pub(crate) fn create_bucket_request<'body>(
    bucket: &str,
    project: &str,
    body: &'body [u8],
) -> Request<'body> {
    Request::new("POST", "buckets.insert", bucket, "")
        .target(format!("{API}/b"))
        .query("project", project)
        .header("content-type", "application/json; charset=UTF-8")
        .body(body)
}

/// Delete the bucket itself.
pub(crate) fn delete_bucket_request<'body>(bucket: &str) -> Request<'body> {
    Request::new("DELETE", "buckets.delete", bucket, "").target(bucket_path(bucket))
}

/// Delete several objects in one round trip, as an HTTP batch.
pub(crate) fn batch_request<'body>(
    bucket: &str,
    body: &'body [u8],
    boundary: &str,
) -> Request<'body> {
    Request::new("POST", "storage.objects.delete", bucket, "")
        .absolute_path(BATCH)
        .header(
            "content-type",
            format!("multipart/mixed; boundary=\"{boundary}\""),
        )
        .body(body)
}

/// The body of a batch of deletes: one embedded HTTP request per key.
pub(crate) fn batch_body(bucket: &str, keys: &[String], boundary: &str) -> Vec<u8> {
    let mut body = String::new();
    for (index, key) in keys.iter().enumerate() {
        body.push_str(&format!("--{boundary}\r\n"));
        body.push_str("Content-Type: application/http\r\n");
        body.push_str(&format!("Content-ID: <yggdryl-{index}>\r\n\r\n"));
        body.push_str(&format!(
            "DELETE {} HTTP/1.1\r\n\r\n\r\n",
            object_path(bucket, key)
        ));
    }
    body.push_str(&format!("--{boundary}--"));
    body.into_bytes()
}

/// The query every request carries when the options ask for it.
///
/// Requester pays and quota attribution are per-request facts on Google, where
/// the other two stores put them in a header or nowhere.
pub(crate) fn common_query(options: &ObjectOptions) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(project) = options.google().user_project() {
        query.push(("userProject".to_owned(), project.to_owned()));
    }
    if let Some(project) = options.google().quota_project() {
        query.push(("quotaUser".to_owned(), project.to_owned()));
    }
    if let Some(acl) = options.google().predefined_acl() {
        query.push(("predefinedAcl".to_owned(), acl.to_owned()));
    }
    query
}
