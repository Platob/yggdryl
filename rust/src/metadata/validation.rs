//! Metadata key canonicalization and typed value validation.

use std::collections::HashSet;

use crate::Scalar;
use crate::expression::Function;
use crate::txhash::{
    DIGEST_TIME_KEY, DIGEST_UNIT_KEY, canonicalize_digest_unit, validate_digest_time,
};
use crate::types::protocol::{
    PYTHON_KIND_KEY, PYTHON_MODULE_KEY, PYTHON_QUALNAME_KEY, canonicalize_python_kind,
    validate_python_module, validate_python_qualname,
};
use crate::xxhash::{
    DIGEST_ALGORITHM_KEY, DIGEST_ROLE_HOLDER, DIGEST_ROLE_KEY, DIGEST_SOURCES_KEY,
    canonicalize_digest_algorithm,
};

use super::*;

/// The shape every `sources` property has, whatever namespace declares it.
pub(crate) const SOURCE_LIST_SHAPE: &str = "a JSON array of unique non-empty field path strings";

/// The one source spelling that names every field rather than one path.
pub(crate) const ALL_SOURCES: &str = "*";

/// Return whether a source list is the select-everything spelling.
pub(crate) fn is_all_sources(sources: &[String]) -> bool {
    sources.len() == 1 && sources[0] == ALL_SOURCES
}

/// Parse the ordered field paths one `sources` property names.
///
/// One shape serves every namespace that names its inputs, so a `digest:` and
/// a `partition:` list are read, written and refused identically. `["*"]` is
/// the whole selection rather than a path, so it is the one entry that may not
/// travel beside another: a list naming both everything and one column states
/// no order for the rest.
///
/// # Errors
///
/// Returns an error naming `key` when the text is not that array.
pub(crate) fn parse_source_list(key: &str, value: &str) -> Result<Vec<String>> {
    let document = crate::text::json::from_utf8(value).map_err(|error| {
        invalid_source_list(
            key,
            format_smolstr!(
                "expected {SOURCE_LIST_SHAPE}, got invalid JSON: {}",
                crate::text::elide_display(&error)
            ),
        )
    })?;
    let Some(values) = document.as_sequence() else {
        return Err(invalid_source_list(
            key,
            crate::text::expected_got(
                SOURCE_LIST_SHAPE,
                format_args!("{:?}", crate::text::elide_to(value, 256)),
            ),
        ));
    };
    let mut sources = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Some(path) = value.as_str() else {
            let actual = crate::text::json::into_utf8(value)
                .unwrap_or_else(|_| "<unencodable JSON value>".to_owned());
            return Err(invalid_source_list(
                key,
                format_smolstr!(
                    "expected a field path string at index {index}, got {:?}",
                    crate::text::elide_to(&actual, 256)
                ),
            ));
        };
        if path.is_empty() {
            return Err(invalid_source_list(
                key,
                format_smolstr!("expected a non-empty field path string at index {index}"),
            ));
        }
        if !seen.insert(path) {
            return Err(invalid_source_list(
                key,
                format_smolstr!("expected each field path once, got {path:?} twice"),
            ));
        }
        sources.push(path.to_owned());
    }
    reject_mixed_all(key, &sources)?;
    Ok(sources)
}

/// Render ordered sources through the canonical compact JSON codec.
///
/// # Errors
///
/// [`parse_source_list`] carries the rule.
pub(crate) fn render_source_list<I, P>(key: &str, sources: I) -> Result<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let sources: Vec<String> = sources
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect();
    let mut seen = HashSet::with_capacity(sources.len());
    for (index, path) in sources.iter().enumerate() {
        if path.is_empty() {
            return Err(invalid_source_list(
                key,
                format_smolstr!("expected a non-empty field path string at index {index}"),
            ));
        }
        if !seen.insert(path.as_str()) {
            return Err(invalid_source_list(
                key,
                format_smolstr!("expected each field path once, got {path:?} twice"),
            ));
        }
    }
    reject_mixed_all(key, &sources)?;
    let document = Scalar::from_sequence(sources.into_iter().map(Scalar::from));
    crate::text::json::into_utf8(&document).map_err(|error| {
        invalid_source_list(
            key,
            format_smolstr!(
                "could not encode canonical field paths: {}",
                crate::text::elide_display(&error)
            ),
        )
    })
}

/// Restate externally supplied source JSON in its one stored spelling.
///
/// # Errors
///
/// [`parse_source_list`] carries the rule.
pub(crate) fn canonicalize_source_list(key: &str, value: &str) -> Result<String> {
    render_source_list(key, parse_source_list(key, value)?)
}

/// Refuse the select-everything spelling beside a named path.
fn reject_mixed_all(key: &str, sources: &[String]) -> Result<()> {
    if sources.len() > 1 && sources.iter().any(|source| source == ALL_SOURCES) {
        return Err(invalid_source_list(
            key,
            format_smolstr!(
                "expected {ALL_SOURCES:?} alone, got it beside {} named path(s)",
                sources.len() - 1
            ),
        ));
    }
    Ok(())
}

fn invalid_source_list(key: &str, reason: SmolStr) -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason,
    }
}

