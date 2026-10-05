//! Metadata key canonicalization and typed value validation.

use std::collections::HashSet;

use crate::Scalar;
use crate::expression::{
    Ordering, Projection, TRANSFORM_BY_KEY, TRANSFORM_EXPRESSION_KEY, TRANSFORM_FUNCTION_KEY, Term,
    canonicalize_transform_expression, canonicalize_transform_function,
};
use crate::mime_type::is_http_token_byte;
use crate::protocol::{
    PYTHON_KIND_KEY, PYTHON_MODULE_KEY, PYTHON_QUALNAME_KEY, canonicalize_python_kind,
    validate_python_module, validate_python_qualname,
};
use crate::txhash::{
    DIGEST_TIME_KEY, DIGEST_UNIT_KEY, canonicalize_digest_unit, validate_digest_time,
};
use crate::xxhash::{
    DIGEST_ALGORITHM_KEY, DIGEST_BY_KEY, DIGEST_ROLE_HOLDER, DIGEST_ROLE_KEY,
    canonicalize_digest_algorithm,
};

use super::*;

/// The shape every `by` property has, whatever namespace declares it.
pub(crate) const BY_LIST_SHAPE: &str = "a JSON array of unique non-empty expression texts";

/// The one `DIGEST:by` entry that names every column rather than one term.
pub(crate) const ALL_COLUMNS: &str = "*";

/// Return whether a `by` list is the select-everything spelling.
pub(crate) fn is_all_columns(entries: &[String]) -> bool {
    entries.len() == 1 && entries[0] == ALL_COLUMNS
}

/// Parse the ordered expression texts one `by` property holds.
///
/// One shape serves every namespace that names what it reads, orders or
/// partitions by: a JSON array of expression texts, each read by the
/// grammar its key names - a term for `DIGEST:by` and `TRANSFORM:by`, a
/// projection for `PARTITION:by`, an `order by` key for `SORT:by` - and
/// stored canonically. This reads the array; the caller reads each entry
/// through its grammar. `"*"` is the whole selection rather than a term,
/// so it is the one entry that may not travel beside another.
///
/// # Errors
///
/// Returns an error naming `key` when the text is not that array.
pub(crate) fn parse_by_list(key: &str, value: &str) -> Result<Vec<String>> {
    let document = crate::json::from_utf8(value).map_err(|error| {
        invalid_by_list(
            key,
            format_smolstr!(
                "expected {BY_LIST_SHAPE}, got invalid JSON: {}",
                crate::text::elide_display(&error)
            ),
        )
    })?;
    let Some(values) = document.as_sequence() else {
        return Err(invalid_by_list(
            key,
            crate::text::expected_got(
                BY_LIST_SHAPE,
                format_args!("{:?}", crate::text::elide_to(value, 256)),
            ),
        ));
    };
    let mut entries = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Some(text) = value.as_str() else {
            let actual = crate::json::into_utf8(value)
                .unwrap_or_else(|_| "<unencodable JSON value>".to_owned());
            return Err(invalid_by_list(
                key,
                format_smolstr!(
                    "expected an expression text at index {index}, got {:?}",
                    crate::text::elide_to(&actual, 256)
                ),
            ));
        };
        entries.push(text.to_owned());
    }
    check_by_entries(key, &entries)?;
    Ok(entries)
}

/// Render ordered expression texts through the canonical compact JSON codec.
///
/// # Errors
///
/// [`parse_by_list`] carries the rule.
pub(crate) fn render_by_list<I, P>(key: &str, entries: I) -> Result<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let entries: Vec<String> = entries
        .into_iter()
        .map(|entry| entry.as_ref().to_owned())
        .collect();
    check_by_entries(key, &entries)?;
    let document = Scalar::from_sequence(entries.into_iter().map(Scalar::from));
    crate::json::into_utf8(&document).map_err(|error| {
        invalid_by_list(
            key,
            format_smolstr!(
                "could not encode canonical expression texts: {}",
                crate::text::elide_display(&error)
            ),
        )
    })
}

