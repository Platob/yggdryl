//! Shared URI component and platform-path parsing.

use super::*;

pub(super) fn parse_error(target: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position,
        reason: SmolStr::new_static(reason),
    }
}

pub(super) const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

pub(super) const fn is_sub_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

pub(super) const fn is_authority_byte(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delimiter(byte) || matches!(byte, b':' | b'@' | b'[' | b']')
}

pub(super) const fn is_path_byte(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delimiter(byte) || matches!(byte, b'/' | b':' | b'@')
}

pub(super) const fn is_query_fragment_byte(byte: u8) -> bool {
    is_path_byte(byte) || byte == b'?'
}

pub(super) fn validate_component(
    value: &str,
    target: &'static str,
    base: usize,
    allowed: impl Fn(u8) -> bool,
) -> Result<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(parse_error(
                    target,
                    base + index,
                    "percent escape must contain exactly two hexadecimal digits",
                ));
            }
            index += 3;
            continue;
        }
        if !byte.is_ascii() || !allowed(byte) {
            return Err(parse_error(
                target,
                base + index,
                "character is not permitted in this URI component",
            ));
        }
        index += 1;
    }
    Ok(())
}

pub(super) fn normalize_percent_hex(value: &str) -> SmolStr {
    let bytes = value.as_bytes();
    let needs_normalization = bytes.windows(3).any(|window| {
        window[0] == b'%' && (window[1].is_ascii_lowercase() || window[2].is_ascii_lowercase())
    });
    if !needs_normalization {
        return SmolStr::new(value);
    }

    let mut normalized = SmolStrBuilder::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            normalized.push('%');
            normalized.push(char::from(bytes[index + 1].to_ascii_uppercase()));
            normalized.push(char::from(bytes[index + 2].to_ascii_uppercase()));
            index += 3;
        } else {
            normalized.push(char::from(bytes[index]));
            index += 1;
        }
    }
    normalized.into()
}

pub(super) fn validate_optional_component(
    value: Option<SmolStr>,
    target: &'static str,
    allowed: impl Fn(u8) -> bool,
) -> Result<Option<SmolStr>> {
    value
        .map(|value| {
            validate_component(value.as_str(), target, 0, &allowed)?;
            Ok(normalize_percent_hex(value.as_str()))
        })
        .transpose()
}

pub(super) fn normalize_resource_segment(
    value: &str,
    target: &'static str,
    empty_reason: &'static str,
) -> Result<SmolStr> {
    if value.is_empty() {
        return Err(parse_error(target, 0, empty_reason));
    }
    if let Some(position) = value.find('/') {
        return Err(parse_error(
            target,
            position,
            "resource path value must contain exactly one segment",
        ));
    }
    validate_component(value, target, 0, is_path_byte)?;
    Ok(normalize_percent_hex(value))
}