/// Resolve the one-argument transform a stored `partition:transform` names.
///
/// The vocabulary is the expression grammar's own [`Function`] set, so a
/// derived partition column and a predicate over the same value share one
/// implementation. Only a function of a single argument is a transform of one
/// column; the parameterized ones are not one yet.
///
/// # Errors
///
/// Returns an error naming `key` when the text is no such function.
pub(crate) fn parse_partition_transform(key: &str, value: &str) -> Result<Function> {
    let function = Function::from_name(value).ok_or_else(|| Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: crate::text::expected_got(
            format_args!("one of {}", Function::vocabulary()),
            format_args!("{:?}", crate::text::elide_to(value, 64)),
        ),
    })?;
    let (least, most) = function.arity();
    if least > 1 || most < 1 {
        return Err(Error::InvalidMetadataValue {
            key: SmolStr::new(key),
            reason: format_smolstr!(
                "expected a transform of one argument, got {value} taking {least} to {most}"
            ),
        });
    }
    Ok(function)
}

/// Restate an externally supplied transform in its one canonical spelling.
///
/// # Errors
///
/// [`parse_partition_transform`] carries the rule.
pub(crate) fn canonicalize_partition_transform(key: &str, value: &str) -> Result<String> {
    Ok(parse_partition_transform(key, value)?.as_str().to_owned())
}

/// Return the full `scheme:name` key one property is stored under.
pub(crate) fn property_key(scheme: &Scheme, name: &str) -> String {
    let prefix = protocol_metadata_prefix(scheme);
    let mut key = String::with_capacity(prefix.len() + 1 + name.len());
    key.push_str(prefix);
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
        DIGEST_SOURCES_KEY => canonicalize_source_list(DIGEST_SOURCES_KEY, &value)?,
        DIGEST_TIME_KEY => {
            validate_digest_time(&value)?;
            value
        }
        DIGEST_UNIT_KEY => canonicalize_digest_unit(&value)?,
        PARTITION_SOURCES_KEY => canonicalize_source_list(PARTITION_SOURCES_KEY, &value)?,
        PARTITION_TRANSFORM_KEY => {
            canonicalize_partition_transform(PARTITION_TRANSFORM_KEY, &value)?
        }
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
            if key.starts_with("http:") {
                validate_http_header_value(&key, &value)?;
                if key == HTTP_CONTENT_LENGTH_KEY {
                    return Ok((key, parse_content_length(&value)?.to_string()));
                }
            }
            if let Some((prefix, name)) = key.split_once(':') {
                if Scheme::from_str(prefix).is_ok_and(|scheme| scheme.as_str() == prefix) {
                    validate_property_part(&key, "property name", name)?;
                }
            }
            value
        }
    };
    Ok((key, value))
}

pub(super) fn canonicalize_metadata_key(mut key: String) -> Result<String> {
    if let Some((prefix, name)) = http_header_parts(&key) {
        validate_http_header_name(&key, name)?;
        let prefix_len = prefix.len();
        key.replace_range(..prefix_len, Scheme::HTTP.as_str());
        key.make_ascii_lowercase();
    }
    Ok(key)
}

pub(super) fn canonical_http_lookup_key(key: &str) -> Cow<'_, str> {
    let Some((prefix, _)) = http_header_parts(key) else {
        return Cow::Borrowed(key);
    };
    if prefix == Scheme::HTTP.as_str() && !key.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Cow::Borrowed(key);
    }
    let mut canonical = key.to_owned();
    canonical.replace_range(..prefix.len(), Scheme::HTTP.as_str());
    canonical.make_ascii_lowercase();
    Cow::Owned(canonical)
}

pub(super) fn http_header_parts(key: &str) -> Option<(&str, &str)> {
    let (prefix, name) = key.split_once(':')?;
    is_http_metadata_prefix(prefix).then_some((prefix, name))
}

pub(super) fn is_http_metadata_prefix(prefix: &str) -> bool {
    prefix.eq_ignore_ascii_case(Scheme::HTTP.as_str())
        || prefix.eq_ignore_ascii_case(Scheme::HTTPS.as_str())
}

pub(crate) fn protocol_metadata_prefix(scheme: &Scheme) -> &str {
    if scheme == &Scheme::HTTPS {
        Scheme::HTTP.as_str()
    } else {
        scheme.as_str()
    }
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

pub(super) const fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
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
    value.parse().map_err(|_| Error::InvalidMetadataValue {
        key: SmolStr::new_static(PARQUET_FIELD_ID_KEY),
        reason: SmolStr::new_static("must be a signed 32-bit decimal integer"),
    })
}

pub(super) fn validate_reserved_text(key: &str, value: &str) -> Result<()> {
    validate_property_part(key, "value", value)
}

/// Parse a reserved boolean metadata value.
///
/// Reserved booleans are stored in exactly one canonical spelling so a reader
/// never has to guess between `true`, `True`, `1`, and `yes`.
pub(crate) fn parse_reserved_bool(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(Error::InvalidMetadataValue {
            key: SmolStr::new(key),
            reason: crate::text::expected_got("true or false", format_args!("{other:?}")),
        }),
    }
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