/// Refuse an empty entry, a repeated one, and the select-everything spelling
/// beside a named one.
fn check_by_entries(key: &str, entries: &[String]) -> Result<()> {
    let mut seen = HashSet::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        if entry.is_empty() {
            return Err(invalid_by_list(
                key,
                format_smolstr!("expected a non-empty expression text at index {index}"),
            ));
        }
        if !seen.insert(entry.as_str()) {
            return Err(invalid_by_list(
                key,
                format_smolstr!("expected each entry once, got {entry:?} twice"),
            ));
        }
    }
    if entries.len() > 1 && entries.iter().any(|entry| entry == ALL_COLUMNS) {
        return Err(invalid_by_list(
            key,
            format_smolstr!(
                "expected {ALL_COLUMNS:?} alone, got it beside {} named entries",
                entries.len() - 1
            ),
        ));
    }
    Ok(())
}

/// Restate an externally supplied `by` list in its one stored spelling: each
/// entry read through `canonical` and written back as that reading spells it.
///
/// # Errors
///
/// [`parse_by_list`] and `canonical` carry the rule.
fn canonicalize_by_list(
    key: &str,
    value: &str,
    canonical: impl Fn(&str) -> Result<String>,
) -> Result<String> {
    let entries = parse_by_list(key, value)?
        .iter()
        .map(|entry| canonical(entry))
        .collect::<Result<Vec<String>>>()?;
    render_by_list(key, entries)
}

/// Read one `by` entry as a term of the expression grammar.
///
/// # Errors
///
/// Returns an error naming `key` when the text is not a term.
pub(crate) fn parse_by_term(key: &str, text: &str) -> Result<Term> {
    text.parse::<Term>()
        .map_err(|error| invalid_by_list(key, format_smolstr!("entry {text:?}: {error}")))
}

/// Read one `by` entry as a term with an optional alias, the shape a
/// partition entry has: `venue`, `years(ts)`, `truncate(name, 4) as prefix`.
///
/// A declared datatype, nullability or `with (...)` metadata is a column
/// declaration rather than a partition entry and is refused by name.
///
/// # Errors
///
/// Returns an error naming `key` when the text is not that shape.
pub(crate) fn parse_by_projection(key: &str, text: &str) -> Result<Projection> {
    let projection = text
        .parse::<Projection>()
        .map_err(|error| invalid_by_list(key, format_smolstr!("entry {text:?}: {error}")))?;
    if projection.dtype().is_some() || projection.nullable().is_some() {
        return Err(invalid_by_list(
            key,
            format_smolstr!(
                "entry {text:?}: expected a term with an optional alias, got a declared datatype"
            ),
        ));
    }
    if !projection.metadata().is_empty() {
        return Err(invalid_by_list(
            key,
            format_smolstr!(
                "entry {text:?}: expected a term with an optional alias, got column metadata"
            ),
        ));
    }
    Ok(projection)
}

/// Read one `by` entry as an `order by` key: a term, then `asc` or `desc`,
/// then `nulls first` or `nulls last`.
///
/// # Errors
///
/// Returns an error naming `key` when the text is not that key.
pub(crate) fn parse_by_ordering(key: &str, text: &str) -> Result<Ordering> {
    text.parse::<Ordering>()
        .map_err(|error| invalid_by_list(key, format_smolstr!("entry {text:?}: {error}")))
}

fn invalid_by_list(key: &str, reason: SmolStr) -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason,
    }
}

/// Return the full `SCHEME:name` key one property is stored under.
pub(crate) fn property_key(scheme: &Scheme, name: &str) -> String {
    let prefix = protocol_metadata_prefix(scheme);
    let mut key = String::with_capacity(prefix.len() + 1 + name.len());
    key.push_str(&prefix);
    key.push(':');
    key.push_str(name);
    key
}

