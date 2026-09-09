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

/// Spell one raw name as a single URI path segment.
///
/// Everything a path segment admits is kept, so `year=2024` stays readable,
/// and `/` is escaped rather than kept: a name a backend gave us is one
/// segment, and letting a slash through would make it two.
///
/// This is the door raw text comes through: [`Url::join_path`] spells one
/// platform component with it, and a backend that names resources with raw
/// text - the S3 one - spells one object name.
pub(crate) fn percent_encode_segment(value: &str) -> Cow<'_, str> {
    percent_encode(value, |byte| is_path_byte(byte) && byte != b'/')
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

/// Return whether one segment is structure rather than a name.
///
/// `.` and `..` address a directory, so a path that gains one addresses
/// something other than the resource the caller was naming.
pub(super) fn is_dot_segment(value: &str) -> bool {
    matches!(value, "." | "..")
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
    if is_dot_segment(value) {
        return Err(parse_error(
            target,
            0,
            "a dot segment addresses a directory rather than naming a resource",
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
        // A one-letter scheme and a Windows drive are spelled the same way, so
        // the drive reading is taken wherever nothing else can be meant. A
        // backslash settles it outright - URI syntax carries none - and so does
        // a single slash. Only `X://` is ambiguous, and there the two slashes
        // are the authority marker: `a://host/p` has to stay the URI that
        // `from_parts` builds from the one-letter scheme `a`.
        let bytes = value.as_bytes();
        let drive_designator = is_windows_drive_absolute(value)
            && (bytes[2] == b'\\' || !(bytes.get(3) == Some(&b'/') || value.contains(['?', '#'])));
        if drive_designator
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
    if value.query.is_some() || value.fragment.is_some() {
        // The offset names the delimiter in the URI the caller wrote, so the
        // authority marker and the authority itself count toward it.
        let authority_len = if value.has_authority() {
            2 + value.authority().as_str().len()
        } else {
            0
        };
        return Err(parse_error(
            "path",
            value.scheme().as_str().len() + 1 + authority_len + value.path().as_str().len(),
            "file URI query and fragment components cannot be represented by a path",
        ));
    }

    let path = decode_file_component(value.path().as_str(), "file URI path")?;
    if let Some(position) = escaped_dot_segment_position(value.path().as_str(), &path) {
        return Err(parse_error(
            "file URI path",
            position,
            "percent escapes cannot create a dot segment",
        ));
    }
    if value.authority().is_empty() {
        // With no authority to spell it, a path opening on two slashes would
        // come back out as `//server/share`: the UNC form, naming a host this
        // URI never carried. `Uri::from_path` cannot produce one, so refusing
        // is what keeps the two directions each other's inverse.
        if path.starts_with("//") {
            return Err(parse_error(
                "file URI path",
                0,
                "a path opening on two slashes would name a UNC server this URI has no authority for",
            ));
        }
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

/// Find an escape that turns a path segment into `.` or `..`.
///
/// Every structural view - `parts`, `normalize`, `segments_under` - reads the
/// encoded path, so an escaped dot segment is invisible to all of them and then
/// resolves as a real one the moment a platform walks the path: `lake/%2E%2E/x`
/// reads as a name under `lake` and addresses its parent. That is the same
/// reason an escaped separator is refused, so the escape is refused here rather
/// than quietly deciding which of the two readings the caller meant.
fn escaped_dot_segment_position(encoded: &str, decoded: &str) -> Option<usize> {
    if !encoded.contains('%') {
        return None;
    }
    let mut decoded_offset = 0;
    for segment in decoded.split('/') {
        if matches!(segment, "." | "..") {
            let position = encoded_position_for_decoded_byte(encoded, decoded_offset);
            let spelled = encoded[position..].split('/').next().unwrap_or_default();
            if spelled != segment {
                return Some(position);
            }
        }
        decoded_offset += segment.len() + 1;
    }
    None
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

/// Decode the percent escapes in one component, rejecting bytes a policy bars.
///
/// `None` means the component carries no escape at all, which is what lets
/// every caller answer a decode request without allocating for the common case.
fn decode_percent_bytes(
    value: &str,
    target: &'static str,
    rejected: impl Fn(u8) -> Option<&'static str>,
) -> Result<Option<Vec<u8>>> {
    let bytes = value.as_bytes();
    if !bytes.contains(&b'%') {
        return Ok(None);
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let escape = bytes
            .get(index + 1)
            .zip(bytes.get(index + 2))
            .and_then(|(high, low)| Some((hex_value(*high)?, hex_value(*low)?)));
        let Some((high, low)) = escape else {
            return Err(parse_error(
                target,
                index,
                "percent escape must contain exactly two hexadecimal digits",
            ));
        };
        let byte = (high << 4) | low;
        if let Some(reason) = rejected(byte) {
            return Err(parse_error(target, index, reason));
        }
        decoded.push(byte);
        index += 3;
    }
    Ok(Some(decoded))
}

/// Decode one component's percent escapes into the text they stand for.
///
/// The escapes are decoded, not re-interpreted: a component keeps its own
/// syntax, so `%2F` in a path segment becomes a literal `/` in the returned
/// text rather than a new segment boundary, and the borrowed form is returned
/// untouched when there is nothing to decode.
pub(crate) fn percent_decode<'a>(value: &'a str, target: &'static str) -> Result<Cow<'a, str>> {
    let Some(decoded) = decode_percent_bytes(value, target, |_| None)? else {
        return Ok(Cow::Borrowed(value));
    };
    String::from_utf8(decoded).map(Cow::Owned).map_err(|error| {
        parse_error(
            target,
            encoded_position_for_decoded_byte(value, error.utf8_error().valid_up_to()),
            "percent escapes must decode to UTF-8",
        )
    })
}

/// Answer one component as raw text or as the text its escapes stand for.
pub(super) fn decoded_component<'a>(
    value: &'a str,
    decode: bool,
    target: &'static str,
) -> Result<Cow<'a, str>> {
    if decode {
        percent_decode(value, target)
    } else {
        Ok(Cow::Borrowed(value))
    }
}

/// Percent-encode every byte the component's syntax cannot carry literally.
///
/// Text that needs no escape is borrowed. `%` is always encoded, so encoding
/// decoded text can never produce an escape the caller did not mean.
pub(super) fn percent_encode(value: &str, allowed: impl Fn(u8) -> bool) -> Cow<'_, str> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let bytes = value.as_bytes();
    // Proving the text needs no escape reads every byte anyway, so counting
    // while proving it buys the exact capacity for the case that does.
    let escapes = bytes
        .iter()
        .filter(|byte| **byte == b'%' || !allowed(**byte))
        .count();
    if escapes == 0 {
        return Cow::Borrowed(value);
    }
    let mut encoded = String::with_capacity(bytes.len() + 2 * escapes);
    for byte in bytes {
        if *byte != b'%' && allowed(*byte) {
            encoded.push(*byte as char);
            continue;
        }
        encoded.push('%');
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0F)] as char);
    }
    Cow::Owned(encoded)
}

pub(super) fn decode_file_component<'a>(
    value: &'a str,
    target: &'static str,
) -> Result<Cow<'a, str>> {
    let Some(decoded) = decode_percent_bytes(value, target, |byte| match byte {
        b'/' | b'\\' => Some("encoded path separators cannot be converted safely"),
        0 => Some("file path must not contain NUL"),
        _ => None,
    })?
    else {
        return Ok(Cow::Borrowed(value));
    };
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

    if !authority.is_empty() {
        // What follows a UNC server is a share name: there is no drive at
        // `\\server\c:`, and RFC 8089 s2 keeps a file path's case as given.
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
        // `file:/data` and `file:///data` name the same local path, so an
        // absolute one always carries the authority marker. Without the one
        // spelling, the URI a platform path converts to is not the URI that
        // parses back, and two values of one file compare and hash apart.
        *has_authority |= value.starts_with('/');
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
