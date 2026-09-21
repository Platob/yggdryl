//! The subset of Amazon S3's XML this dialect reads and writes.
//!
//! S3 answers listings, bulk deletes, multipart uploads, and every failure
//! with small documents of a fixed shape, and takes three equally small ones
//! as request bodies. The reader is [`crate::object::xml`]; this module
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/object/aws/xml.rs` pins and a caller cannot reach.
    //!
    //! S3's document vocabulary is read and written here and nowhere else, so
    //! the reading of a listing, an upload id, a bulk delete and a late
    //! failure is pinned against documents spelled the way S3 spells them.
    //! Each item forwards, so nothing changes visibility.
    use crate::object::answer::{DeleteFailure, ListPage};
    use crate::object::xml::XmlError;

    /// Read one `ListObjectsV2` page.
    ///
    /// # Errors
    ///
    /// Malformed XML, another root, or a `<Contents>` without a `<Key>` or a
    /// numeric `<Size>`.
    pub fn parse_list_objects(xml: &[u8]) -> Result<ListPage, XmlError> {
        super::parse_list_objects(xml)
    }

    /// `<InitiateMultipartUploadResult><UploadId>`.
    ///
    /// # Errors
    ///
    /// Malformed XML, another root, or a missing or empty `<UploadId>`.
    pub fn parse_upload_id(xml: &[u8]) -> Result<String, XmlError> {
        super::parse_upload_id(xml)
    }

    /// `<CompleteMultipartUploadResult><ETag>`.
    ///
    /// # Errors
    ///
    /// Malformed XML, another root, or the late `<Error>` a 200 can carry.
    pub fn parse_complete_multipart(xml: &[u8]) -> Result<Option<String>, XmlError> {
        super::parse_complete_multipart(xml)
    }

    /// `<DeleteResult>`: the `<Error>` entries a bulk delete answered.
    ///
    /// # Errors
    ///
    /// Malformed XML, another root, or an `<Error>` without a `<Key>`.
    pub fn parse_delete_result(xml: &[u8]) -> Result<Vec<DeleteFailure>, XmlError> {
        super::parse_delete_result(xml)
    }

    /// The `<CreateBucketConfiguration>` body naming `region`.
    pub fn render_create_bucket(region: &str) -> String {
        super::render_create_bucket(region)
    }

    /// The `<Delete>` body listing `keys`.
    pub fn render_delete_objects(keys: &[String], quiet: bool) -> String {
        super::render_delete_objects(keys, quiet)
    }

    /// The `<CompleteMultipartUpload>` body listing `parts`.
    pub fn render_complete_multipart(parts: &[(u32, String)]) -> String {
        super::render_complete_multipart(parts)
    }

    /// `text` with S3's form-encoding undone.
    pub fn decode_url(text: &str) -> String {
        super::decode_url(text)
    }
}