/// Return the same key without allocating for a name of ordinary length.
pub(super) fn property_lookup_key(scheme: &Scheme, name: &str) -> SmolStr {
    format_smolstr!("{}:{name}", protocol_metadata_prefix(scheme))
}

pub(crate) fn property_name<'a>(key: &'a str, scheme: &str) -> Option<&'a str> {
    key.strip_prefix(scheme)?.strip_prefix(':')
}

pub(super) enum PropertyKeyPosition<'a> {
    Before,
    Match(&'a str),
    After,
}

pub(super) fn property_key_position<'a>(key: &'a str, scheme: &str) -> PropertyKeyPosition<'a> {
    let Some(suffix) = key.strip_prefix(scheme) else {
        return PropertyKeyPosition::After;
    };
    let Some(first) = suffix.as_bytes().first() else {
        return PropertyKeyPosition::Before;
    };
    match first.cmp(&b':') {
        std::cmp::Ordering::Less => PropertyKeyPosition::Before,
        std::cmp::Ordering::Equal => PropertyKeyPosition::Match(&suffix[1..]),
        std::cmp::Ordering::Greater => PropertyKeyPosition::After,
    }
}

pub(super) fn validate_entry(key: String, value: String) -> Result<(String, String)> {
    let key = canonicalize_metadata_key(key)?;
    if key.is_empty() {
        return Err(Error::EmptyMetadataKey);
    }
    let value = match key.as_str() {
        ALIAS_KEY | COMMENT_KEY | DESCRIPTION_KEY | DISPLAY_KEY => {
            validate_reserved_text(&key, &value)?;
            value
        }
        LOCATION_KEY => Url::from_str(&value)?.to_string(),
        DIGEST_ALGORITHM_KEY => canonicalize_digest_algorithm(&value)?,
        DIGEST_BY_KEY => canonicalize_by_list(DIGEST_BY_KEY, &value, |entry| {
            if entry == ALL_COLUMNS {
                return Ok(entry.to_owned());
            }
            parse_by_term(DIGEST_BY_KEY, entry).map(|term| term.to_string())
        })?,
        DIGEST_TIME_KEY => {
            validate_digest_time(&value)?;
            value
        }
        DIGEST_UNIT_KEY => canonicalize_digest_unit(&value)?,
        PARTITION_BY_KEY => canonicalize_by_list(PARTITION_BY_KEY, &value, |entry| {
            parse_by_projection(PARTITION_BY_KEY, entry).map(|projection| projection.to_string())
        })?,
        SORT_BY_KEY => canonicalize_by_list(SORT_BY_KEY, &value, |entry| {
            parse_by_ordering(SORT_BY_KEY, entry).map(|ordering| ordering.to_string())
        })?,
        TRANSFORM_EXPRESSION_KEY => {
            canonicalize_transform_expression(TRANSFORM_EXPRESSION_KEY, &value)?
        }
        TRANSFORM_FUNCTION_KEY => canonicalize_transform_function(TRANSFORM_FUNCTION_KEY, &value)?,
        TRANSFORM_BY_KEY => canonicalize_by_list(TRANSFORM_BY_KEY, &value, |entry| {
            parse_by_term(TRANSFORM_BY_KEY, entry).map(|term| term.to_string())
        })?,
        DIGEST_ROLE_KEY => {
            if value.as_str() != DIGEST_ROLE_HOLDER {
                return Err(Error::InvalidMetadataValue {
                    key: SmolStr::new_static(DIGEST_ROLE_KEY),
                    reason: crate::text::expected_got(
                        DIGEST_ROLE_HOLDER,
                        format_args!("{value:?}"),
                    ),
                });
            }
            value
        }
        PYTHON_KIND_KEY => canonicalize_python_kind(&value)?,
        PYTHON_MODULE_KEY => {
            validate_python_module(&value)?;
            value
        }
        PYTHON_QUALNAME_KEY => {
            validate_python_qualname(&value)?;
            value
        }
        FIELD_ENUM_KEY => parse_string_enum(&value)?.into_json(),
        FIELD_INIT_KEY => parse_reserved_bool(FIELD_INIT_KEY, &value)?.to_string(),
        FIELD_PARTITION_KEY => parse_reserved_bool(FIELD_PARTITION_KEY, &value)?.to_string(),
        PARQUET_FIELD_ID_KEY => parse_field_id(&value)?.to_string(),
        _ => {
            if key.starts_with("HTTP:") {
                validate_http_header_value(&key, &value)?;
                if key == HTTP_CONTENT_LENGTH_KEY {
                    return Ok((key, parse_content_length(&value)?.to_string()));
                }
            }
            if let Some((prefix, name)) = key.split_once(':')
                && Scheme::from_str(prefix)
                    .is_ok_and(|scheme| protocol_metadata_prefix(&scheme) == prefix)
            {
                validate_property_part(&key, "property name", name)?;
            }
            value
        }
    };
    Ok((key, value))
}