impl FromStr for Uri {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        if value
            .as_bytes()
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"file:"))
        {
            let remainder = &value[5..];
            let hierarchy_end = remainder.find(['?', '#']).unwrap_or(remainder.len());
            let hierarchy = &remainder[..hierarchy_end];
            if hierarchy.contains('\\') {
                let mut normalized = String::with_capacity(value.len());
                normalized.push_str("file:");
                for character in hierarchy.chars() {
                    normalized.push(if character == '\\' { '/' } else { character });
                }
                normalized.push_str(&remainder[hierarchy_end..]);
                return Self::from_str(&normalized);
            }
        }
        if is_windows_drive_absolute(value)
            || value.starts_with("\\\\")
            || (!value.contains(':') && value.contains('\\'))
        {
            return Self::from_path(value);
        }

        // A scheme cannot contain `/`, so a colon that appears after the first
        // separator belongs to the path. Anything without a usable scheme is a
        // filesystem path and falls back to `file:`; `/data/x` becomes
        // `file:///data/x` and `data/x` stays a relative `file:` path.
        let scheme_end = value
            .find(':')
            .filter(|position| !value[..*position].contains('/'));
        let Some(scheme_end) = scheme_end else {
            return Self::from_path(value);
        };
        let scheme = Scheme::from_str(&value[..scheme_end])
            .map_err(|error| offset_parse_error(error, "uri", 0, "invalid URI scheme"))?;
        let remainder_start = scheme_end + 1;
        let remainder = &value[remainder_start..];

        let fragment_offset = remainder.find('#');
        let before_fragment = fragment_offset.map_or(remainder, |position| &remainder[..position]);
        let fragment = fragment_offset
            .map(|position| &remainder[position + 1..])
            .map(|fragment| {
                validate_component(
                    fragment,
                    "uri",
                    remainder_start + fragment_offset.unwrap_or(0) + 1,
                    is_query_fragment_byte,
                )?;
                Ok::<SmolStr, Error>(normalize_percent_hex(fragment))
            })
            .transpose()?;

        let query_offset = before_fragment.find('?');
        let hierarchy =
            query_offset.map_or(before_fragment, |position| &before_fragment[..position]);
        let query = query_offset
            .map(|position| &before_fragment[position + 1..])
            .map(|query| {
                validate_component(
                    query,
                    "uri",
                    remainder_start + query_offset.unwrap_or(0) + 1,
                    is_query_fragment_byte,
                )?;
                Ok::<SmolStr, Error>(normalize_percent_hex(query))
            })
            .transpose()?;

        let (authority, path, has_authority) =
            if let Some(after_marker) = hierarchy.strip_prefix("//") {
                let authority_end = after_marker.find('/').unwrap_or(after_marker.len());
                let authority_text = &after_marker[..authority_end];
                validate_component(
                    authority_text,
                    "uri",
                    remainder_start + 2,
                    is_authority_byte,
                )?;
                let authority = Authority::from_str(authority_text).map_err(|error| {
                    offset_parse_error(error, "uri", remainder_start + 2, "invalid URI authority")
                })?;
                let path_text = &after_marker[authority_end..];
                validate_component(
                    path_text,
                    "uri",
                    remainder_start + 2 + authority_end,
                    is_path_byte,
                )?;
                (authority, UriPath(normalize_percent_hex(path_text)), true)
            } else {
                validate_component(hierarchy, "uri", remainder_start, is_path_byte)?;
                (
                    Authority(SmolStr::new("")),
                    UriPath(normalize_percent_hex(hierarchy)),
                    false,
                )
            };

        Self::from_parts_with_authority(scheme, authority, path, has_authority, query, fragment)
    }
}

impl TryFrom<&Path> for Uri {
    type Error = Error;

