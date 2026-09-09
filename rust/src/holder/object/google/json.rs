//! The Google JSON API's documents, read into the shapes every store answers.
//!
//! Google is the store of the three that answers JSON, and the reading is the
//! core JSON codec's - there is no second parser here, only the names. Two of
//! those names matter more than the rest: `size`, `generation` and
//! `metageneration` are JSON *strings* rather than numbers, because they are
//! 64-bit and JSON numbers are not, so a reader that takes them as numbers gets
//! nothing at all.

use super::super::answer::{ErrorBody, ListPage, ObjectMeta, ObjectSummary};
use super::super::xml::XmlError;
use crate::{Result, Scalar};

/// Read one `Object` resource as the metadata a handle caches.
///
/// # Errors
///
/// Returns a refusal when the body is not JSON.
pub(crate) fn parse_object(body: &[u8]) -> std::result::Result<ObjectMeta, XmlError> {
    let value = read(body)?;
    Ok(ObjectMeta {
        size: number(&value, "size").unwrap_or(0),
        etag: text(&value, "etag").map(str::to_owned),
        content_type: text(&value, "contentType").map(str::to_owned),
    })
}

/// Read one `Objects` page.
///
/// `prefixes` already end in the delimiter, and `nextPageToken` is the only
/// thing that says another page follows - there is no truncation flag.
///
/// # Errors
///
/// Returns a refusal when the body is not JSON, or an item states no name.
pub(crate) fn parse_list(body: &[u8]) -> std::result::Result<ListPage, XmlError> {
    let value = read(body)?;
    let mut page = ListPage::default();
    if let Some(items) = value.get_key_str("items") {
        for item in items.sequence_iter() {
            let key = text(item, "name")
                .ok_or_else(|| XmlError("an object without a name".to_owned()))?;
            page.objects.push(ObjectSummary {
                key: key.to_owned(),
                size: number(item, "size").unwrap_or(0),
                etag: text(item, "etag").map(str::to_owned),
                last_modified: text(item, "updated").map(str::to_owned),
            });
        }
    }
    if let Some(prefixes) = value.get_key_str("prefixes") {
        page.prefixes.extend(
            prefixes
                .sequence_iter()
                .filter_map(|prefix| prefix.as_str().map(str::to_owned)),
        );
    }
    page.next_continuation_token = text(&value, "nextPageToken").map(str::to_owned);
    page.is_truncated = page.next_continuation_token.is_some();
    Ok(page)
}

/// Read the session URI a resumable upload was started at.
///
/// It is the `Location` header rather than the body, so this only names what a
/// missing one means.
pub(crate) fn missing_session() -> XmlError {
    XmlError("a resumable upload without a Location".to_owned())
}

/// Read an error document, in either of the two shapes Google answers with.
///
/// The rich form carries an `errors[]` whose first `reason` is the code every
/// other store puts in one; the minimal form has only `code` and `message`.
pub(crate) fn parse_error(body: &[u8]) -> Option<ErrorBody> {
    let value = read(body).ok()?;
    let error = value.get_key_str("error")?;
    let first = error
        .get_key_str("errors")
        .and_then(|errors| errors.sequence_iter().next());
    let code = first
        .and_then(|first| text(first, "reason"))
        .map(str::to_owned)
        .or_else(|| number(error, "code").map(|code| code.to_string()))
        .unwrap_or_default();
    Some(ErrorBody {
        code,
        message: text(error, "message").unwrap_or_default().to_owned(),
        request_id: None,
        resource: first
            .and_then(|first| text(first, "location"))
            .map(str::to_owned),
    })
}

/// The object metadata a write sends alongside its bytes.
///
/// # Errors
///
/// Returns a refusal when the metadata will not render, which it cannot.
pub(crate) fn object_metadata(name: &str, headers: &[(String, String)]) -> Result<String> {
    let mut entries: Vec<(&str, Scalar)> = vec![("name", Scalar::from(name))];
    let user: Vec<(String, Scalar)> = headers
        .iter()
        .filter_map(|(header, value)| {
            header
                .strip_prefix("x-goog-meta-")
                .map(|name| (name.to_owned(), Scalar::from(value.as_str())))
        })
        .collect();
    let content_type = headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| Scalar::from(value.as_str()));
    if let Some(content_type) = &content_type {
        entries.push(("contentType", content_type.clone()));
    }
    let metadata = if user.is_empty() {
        None
    } else {
        Some(Scalar::from_record(
            user.iter()
                .map(|(name, value)| (name.as_str(), value.clone())),
        )?)
    };
    if let Some(metadata) = &metadata {
        entries.push(("metadata", metadata.clone()));
    }
    crate::text::json::into_utf8(&Scalar::from_record(entries)?)
}

/// Read a JSON body, naming what it was not.
fn read(body: &[u8]) -> std::result::Result<Scalar, XmlError> {
    crate::text::json::from_bytes(body)
        .map_err(|error| XmlError(format!("expected a JSON document: {error}")))
}

/// One string field of a record.
fn text<'value>(value: &'value Scalar, name: &str) -> Option<&'value str> {
    value.get_key_str(name).and_then(Scalar::as_str)
}

/// One number field, which Google spells as a string when it is 64-bit.
fn number(value: &Scalar, name: &str) -> Option<u64> {
    let held = value.get_key_str(name)?;
    if let Some(text) = held.as_str() {
        return text.trim().parse().ok();
    }
    held.as_u128().and_then(|value| u64::try_from(value).ok())
}