/// Restate a key in the one spelling it is stored under.
///
/// A protocol key spells its scheme upper case - `FIX:tag` - and a scheme is
/// case-insensitive, so a key written `FIX:tag` or `Fix:tag` is the same key
/// and folds to it on the way in. An HTTP field name folds to lower case
/// beside that, as the protocol defines it. A key with no scheme in front of
/// its colon, or none at all, is stored as written.
pub(super) fn canonicalize_metadata_key(mut key: String) -> Result<String> {
    if let Some((prefix, name)) = http_header_parts(&key) {
        validate_http_header_name(&key, name)?;
        let prefix_len = prefix.len();
        key.make_ascii_lowercase();
        key.replace_range(..prefix_len, HTTP_PREFIX);
        return Ok(key);
    }
    if let Some((end, canonical)) = folded_scheme_prefix(&key) {
        key.replace_range(..end, &canonical);
    }
    Ok(key)
}

/// The key a lookup reads: the stored spelling of `key`, allocated only when
/// `key` is not already it.
pub(super) fn canonical_lookup_key(key: &str) -> Cow<'_, str> {
    if let Some((prefix, name)) = http_header_parts(key) {
        if prefix == HTTP_PREFIX && !name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Cow::Borrowed(key);
        }
        let mut canonical = key.to_owned();
        canonical.make_ascii_lowercase();
        canonical.replace_range(..prefix.len(), HTTP_PREFIX);
        return Cow::Owned(canonical);
    }
    match folded_scheme_prefix(key) {
        Some((end, prefix)) => {
            let mut canonical = String::with_capacity(key.len());
            canonical.push_str(&prefix);
            canonical.push_str(&key[end..]);
            Cow::Owned(canonical)
        }
        None => Cow::Borrowed(key),
    }
}

/// The scheme prefix `key` carries in another case than the stored one, as
/// the byte the prefix ends at and the spelling to store it under.
fn folded_scheme_prefix(key: &str) -> Option<(usize, Cow<'static, str>)> {
    let (prefix, _) = key.split_once(':')?;
    if prefix.is_empty() || !prefix.bytes().any(|byte| byte.is_ascii_lowercase()) {
        return None;
    }
    let scheme = Scheme::from_str(prefix).ok()?;
    let canonical = scheme.metadata_prefix().into_owned();
    Some((prefix.len(), Cow::Owned(canonical)))
}

/// The prefix every HTTP field key carries, HTTPS sharing it.
pub(crate) const HTTP_PREFIX: &str = "HTTP";

pub(super) fn http_header_parts(key: &str) -> Option<(&str, &str)> {
    let (prefix, name) = key.split_once(':')?;
    is_http_metadata_prefix(prefix).then_some((prefix, name))
}

pub(super) fn is_http_metadata_prefix(prefix: &str) -> bool {
    prefix.eq_ignore_ascii_case(Scheme::HTTP.as_str())
        || prefix.eq_ignore_ascii_case(Scheme::HTTPS.as_str())
}