    fn try_from(value: &Path) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<PathBuf> for Uri {
    type Error = Error;

    fn try_from(value: PathBuf) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<&PathBuf> for Uri {
    type Error = Error;

    fn try_from(value: &PathBuf) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<Uri> for PathBuf {
    type Error = Error;

    fn try_from(value: Uri) -> Result<Self> {
        value.into_path()
    }
}

impl TryFrom<&Uri> for PathBuf {
    type Error = Error;

    fn try_from(value: &Uri) -> Result<Self> {
        value.clone().into_path()
    }
}

pub(super) fn offset_parse_error(
    error: Error,
    target: &'static str,
    offset: usize,
    fallback: &'static str,
) -> Error {
    match error {
        Error::Parse {
            position, reason, ..
        } => Error::Parse {
            target,
            position: offset + position,
            reason,
        },
        _ => parse_error(target, offset, fallback),
    }
}

pub(super) fn is_windows_drive_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

pub(super) fn encode_file_path(value: &str, prefix_slash: bool, uppercase_drive: bool) -> SmolStr {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = SmolStrBuilder::new();
    if prefix_slash {
        encoded.push('/');
    }
    for (index, mut byte) in value.bytes().enumerate() {
        if uppercase_drive && index == 0 {
            byte = byte.to_ascii_uppercase();
        }
        if byte == b'\\' {
            encoded.push('/');
        } else if byte != b'%' && is_path_byte(byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded.into()
}

pub(super) fn authority_from_file_server(value: &str, source_offset: usize) -> Result<Authority> {
    if value
        .bytes()
        .all(|byte| byte != b'%' && is_authority_byte(byte))
    {
        return Authority::from_str(value).map_err(|error| {
            offset_parse_error(error, "path", source_offset, "invalid UNC server")
        });
    }

    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = SmolStrBuilder::new();
    for byte in value.bytes() {
        if byte != b'%' && is_authority_byte(byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    let encoded: SmolStr = encoded.into();
    validate_authority(encoded.as_str()).map_err(|error| {
        remap_file_authority_error(error, value, source_offset, "invalid UNC server")
    })?;
    Ok(Authority(encoded))
}

pub(super) fn remap_file_authority_error(
    error: Error,
    source: &str,
    source_offset: usize,
    fallback: &'static str,
) -> Error {
    match error {
        Error::Parse {
            position, reason, ..
        } => Error::Parse {
            target: "path",
            position: source_offset + decoded_authority_position(source, position),
            reason,
        },
        _ => parse_error("path", source_offset, fallback),
    }
}

pub(super) fn decoded_authority_position(source: &str, encoded_position: usize) -> usize {
    let mut encoded_offset = 0;
    for (source_offset, byte) in source.bytes().enumerate() {
        let encoded_width = if byte != b'%' && is_authority_byte(byte) {
            1
        } else {
            3
        };
        if encoded_position < encoded_offset + encoded_width {
            return source_offset;
        }
        encoded_offset += encoded_width;
    }
    source.len()
}

pub(super) fn file_path_from_uri(value: &Uri) -> Result<PathBuf> {
    if value.scheme() != &Scheme::FILE {
        return Err(parse_error(
            "path",
            0,
            "only a file URI can be converted to a platform path",
        ));
    }
    if value.path().is_empty() && value.authority().is_empty() {
        return Err(parse_error(
            "path",
            value.scheme().as_str().len() + 1,
            "file URI path must not be empty",
        ));
    }
    if value.query().is_some() || value.fragment().is_some() {
        return Err(parse_error(
            "path",
            value.scheme().as_str().len() + 1 + value.path().as_str().len(),
            "file URI query and fragment components cannot be represented by a path",
        ));
    }

    let path = decode_file_component(value.path().as_str(), "file URI path")?;
    if value.authority().is_empty() {
        if let Some(position) = encoded_windows_drive_position(value.path().as_str(), &path) {
            return Err(parse_error(
                "file URI path",
                position,
                "percent escapes cannot create a Windows drive designator",
            ));
        }
    }
    validate_file_authority_round_trip(value.authority().as_str())?;
    let authority = decode_file_component(value.authority().as_str(), "file URI authority")?;
    let drive_path = path.as_bytes().get(..4).is_some_and(|prefix| {
        prefix[0] == b'/'
            && prefix[1].is_ascii_alphabetic()
            && prefix[2] == b':'
            && prefix[3] == b'/'
    });

    if !authority.is_empty() {
        let mut result = String::with_capacity(2 + authority.len() + path.len());
        result.push_str("//");
        result.push_str(&authority);
        if !path.starts_with('/') {
            result.push('/');
        }
        result.push_str(&path);
        return Ok(PathBuf::from(result));
    }
    if drive_path {
        return Ok(PathBuf::from(&path[1..]));
    }
    Ok(PathBuf::from(path.as_ref()))
}

pub(super) fn encoded_windows_drive_position(encoded: &str, decoded: &str) -> Option<usize> {
    let decoded_bytes = decoded.as_bytes();
    let drive_offset = if is_windows_drive_absolute(decoded) {
        0
    } else if decoded.starts_with('/') && is_windows_drive_absolute(&decoded[1..]) {
        1
    } else {
        return None;
    };
    let encoded_bytes = encoded.as_bytes();
    if encoded_bytes.get(drive_offset) != decoded_bytes.get(drive_offset) {
        return Some(drive_offset);
    }
    if encoded_bytes.get(drive_offset + 1) != Some(&b':') {
        return Some(drive_offset + 1);
    }
    (encoded_bytes.get(drive_offset + 2) != Some(&b'/')).then_some(drive_offset + 2)
}

pub(super) fn validate_file_authority_round_trip(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(parse_error(
                "file URI authority",
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        }
        let Some(high) = hex_value(bytes[index + 1]) else {
            return Err(parse_error(
                "file URI authority",
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        };
        let Some(low) = hex_value(bytes[index + 2]) else {
            return Err(parse_error(
                "file URI authority",
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        };
        if is_authority_byte((high << 4) | low) {
            return Err(parse_error(
                "file URI authority",
                index,
                "escaped ASCII authority syntax cannot round-trip through a path",
            ));
        }
        index += 3;
    }
    Ok(())
}

pub(super) fn decode_file_component<'a>(
    value: &'a str,
    target: &'static str,
) -> Result<Cow<'a, str>> {
    let bytes = value.as_bytes();
    if !bytes.contains(&b'%') {
        return Ok(Cow::Borrowed(value));
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(parse_error(
                target,
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        }
        let Some(high) = hex_value(bytes[index + 1]) else {
            return Err(parse_error(
                target,
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        };
        let Some(low) = hex_value(bytes[index + 2]) else {
            return Err(parse_error(
                target,
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        };
        let byte = (high << 4) | low;
        if matches!(byte, b'/' | b'\\') {
            return Err(parse_error(
                target,
                index,
                "encoded path separators cannot be converted safely",
            ));
        }
        if byte == 0 {
            return Err(parse_error(target, index, "file path must not contain NUL"));
        }
        decoded.push(byte);
        index += 3;
    }
    let decoded = String::from_utf8(decoded).map_err(|error| {
        parse_error(
            target,
            encoded_position_for_decoded_byte(value, error.utf8_error().valid_up_to()),
            "file path percent escapes must decode to UTF-8",
        )
    })?;
    if let Some((position, _)) = decoded
        .char_indices()
        .find(|(_, character)| character.is_control() && *character != '\t')
    {
        return Err(parse_error(
            target,
            encoded_position_for_decoded_byte(value, position),
            "file path must not contain control characters",
        ));
    }
    Ok(Cow::Owned(decoded))
}

pub(super) fn encoded_position_for_decoded_byte(value: &str, decoded_position: usize) -> usize {
    let bytes = value.as_bytes();
    let mut encoded = 0;
    let mut decoded = 0;
    while encoded < bytes.len() && decoded < decoded_position {
        encoded += if bytes[encoded] == b'%' { 3 } else { 1 };
        decoded += 1;
    }
    encoded
}

pub(super) const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn canonicalize_file_drive(
    authority: &mut Authority,
    path: &mut UriPath,
    has_authority: &mut bool,
) {
    if is_file_drive_authority(authority.as_str(), path.as_str()) {
        let authority_bytes = authority.as_str().as_bytes();
        let mut normalized = SmolStrBuilder::new();
        normalized.push('/');
        normalized.push(char::from(authority_bytes[0].to_ascii_uppercase()));
        normalized.push(':');
        normalized.push_str(path.as_str());
        *authority = Authority(SmolStr::new(""));
        *path = UriPath(normalized.into());
        *has_authority = true;
        return;
    }

    let value = path.as_str();
    let drive_offset = if is_windows_drive_absolute(value) {
        Some(0)
    } else if value.starts_with('/') && is_windows_drive_absolute(&value[1..]) {
        Some(1)
    } else {
        None
    };
    let Some(drive_offset) = drive_offset else {
        return;
    };
    let drive = value.as_bytes()[drive_offset].to_ascii_uppercase();
    let needs_leading_slash = drive_offset == 0;
    let needs_uppercase = drive != value.as_bytes()[drive_offset];
    if needs_leading_slash || needs_uppercase {
        let mut normalized = SmolStrBuilder::new();
        if needs_leading_slash {
            normalized.push('/');
        }
        normalized.push_str(&value[..drive_offset]);
        normalized.push(char::from(drive));
        normalized.push_str(&value[drive_offset + 1..]);
        *path = UriPath(normalized.into());
    }
    *has_authority = true;
}

pub(super) fn is_file_drive_authority(authority: &str, path: &str) -> bool {
    let bytes = authority.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && path.starts_with('/')
}