/// The prefix one protocol's keys are stored under: the scheme upper case,
/// HTTPS sharing HTTP's namespace.
pub(crate) fn protocol_metadata_prefix(scheme: &Scheme) -> Cow<'_, str> {
    scheme.metadata_prefix()
}

pub(super) fn validate_http_header_name(key: &str, name: &str) -> Result<()> {
    if !name.is_empty() && name.bytes().all(is_http_token_byte) {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: SmolStr::new_static(
            "HTTP field name must be a non-empty ASCII token without a colon",
        ),
    })
}

pub(super) fn validate_http_header_value(key: &str, value: &str) -> Result<()> {
    if value
        .bytes()
        .all(|byte| byte == b'\t' || (byte >= b' ' && byte != 0x7f))
    {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: SmolStr::new_static(
            "HTTP field value must not contain CR, LF, NUL, DEL, or controls other than HTAB",
        ),
    })
}

pub(crate) fn parse_content_length(value: &str) -> Result<u64> {
    if value.is_empty() || value.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(invalid_content_length());
    }
    value.parse().map_err(|_| invalid_content_length())
}

pub(super) fn invalid_content_length() -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new_static(HTTP_CONTENT_LENGTH_KEY),
        reason: SmolStr::new_static("must be an unsigned 64-bit decimal integer"),
    }
}

/// Parse the enum document a field's ASCII values are named by.
///
/// The stored spelling is the one [`StringEnum::into_json`] renders, so a
/// document that reaches storage reads back as the enum that wrote it.
pub(crate) fn parse_string_enum(value: &str) -> Result<StringEnum> {
    StringEnum::from_json(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(FIELD_ENUM_KEY),
        reason: SmolStr::new(error.to_string()),
    })
}

pub(crate) fn parse_field_id(value: &str) -> Result<i32> {
    crate::integer::integer_from_text_as(value).ok_or_else(|| Error::InvalidMetadataValue {
        key: SmolStr::new_static(PARQUET_FIELD_ID_KEY),
        reason: SmolStr::new_static("must be a signed 32-bit decimal integer"),
    })
}

pub(super) fn validate_reserved_text(key: &str, value: &str) -> Result<()> {
    validate_property_part(key, "value", value)
}

/// Read a reserved boolean metadata value at its intake.
///
/// The text is read by the boolean reader every flag in the crate shares, so
/// `yes`, `True` and `1` are readings; the caller stores the one canonical
/// spelling, which is why nothing past intake reads a flag as text again.
fn parse_reserved_bool(key: &str, value: &str) -> Result<bool> {
    crate::boolean::bool_from_text(value).ok_or_else(|| Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: crate::text::expected_got(
            crate::boolean::BOOLEAN_SPELLINGS,
            format_args!(
                "{:?}",
                crate::text::elide_to(value, crate::text::ERROR_TEXT_LIMIT)
            ),
        ),
    })
}

pub(super) fn validate_property_part(key: &str, label: &str, value: &str) -> Result<()> {
    let reason = if value.is_empty() {
        Some(format!("{label} must not be empty"))
    } else if value.chars().any(char::is_control) {
        Some(format!("{label} must not contain control characters"))
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(Error::InvalidMetadataValue {
            key: SmolStr::new(key),
            reason: SmolStr::new(reason),
        });
    }
    Ok(())
}

pub(crate) fn write_json_string(formatter: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    formatter.write_str("\"")?;
    for character in value.chars() {
        match character {
            '"' => formatter.write_str("\\\"")?,
            '\\' => formatter.write_str("\\\\")?,
            '\n' => formatter.write_str("\\n")?,
            '\r' => formatter.write_str("\\r")?,
            '\t' => formatter.write_str("\\t")?,
            '\u{08}' => formatter.write_str("\\b")?,
            '\u{0c}' => formatter.write_str("\\f")?,
            character if character.is_control() => {
                write!(formatter, "\\u{:04x}", u32::from(character))?;
            }
            character => formatter.write_char(character)?,
        }
    }
    formatter.write_str("\"")
}
