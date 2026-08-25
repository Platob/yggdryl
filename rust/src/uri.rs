use std::borrow::Cow;
use std::fmt;
use std::iter::FusedIterator;
use std::net::Ipv6Addr;
use std::ops::Div;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, SmolStrBuilder};

use crate::generic::{MediaType, MimeType, Scheme};
use crate::{Error, Result, stable_hash_display};

pub(crate) mod pattern;

fn parse_error(target: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position,
        reason: SmolStr::new_static(reason),
    }
}

const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

const fn is_sub_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

const fn is_authority_byte(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delimiter(byte) || matches!(byte, b':' | b'@' | b'[' | b']')
}

const fn is_path_byte(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delimiter(byte) || matches!(byte, b'/' | b':' | b'@')
}

const fn is_query_fragment_byte(byte: u8) -> bool {
    is_path_byte(byte) || byte == b'?'
}

fn validate_component(
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

fn normalize_percent_hex(value: &str) -> SmolStr {
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

fn validate_optional_component(
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

fn normalize_resource_segment(
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

fn normalize_extension(value: &str) -> Result<SmolStr> {
    if value.is_empty() {
        return Err(parse_error(
            "uri extension",
            0,
            "filename extension must not be empty",
        ));
    }
    if let Some(position) = value.find('.') {
        return Err(parse_error(
            "uri extension",
            position,
            "filename extension must not contain a dot",
        ));
    }
    normalize_resource_segment(
        value,
        "uri extension",
        "filename extension must not be empty",
    )
}

fn preferred_mime_extension(value: &MimeType) -> Result<&'static str> {
    value.extension().ok_or_else(|| {
        parse_error(
            "MIME type",
            0,
            "MIME type has no preferred filename extension",
        )
    })
}

/// A validated URI authority.
///
/// The empty value is a concrete authority component used when a URI has no
/// authority; it is never represented as an optional or nullable value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Authority(SmolStr);

impl Authority {
    /// Parse and validate an authority component without surrounding `//`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the normalized authority without allocating.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Return whether this concrete authority is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the user name before the first user-information colon.
    pub fn user(&self) -> Option<&str> {
        let user_information = self.as_str().rsplit_once('@')?.0;
        Some(
            user_information
                .split_once(':')
                .map_or(user_information, |(user, _)| user),
        )
    }

    /// Return the password after the first user-information colon.
    ///
    /// Later colons belong to the password, so `user:pass:word@host` returns
    /// `pass:word` without allocating.
    pub fn password(&self) -> Option<&str> {
        self.as_str()
            .rsplit_once('@')?
            .0
            .split_once(':')
            .map(|(_, password)| password)
    }

    /// Return the host without user information, brackets, or a port.
    pub fn host(&self) -> &str {
        let host_port = self
            .as_str()
            .rsplit_once('@')
            .map_or(self.as_str(), |(_, host_port)| host_port);
        if let Some(bracketed) = host_port.strip_prefix('[') {
            return bracketed
                .split_once(']')
                .map_or(bracketed, |(host, _)| host);
        }
        host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host)
    }

    /// Return a deterministic cross-language hash of the authority.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

impl FromStr for Authority {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        validate_authority(value)?;
        Ok(Self(normalize_percent_hex(value)))
    }
}

fn validate_authority(value: &str) -> Result<()> {
    validate_component(value, "authority", 0, is_authority_byte)?;
    if value.bytes().filter(|byte| *byte == b'@').count() > 1 {
        return Err(parse_error(
            "authority",
            value.find('@').unwrap_or(0),
            "authority may contain at most one user-information delimiter",
        ));
    }

    let (user_information, host_port) = value
        .rsplit_once('@')
        .map_or((None, value), |(user_information, host_port)| {
            (Some(user_information), host_port)
        });
    if let Some(position) = user_information.and_then(|part| part.find(['[', ']'])) {
        return Err(parse_error(
            "authority",
            position,
            "brackets are permitted only around an IP literal host",
        ));
    }
    if !value.is_empty() && host_port.is_empty() {
        return Err(parse_error(
            "authority",
            value.len(),
            "non-empty authority must contain a host",
        ));
    }
    if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else {
            return Err(parse_error(
                "authority",
                value.len(),
                "bracketed host is missing its closing bracket",
            ));
        };
        if close == 0 {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len() + 1,
                "bracketed host must not be empty",
            ));
        }
        let literal = &bracketed[..close];
        validate_ip_literal(literal, value.len() - host_port.len() + 1)?;
        let suffix = &bracketed[close + 1..];
        if !suffix.is_empty()
            && (!suffix.starts_with(':') || !suffix[1..].bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(parse_error(
                "authority",
                value.len() - suffix.len(),
                "characters after a bracketed host must form a numeric port",
            ));
        }
    } else {
        if let Some(position) = host_port.find(['[', ']']) {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len() + position,
                "brackets are permitted only around an IP literal host",
            ));
        }
        let colon_count = host_port.bytes().filter(|byte| *byte == b':').count();
        if colon_count > 1 {
            return Err(parse_error(
                "authority",
                value.len() - host_port.len(),
                "IPv6 hosts must be enclosed in brackets",
            ));
        }
        if let Some((host, port)) = host_port.rsplit_once(':') {
            if host.is_empty() {
                return Err(parse_error(
                    "authority",
                    value.len() - host_port.len(),
                    "authority host must not be empty",
                ));
            }
            if !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(parse_error(
                    "authority",
                    value.len() - port.len(),
                    "authority port must contain only decimal digits",
                ));
            }
        }
    }

    Ok(())
}

fn validate_ip_literal(value: &str, base: usize) -> Result<()> {
    if let Some(future) = value.strip_prefix('v').or_else(|| value.strip_prefix('V')) {
        let Some((version, address)) = future.split_once('.') else {
            return Err(parse_error(
                "authority",
                base,
                "IPvFuture literal requires a hexadecimal version and address",
            ));
        };
        if version.is_empty()
            || !version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || address.is_empty()
            || !address
                .bytes()
                .all(|byte| is_unreserved(byte) || is_sub_delimiter(byte) || byte == b':')
        {
            return Err(parse_error(
                "authority",
                base,
                "IPvFuture literal contains an invalid version or address",
            ));
        }
        return Ok(());
    }

    let (address, zone) = value
        .split_once("%25")
        .map_or((value, None), |(address, zone)| (address, Some(zone)));
    if address.parse::<Ipv6Addr>().is_err() {
        return Err(parse_error(
            "authority",
            base,
            "bracketed host must contain a valid IPv6 or IPvFuture literal",
        ));
    }
    if let Some(zone) = zone {
        if zone.is_empty() {
            return Err(parse_error(
                "authority",
                base + address.len() + 3,
                "IPv6 zone identifier must not be empty",
            ));
        }
        validate_component(zone, "authority", base + address.len() + 3, is_unreserved)?;
    }
    Ok(())
}

impl AsRef<str> for Authority {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Authority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Authority {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Authority {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

/// A validated slash-separated URI path.
///
/// The path is always a concrete value. Empty paths are represented by the
/// empty string rather than an optional value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UriPath(SmolStr);

/// A cloning, allocation-free iterator over non-empty URI path segments.
#[derive(Clone, Debug)]
pub struct PathSegments<'a> {
    inner: std::str::Split<'a, char>,
}

impl<'a> Iterator for PathSegments<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.find(|segment| !segment.is_empty())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.inner.size_hint().1)
    }
}

impl DoubleEndedIterator for PathSegments<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.rfind(|segment| !segment.is_empty())
    }
}

impl FusedIterator for PathSegments<'_> {}

/// A cloning, allocation-free iterator over compound filename extensions.
#[derive(Clone, Debug)]
pub struct Extensions<'a> {
    inner: Option<std::str::Split<'a, char>>,
    prefixes_to_skip: u8,
}

impl<'a> Iterator for Extensions<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let inner = self.inner.as_mut()?;
        while self.prefixes_to_skip > 0 {
            inner.next();
            self.prefixes_to_skip -= 1;
        }
        inner.find(|extension| !extension.is_empty())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.inner.as_ref().and_then(|inner| inner.size_hint().1))
    }
}

impl FusedIterator for Extensions<'_> {}

fn file_name_from_path(value: &str) -> Option<&str> {
    if value.is_empty() || value.ends_with('/') {
        return None;
    }
    value.rsplit('/').next().filter(|name| !name.is_empty())
}

fn extension_from_file_name(file_name: &str) -> Option<&str> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(extension)
}

fn stem_from_file_name(file_name: &str) -> &str {
    extension_from_file_name(file_name).map_or(file_name, |extension| {
        &file_name[..file_name.len() - extension.len() - 1]
    })
}

fn extensions_from_file_name(file_name: Option<&str>) -> Extensions<'_> {
    let (inner, prefixes_to_skip) = file_name.map_or((None, 0), |name| {
        let hidden = name.starts_with('.');
        let search = if hidden { &name[1..] } else { name };
        let has_extension = search
            .find('.')
            .is_some_and(|position| position + 1 < search.len());
        if has_extension {
            (Some(name.split('.')), if hidden { 2 } else { 1 })
        } else {
            (None, 0)
        }
    });
    Extensions {
        inner,
        prefixes_to_skip,
    }
}

impl UriPath {
    /// Parse and validate a canonical URI path.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical path without allocating.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Return whether the concrete path is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over non-empty path segments without allocating.
    pub fn segments(&self) -> PathSegments<'_> {
        PathSegments {
            inner: self.as_str().split('/'),
        }
    }

    /// Return the number of non-empty path segments without allocating.
    pub fn segment_len(&self) -> usize {
        self.segments().count()
    }

    /// Return a path segment by zero-based index without allocating.
    pub fn get_segment(&self, index: usize) -> Option<&str> {
        self.segments().nth(index)
    }

    /// Return the next segment and opaque byte cursor for linear external iteration.
    ///
    /// Start with cursor `0` and pass the returned cursor to the next call.
    /// This is useful for FFI iterators that own the path and therefore cannot
    /// retain Rust's borrowing [`PathSegments`] iterator between calls.
    pub fn next_segment(&self, cursor: usize) -> Option<(usize, &str)> {
        let value = self.as_str();
        if cursor > value.len() {
            return None;
        }
        let bytes = value.as_bytes();
        let mut start = cursor;
        while start < bytes.len() && bytes[start] == b'/' {
            start += 1;
        }
        if start == bytes.len() {
            return None;
        }
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'/' {
            end += 1;
        }
        Some((end, &value[start..end]))
    }

    /// Return whether a path contains an exact segment.
    pub fn contains_segment(&self, value: &str) -> bool {
        self.segments().any(|segment| segment == value)
    }

    /// Return the final filename segment, or `None` for an empty/trailing path.
    pub fn file_name(&self) -> Option<&str> {
        file_name_from_path(self.as_str())
    }

    /// Return the filename without its final non-empty extension.
    ///
    /// A filename without an extension, including a single-dot hidden file,
    /// is its own stem.
    pub fn stem(&self) -> Option<&str> {
        self.file_name().map(stem_from_file_name)
    }

    /// Return the final non-empty filename suffix without its leading dot.
    pub fn extension(&self) -> Option<&str> {
        self.file_name().and_then(extension_from_file_name)
    }

    /// Iterate over every non-empty compound filename suffix from left to right.
    pub fn extensions(&self) -> Extensions<'_> {
        extensions_from_file_name(self.file_name())
    }

    /// Infer the final suffix MIME type, defaulting to octet-stream.
    pub fn mime_type(&self) -> MimeType {
        self.extension()
            .and_then(|extension| MimeType::from_extension(extension).ok())
            .unwrap_or_default()
    }

    /// Infer the base MIME type and ordered transparent encodings.
    pub fn media_type(&self) -> MediaType {
        MediaType::from_file_name(self.file_name().unwrap_or(""))
    }

    /// Replace the final filename segment, or append one to a directory path.
    ///
    /// The value must be one non-empty canonical URI path segment. Validation
    /// completes before the path changes.
    pub fn set_file_name(&mut self, value: &str) -> Result<()> {
        let normalized =
            normalize_resource_segment(value, "uri file name", "file name must not be empty")?;
        self.replace_file_name(normalized.as_str());
        Ok(())
    }

    /// Replace the filename stem while preserving its final extension.
    ///
    /// An existing filename is required, and validation completes before the
    /// path changes.
    pub fn set_stem(&mut self, value: &str) -> Result<()> {
        let normalized =
            normalize_resource_segment(value, "uri file stem", "file stem must not be empty")?;
        let Some(file_name) = self.file_name() else {
            return Err(parse_error(
                "uri file stem",
                0,
                "file stem requires an existing file name",
            ));
        };
        let extension = self.extension();
        let mut replacement = SmolStrBuilder::new();
        replacement.push_str(normalized.as_str());
        if let Some(extension) = extension {
            replacement.push('.');
            replacement.push_str(extension);
        }
        let replacement: SmolStr = replacement.into();
        debug_assert!(!file_name.is_empty());
        self.replace_file_name(replacement.as_str());
        Ok(())
    }

    /// Add or replace the final filename extension.
    ///
    /// The extension is supplied without a leading dot. An existing filename
    /// is required, and validation completes before the path changes.
    pub fn set_extension(&mut self, value: &str) -> Result<()> {
        let normalized = normalize_extension(value)?;
        let Some(stem) = self.stem() else {
            return Err(parse_error(
                "uri extension",
                0,
                "filename extension requires an existing file name",
            ));
        };
        let mut replacement = SmolStrBuilder::new();
        replacement.push_str(stem);
        replacement.push('.');
        replacement.push_str(normalized.as_str());
        let replacement: SmolStr = replacement.into();
        self.replace_file_name(replacement.as_str());
        Ok(())
    }

    /// Replace the complete compound extension chain.
    ///
    /// Extensions are supplied without leading dots. An empty iterator clears
    /// every extension. All inputs are validated before the path changes.
    pub fn set_extensions<I, S>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let normalized = values
            .into_iter()
            .map(|value| normalize_extension(value.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        let Some(file_name) = self.file_name() else {
            return Err(parse_error(
                "uri extensions",
                0,
                "filename extensions require an existing file name",
            ));
        };
        let base_end = compound_extension_start(file_name).unwrap_or(file_name.len());
        let mut replacement = SmolStrBuilder::new();
        replacement.push_str(&file_name[..base_end]);
        for extension in normalized {
            replacement.push('.');
            replacement.push_str(extension.as_str());
        }
        let replacement: SmolStr = replacement.into();
        self.replace_file_name(replacement.as_str());
        Ok(())
    }

    /// Add or replace the final suffix using a MIME preferred extension.
    ///
    /// A custom MIME type without a preferred extension returns an error and
    /// leaves the path unchanged.
    pub fn set_mime_type(&mut self, value: MimeType) -> Result<()> {
        self.set_extension(preferred_mime_extension(&value)?)
    }

    /// Replace the full suffix chain using a media type's preferred extensions.
    ///
    /// The base extension is emitted first, followed by encodings in their
    /// application order. A base without a preferred extension returns an
    /// error and leaves the path unchanged.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        preferred_mime_extension(value.base())?;
        self.set_extensions(value.extensions())
    }

    /// Remove the final filename extension and report whether one existed.
    pub fn remove_extension(&mut self) -> bool {
        let Some(stem) = self.stem() else {
            return false;
        };
        if self.extension().is_none() {
            return false;
        }
        let stem = SmolStr::new(stem);
        self.replace_file_name(stem.as_str());
        true
    }

    /// Remove every filename extension and report whether any existed.
    pub fn clear_extensions(&mut self) -> bool {
        let Some(file_name) = self.file_name() else {
            return false;
        };
        let Some(base_end) = compound_extension_start(file_name) else {
            return false;
        };
        let base = SmolStr::new(&file_name[..base_end]);
        self.replace_file_name(base.as_str());
        true
    }

    fn replace_file_name(&mut self, value: &str) {
        let prefix_end = self
            .file_name()
            .map_or(self.as_str().len(), |name| self.as_str().len() - name.len());
        let mut path = SmolStrBuilder::new();
        path.push_str(&self.as_str()[..prefix_end]);
        path.push_str(value);
        self.0 = path.into();
    }

    /// Return whether the path begins at the root.
    pub fn is_absolute(&self) -> bool {
        self.as_str().starts_with('/')
    }

    /// Return the resolved, non-empty segments of the path.
    ///
    /// Unlike [`Self::segments`], this resolves `.` and `..` so the result is
    /// the sequence of names the path actually addresses. A `..` that would
    /// escape an absolute root is dropped, matching filesystem semantics; on a
    /// relative path it is retained because there is no root to clamp against.
    pub fn parts(&self) -> Vec<&str> {
        let absolute = self.is_absolute();
        let mut parts: Vec<&str> = Vec::new();
        for segment in self.segments() {
            match segment {
                "." => {}
                ".." => {
                    if matches!(parts.last(), Some(&last) if last != "..") {
                        parts.pop();
                    } else if !absolute {
                        parts.push(segment);
                    }
                }
                other => parts.push(other),
            }
        }
        parts
    }

    /// Return this path with `value` appended and `.`/`..` resolved.
    ///
    /// An absolute `value` replaces the path outright. Otherwise the segments
    /// of `value` extend this path, so `joinpath` composes the way a shell
    /// `cd` does.
    ///
    /// # Errors
    ///
    /// Returns an error when the joined result is not a valid URI path.
    pub fn joinpath(&self, value: &str) -> Result<Self> {
        if value.starts_with('/') {
            return Self::from_str(&normalize_path_text(value, true));
        }
        let mut combined = String::with_capacity(self.as_str().len() + value.len() + 1);
        combined.push_str(self.as_str());
        if !combined.is_empty() && !combined.ends_with('/') && !value.is_empty() {
            combined.push('/');
        }
        combined.push_str(value);
        Self::from_str(&normalize_path_text(&combined, self.is_absolute()))
    }

    /// Return this path with `.` and `..` resolved.
    ///
    /// # Errors
    ///
    /// Returns an error when the normalized result is not a valid URI path.
    pub fn normalize(&self) -> Result<Self> {
        Self::from_str(&normalize_path_text(self.as_str(), self.is_absolute()))
    }

    /// Return the containing path, or `None` at the root.
    ///
    /// The result is normalized, so the parent of `/a/b/../c` is `/a`.
    pub fn parent(&self) -> Option<Self> {
        let mut parts = self.parts();
        parts.pop()?;
        Some(Self(SmolStr::new(join_parts(&parts, self.is_absolute()))))
    }

    /// Iterate from the immediate parent up to the root.
    ///
    /// The path itself is never yielded. An absolute path ends with the root
    /// `/`; a relative path ends with its outermost remaining segment.
    pub fn parents(&self) -> Parents {
        Parents {
            parts: self.parts().into_iter().map(SmolStr::new).collect(),
            absolute: self.is_absolute(),
        }
    }

    /// Return a deterministic cross-language hash of the canonical path.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

/// An iterator from a path's immediate parent up to its root.
///
/// Created by [`UriPath::parents`], which yields the same [`UriPath`] type as
/// its source value. [`Uri::parents`] and [`Url::parents`] have their own
/// iterators, because each preserves the components its source value carries.
#[derive(Clone, Debug)]
pub struct Parents {
    parts: Vec<SmolStr>,
    absolute: bool,
}

impl Iterator for Parents {
    type Item = UriPath;

    fn next(&mut self) -> Option<Self::Item> {
        self.parts.pop()?;
        let borrowed: Vec<&str> = self.parts.iter().map(SmolStr::as_str).collect();
        Some(UriPath(SmolStr::new(join_parts(&borrowed, self.absolute))))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.parts.len(), Some(self.parts.len()))
    }
}

impl Div<&str> for &UriPath {
    type Output = Result<UriPath>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl Div<&str> for UriPath {
    type Output = Result<Self>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl ExactSizeIterator for Parents {}

impl FusedIterator for Parents {}

/// Render resolved parts back into canonical path text.
fn join_parts(parts: &[&str], absolute: bool) -> String {
    let mut rendered = String::new();
    if absolute {
        rendered.push('/');
    }
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            rendered.push('/');
        }
        rendered.push_str(part);
    }
    rendered
}

/// Resolve `.` and `..` in raw path text.
fn normalize_path_text(value: &str, absolute: bool) -> String {
    let trailing_slash = value.len() > 1 && value.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        match segment {
            "." => {}
            ".." => {
                if matches!(parts.last(), Some(&last) if last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push(segment);
                }
            }
            other => parts.push(other),
        }
    }
    let mut rendered = join_parts(&parts, absolute);
    if trailing_slash && !rendered.ends_with('/') {
        rendered.push('/');
    }
    rendered
}

fn compound_extension_start(file_name: &str) -> Option<usize> {
    let search_start = usize::from(file_name.starts_with('.'));
    let search = &file_name[search_start..];
    search
        .find('.')
        .filter(|position| position + 1 < search.len())
        .map(|position| search_start + position)
}

impl FromStr for UriPath {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        validate_component(value, "uri path", 0, is_path_byte)?;
        Ok(Self(normalize_percent_hex(value)))
    }
}

impl AsRef<str> for UriPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'a> IntoIterator for &'a UriPath {
    type Item = &'a str;
    type IntoIter = PathSegments<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments()
    }
}

impl fmt::Display for UriPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for UriPath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UriPath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

/// An owned, canonical absolute URI with concrete scheme, authority, and path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Uri {
    scheme: Scheme,
    authority: Authority,
    path: UriPath,
    has_authority: bool,
    query: Option<SmolStr>,
    fragment: Option<SmolStr>,
}

impl Uri {
    /// Build a validated URI from owned components.
    ///
    /// Authority syntax is inferred for non-empty authorities and for absolute
    /// `file` paths. Use parsing when an explicitly empty authority marker must
    /// be preserved for another scheme.
    pub fn from_parts(
        scheme: Scheme,
        authority: Authority,
        path: UriPath,
        query: Option<SmolStr>,
        fragment: Option<SmolStr>,
    ) -> Result<Self> {
        let has_authority =
            !authority.is_empty() || (scheme == Scheme::FILE && path.as_str().starts_with('/'));
        Self::from_parts_with_authority(scheme, authority, path, has_authority, query, fragment)
    }

    fn from_parts_with_authority(
        scheme: Scheme,
        mut authority: Authority,
        mut path: UriPath,
        mut has_authority: bool,
        query: Option<SmolStr>,
        fragment: Option<SmolStr>,
    ) -> Result<Self> {
        if scheme == Scheme::FILE {
            canonicalize_file_drive(&mut authority, &mut path, &mut has_authority);
        }
        if !has_authority && !authority.is_empty() {
            return Err(parse_error(
                "uri",
                0,
                "a non-empty authority requires an authority marker",
            ));
        }
        if has_authority && !path.is_empty() && !path.as_str().starts_with('/') {
            return Err(parse_error(
                "uri",
                0,
                "a path following an authority must be empty or start with slash",
            ));
        }
        if !has_authority && path.as_str().starts_with("//") {
            return Err(parse_error(
                "uri",
                0,
                "a path without authority must not start with two slashes",
            ));
        }
        let query = validate_optional_component(query, "uri query", is_query_fragment_byte)?;
        let fragment =
            validate_optional_component(fragment, "uri fragment", is_query_fragment_byte)?;
        Ok(Self {
            scheme,
            authority,
            path,
            has_authority,
            query,
            fragment,
        })
    }

    /// Parse an absolute URI or an unmistakable Windows path.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Convert a platform path string to a canonical `file:` URI.
    ///
    /// Windows drive and UNC detection is textual and behaves identically on
    /// every host operating system.
    pub fn from_path(value: impl AsRef<Path>) -> Result<Self> {
        let value = value
            .as_ref()
            .to_str()
            .ok_or_else(|| parse_error("path", 0, "file path must be valid UTF-8"))?;
        if value.is_empty() {
            return Err(parse_error("path", 0, "file path must not be empty"));
        }
        if let Some((position, _)) = value
            .char_indices()
            .find(|(_, character)| character.is_control() && *character != '\t')
        {
            return Err(parse_error(
                "path",
                position,
                "file path must not contain control characters",
            ));
        }

        let scheme = Scheme::FILE;
        let is_unc =
            value.starts_with("\\\\") || (value.starts_with("//") && !value.starts_with("///"));
        if is_unc {
            let trimmed = value.trim_start_matches(['\\', '/']);
            let leading_separator_len = value.len() - trimmed.len();
            let separator = trimmed.find(['\\', '/']);
            let (server, remainder) = separator.map_or((trimmed, ""), |position| {
                (&trimmed[..position], &trimmed[position + 1..])
            });
            if server.is_empty() {
                return Err(parse_error(
                    "path",
                    2,
                    "UNC path must contain a server name",
                ));
            }
            let authority = authority_from_file_server(server, leading_separator_len)?;
            let encoded = encode_file_path(remainder, true, false);
            return Self::from_parts_with_authority(
                scheme,
                authority,
                UriPath(encoded),
                true,
                None,
                None,
            );
        }

        let drive_absolute = is_windows_drive_absolute(value);
        let local_absolute = value.starts_with('/');
        let (path_input, prefix_slash) = if local_absolute {
            (value.trim_start_matches('/'), true)
        } else {
            (value, drive_absolute)
        };
        let path = UriPath(encode_file_path(path_input, prefix_slash, drive_absolute));
        let has_authority = drive_absolute || local_absolute;
        Self::from_parts_with_authority(
            scheme,
            Authority(SmolStr::new("")),
            path,
            has_authority,
            None,
            None,
        )
    }

    /// Deserialize a URI from its structural JSON representation.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Consume this URI and serialize it as structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Validate and consume this URI as a URL without cloning.
    pub fn into_url(self) -> Result<Url> {
        Url::from_uri(self)
    }

    /// Validate and consume this URI as a URN without cloning.
    pub fn into_urn(self) -> Result<Urn> {
        Urn::from_uri(self)
    }

    /// Consume a canonical `file:` URI and return its platform path.
    pub fn into_path(self) -> Result<PathBuf> {
        file_path_from_uri(&self)
    }

    /// Validate all cross-component URI invariants.
    pub fn validate(&self) -> Result<()> {
        if !self.has_authority && !self.authority.is_empty() {
            return Err(parse_error(
                "uri",
                0,
                "a non-empty authority requires an authority marker",
            ));
        }
        if self.has_authority && !self.path.is_empty() && !self.path.as_str().starts_with('/') {
            return Err(parse_error(
                "uri",
                0,
                "a path following an authority must be empty or start with slash",
            ));
        }
        if !self.has_authority && self.path.as_str().starts_with("//") {
            return Err(parse_error(
                "uri",
                0,
                "a path without authority must not start with two slashes",
            ));
        }
        if let Some(query) = self.query() {
            validate_component(query, "uri query", 0, is_query_fragment_byte)?;
        }
        if let Some(fragment) = self.fragment() {
            validate_component(fragment, "uri fragment", 0, is_query_fragment_byte)?;
        }
        Ok(())
    }

    /// Return the required scheme component.
    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Return the concrete authority component.
    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    /// Return the authority user name, if present.
    pub fn user(&self) -> Option<&str> {
        self.authority.user()
    }

    /// Return the authority password, preserving any later colons.
    pub fn password(&self) -> Option<&str> {
        self.authority.password()
    }

    /// Return the network hostname, if this URI has one.
    ///
    /// For `s3`, an authority or first path part ending in `.com` or `.io` is
    /// a hostname; any other first part is a bucket name.
    pub fn hostname(&self) -> Option<&str> {
        if self.scheme == Scheme::S3 {
            return self.s3_location().and_then(|location| location.hostname);
        }
        (!self.authority.is_empty()).then(|| self.authority.host())
    }

    /// Return the S3 bucket name when this is an `s3` URI.
    pub fn bucket(&self) -> Option<&str> {
        self.s3_location().and_then(|location| location.bucket)
    }

    /// Infer an AWS region from a recognized S3 hostname.
    ///
    /// This borrows the region from the URI and performs no network lookup.
    pub fn region(&self) -> Option<&str> {
        self.s3_location().and_then(|location| location.region)
    }

    /// Return whether canonical syntax contains an authority marker.
    pub fn has_authority(&self) -> bool {
        self.has_authority
    }

    /// Return the concrete path component.
    pub fn path(&self) -> &UriPath {
        &self.path
    }

    /// Return query text without `?`, if it was present.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// Return fragment text without `#`, if it was present.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// Iterate over non-empty path segments without allocating.
    pub fn path_segments(&self) -> PathSegments<'_> {
        self.path.segments()
    }

    /// Return the next path segment and cursor for an owning FFI iterator.
    pub fn next_path_segment(&self, cursor: usize) -> Option<(usize, &str)> {
        self.path.next_segment(cursor)
    }

    /// Return the last filename segment, if present.
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name()
    }

    /// Return the filename without its final non-empty extension.
    pub fn stem(&self) -> Option<&str> {
        self.path.stem()
    }

    /// Return the final filename extension, if present.
    pub fn extension(&self) -> Option<&str> {
        self.path.extension()
    }

    /// Iterate over compound filename extensions without allocating.
    pub fn extensions(&self) -> Extensions<'_> {
        self.path.extensions()
    }

    /// Infer the final suffix MIME type, defaulting to octet-stream.
    pub fn mime_type(&self) -> MimeType {
        self.path.mime_type()
    }

    /// Infer the base MIME type and ordered transparent encodings.
    pub fn media_type(&self) -> MediaType {
        self.path.media_type()
    }

    /// Replace the final filename segment, or append one to a directory path.
    ///
    /// Authority, query, and fragment syntax is retained. An error leaves the
    /// URI unchanged.
    pub fn set_file_name(&mut self, value: &str) -> Result<()> {
        let mut path = self.path.clone();
        path.set_file_name(value)?;
        if self.has_authority && !path.as_str().starts_with('/') {
            let mut absolute = SmolStrBuilder::new();
            absolute.push('/');
            absolute.push_str(path.as_str());
            path = UriPath(absolute.into());
        }
        self.set_resource_path(path)
    }

    /// Replace the filename stem while preserving its final extension.
    ///
    /// An error leaves the URI unchanged.
    pub fn set_stem(&mut self, value: &str) -> Result<()> {
        let mut path = self.path.clone();
        path.set_stem(value)?;
        self.set_resource_path(path)
    }

    /// Add or replace the final filename extension.
    ///
    /// The extension is supplied without a leading dot. An error leaves the
    /// URI unchanged.
    pub fn set_extension(&mut self, value: &str) -> Result<()> {
        let mut path = self.path.clone();
        path.set_extension(value)?;
        self.set_resource_path(path)
    }

    /// Replace the complete compound extension chain.
    ///
    /// Extensions are supplied without leading dots. An empty iterator clears
    /// every extension. An error leaves the URI unchanged.
    pub fn set_extensions<I, S>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut path = self.path.clone();
        path.set_extensions(values)?;
        self.set_resource_path(path)
    }

    /// Add or replace the final suffix using a MIME preferred extension.
    ///
    /// An error leaves the URI unchanged.
    pub fn set_mime_type(&mut self, value: MimeType) -> Result<()> {
        let mut path = self.path.clone();
        path.set_mime_type(value)?;
        self.set_resource_path(path)
    }

    /// Replace the full suffix chain from a base MIME type and encodings.
    ///
    /// An error leaves the URI unchanged.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let mut path = self.path.clone();
        path.set_media_type(value)?;
        self.set_resource_path(path)
    }

    /// Remove the final filename extension and report whether one existed.
    pub fn remove_extension(&mut self) -> bool {
        let mut path = self.path.clone();
        if !path.remove_extension() {
            return false;
        }
        self.path = path;
        true
    }

    /// Remove every filename extension and report whether any existed.
    pub fn clear_extensions(&mut self) -> bool {
        let mut path = self.path.clone();
        if !path.clear_extensions() {
            return false;
        }
        self.path = path;
        true
    }

    fn set_resource_path(&mut self, path: UriPath) -> Result<()> {
        let mut candidate = self.clone();
        candidate.path = path;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Return the resolved, non-empty path segments with `.`/`..` applied.
    pub fn parts(&self) -> Vec<&str> {
        self.path.parts()
    }

    /// Return this URI with `value` appended to its path.
    ///
    /// Scheme, authority, query, and fragment are preserved. An absolute
    /// `value` replaces the path; otherwise it extends it, resolving `.` and
    /// `..`.
    ///
    /// # Errors
    ///
    /// Returns an error when the joined path is invalid for this URI.
    pub fn joinpath(&self, value: &str) -> Result<Self> {
        let path = self.path.joinpath(value)?;
        let mut candidate = self.clone();
        candidate.path = path;
        candidate.validate()?;
        Ok(candidate)
    }

    /// Return this URI addressing its containing path, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        let path = self.path.parent()?;
        let mut candidate = self.clone();
        candidate.path = path;
        candidate.validate().ok()?;
        Some(candidate)
    }

    /// Iterate from the immediate parent up to the root, yielding whole URIs.
    ///
    /// Scheme, authority, query, and fragment are preserved on every step.
    pub fn parents(&self) -> UriParents<'_> {
        UriParents {
            source: self,
            paths: self.path.parents(),
        }
    }

    /// Return the IANA-registered default port for this URI's scheme.
    ///
    /// This is the port a client uses when the authority omits one; it does
    /// not read a port written into the authority.
    pub fn default_port(&self) -> Option<u16> {
        self.scheme.default_port()
    }

    /// Return a deterministic cross-language hash of canonical display output.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    fn s3_location(&self) -> Option<S3Location<'_>> {
        if self.scheme != Scheme::S3 {
            return None;
        }

        let mut path = self.path_segments();
        let first = if self.has_authority && !self.authority.is_empty() {
            self.authority.host()
        } else {
            path.next()?
        };

        if let Some(aws) = parse_aws_s3_hostname(first) {
            return Some(S3Location {
                hostname: Some(first),
                bucket: aws.bucket.or_else(|| path.next()),
                region: aws.region,
            });
        }
        if is_s3_hostname(first) {
            return Some(S3Location {
                hostname: Some(first),
                bucket: path.next(),
                region: None,
            });
        }
        Some(S3Location {
            hostname: None,
            bucket: Some(first),
            region: None,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct S3Location<'a> {
    hostname: Option<&'a str>,
    bucket: Option<&'a str>,
    region: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
struct AwsS3Hostname<'a> {
    bucket: Option<&'a str>,
    region: Option<&'a str>,
}

fn is_s3_hostname(value: &str) -> bool {
    [".com", ".io"].iter().any(|suffix| {
        value
            .get(value.len().saturating_sub(suffix.len())..)
            .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
    })
}

fn strip_suffix_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let head = value.get(..value.len().checked_sub(suffix.len())?)?;
    value[head.len()..]
        .eq_ignore_ascii_case(suffix)
        .then_some(head)
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = value.get(prefix.len()..)?;
    value[..prefix.len()]
        .eq_ignore_ascii_case(prefix)
        .then_some(tail)
}

fn parse_aws_s3_hostname(hostname: &str) -> Option<AwsS3Hostname<'_>> {
    let body = strip_suffix_ascii_case(hostname, ".amazonaws.com.cn")
        .or_else(|| strip_suffix_ascii_case(hostname, ".amazonaws.com"))?;
    let mut start = 0;
    loop {
        let endpoint = &body[start..];
        if let Some(region) = parse_aws_s3_endpoint(endpoint) {
            return Some(AwsS3Hostname {
                bucket: (start != 0).then(|| &body[..start - 1]),
                region,
            });
        }
        let dot = body[start..].find('.')?;
        start += dot + 1;
    }
}

fn parse_aws_s3_endpoint(value: &str) -> Option<Option<&str>> {
    if value.eq_ignore_ascii_case("s3")
        || value.eq_ignore_ascii_case("s3-accelerate")
        || value.eq_ignore_ascii_case("s3-accelerate.dualstack")
    {
        return Some(None);
    }

    for prefix in ["s3.dualstack.", "s3-fips.dualstack.", "s3.", "s3-fips."] {
        if let Some(region) = strip_prefix_ascii_case(value, prefix) {
            return (!region.is_empty() && !region.contains('.')).then_some(Some(region));
        }
    }
    strip_prefix_ascii_case(value, "s3-")
        .and_then(|region| (!region.is_empty() && !region.contains('.')).then_some(Some(region)))
}

/// An iterator from a URI's immediate parent up to its root.
///
/// Created by [`Uri::parents`]. Each item preserves the source scheme,
/// authority, query, and fragment.
#[derive(Clone, Debug)]
pub struct UriParents<'a> {
    source: &'a Uri,
    paths: Parents,
}

impl Iterator for UriParents<'_> {
    type Item = Uri;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let path = self.paths.next()?;
            let mut candidate = self.source.clone();
            candidate.path = path;
            if candidate.validate().is_ok() {
                return Some(candidate);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.paths.size_hint().1)
    }
}

impl Div<&str> for &Uri {
    type Output = Result<Uri>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl Div<&str> for Uri {
    type Output = Result<Self>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl FusedIterator for UriParents<'_> {}

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

impl AsRef<UriPath> for Uri {
    fn as_ref(&self) -> &UriPath {
        self.path()
    }
}

fn offset_parse_error(
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

fn is_windows_drive_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn encode_file_path(value: &str, prefix_slash: bool, uppercase_drive: bool) -> SmolStr {
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

fn authority_from_file_server(value: &str, source_offset: usize) -> Result<Authority> {
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

fn remap_file_authority_error(
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

fn decoded_authority_position(source: &str, encoded_position: usize) -> usize {
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

fn file_path_from_uri(value: &Uri) -> Result<PathBuf> {
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

fn encoded_windows_drive_position(encoded: &str, decoded: &str) -> Option<usize> {
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

fn validate_file_authority_round_trip(value: &str) -> Result<()> {
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

fn decode_file_component<'a>(value: &'a str, target: &'static str) -> Result<Cow<'a, str>> {
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

fn encoded_position_for_decoded_byte(value: &str, decoded_position: usize) -> usize {
    let bytes = value.as_bytes();
    let mut encoded = 0;
    let mut decoded = 0;
    while encoded < bytes.len() && decoded < decoded_position {
        encoded += if bytes[encoded] == b'%' { 3 } else { 1 };
        decoded += 1;
    }
    encoded
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn canonicalize_file_drive(
    authority: &mut Authority,
    path: &mut UriPath,
    has_authority: &mut bool,
) {
    let authority_bytes = authority.as_str().as_bytes();
    if authority_bytes.len() == 2
        && authority_bytes[0].is_ascii_alphabetic()
        && authority_bytes[1] == b':'
        && path.as_str().starts_with('/')
    {
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

impl fmt::Display for Uri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:", self.scheme)?;
        if self.has_authority {
            write!(formatter, "//{}", self.authority)?;
        }
        formatter.write_str(self.path.as_str())?;
        if let Some(query) = &self.query {
            write!(formatter, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(formatter, "#{fragment}")?;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for &'a Uri {
    type Item = &'a str;
    type IntoIter = PathSegments<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.path_segments()
    }
}

impl Serialize for Uri {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Uri", 6)?;
        state.serialize_field("scheme", &self.scheme)?;
        state.serialize_field("authority", &self.authority)?;
        state.serialize_field("path", &self.path)?;
        state.serialize_field("has_authority", &self.has_authority)?;
        state.serialize_field("query", &self.query)?;
        state.serialize_field("fragment", &self.fragment)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Uri {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Representation {
            scheme: Scheme,
            authority: Authority,
            path: UriPath,
            has_authority: bool,
            query: Option<SmolStr>,
            fragment: Option<SmolStr>,
        }

        let value = Representation::deserialize(deserializer)?;
        Self::from_parts_with_authority(
            value.scheme,
            value.authority,
            value.path,
            value.has_authority,
            value.query,
            value.fragment,
        )
        .map_err(D::Error::custom)
    }
}

/// A validated URL backed directly by a canonical [`Uri`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Url(Uri);

impl Url {
    /// Parse and validate a URL.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Convert a file path to a canonical `file:` URL.
    pub fn from_path(value: impl AsRef<Path>) -> Result<Self> {
        Self::from_uri(Uri::from_path(value)?)
    }

    /// Validate and wrap an existing URI as a URL.
    pub fn from_uri(value: Uri) -> Result<Self> {
        if value.scheme() == &Scheme::URN {
            return Err(parse_error("url", 0, "URN values are not URLs"));
        }
        let file_url = value.scheme() == &Scheme::FILE;
        if !value.has_authority() || (!file_url && value.authority().is_empty()) {
            return Err(parse_error(
                "url",
                value.scheme().as_str().len() + 1,
                "URL requires hierarchical authority syntax and non-file URLs require a host",
            ));
        }
        Ok(Self(value))
    }

    /// Deserialize a URL from structural JSON.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Consume this URL and serialize it as structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Consume this URL and return its URI without allocating.
    pub fn into_uri(self) -> Uri {
        self.0
    }

    /// Consume this `file:` URL and return its platform path.
    pub fn into_path(self) -> Result<PathBuf> {
        self.0.into_path()
    }

    /// Return whether this URL locates something on the local file system.
    ///
    /// Only a local URL converts to a [`Path`]; everything else needs a client
    /// for its scheme.
    pub fn is_local(&self) -> bool {
        self.scheme() == &Scheme::FILE
    }

    /// Append `path` to this URL, one segment per component.
    ///
    /// This is [`Path::join`] for URLs: `.` and `..` resolve exactly as
    /// [`Self::joinpath`] resolves them, and an absolute `path` replaces this
    /// URL's path rather than extending it.
    ///
    /// # Errors
    ///
    /// Returns an error when a component is not valid UTF-8, when a component
    /// cannot be encoded as a path segment, or when `..` escapes the root.
    pub fn join_path(&self, path: impl AsRef<Path>) -> Result<Self> {
        use std::path::Component;

        let path = path.as_ref();
        let mut joined = self.clone();
        for component in path.components() {
            joined = match component {
                Component::Prefix(_) | Component::RootDir => Self::from_path(path)?,
                Component::CurDir => continue,
                Component::ParentDir => joined.joinpath("..")?,
                Component::Normal(segment) => {
                    let segment = segment
                        .to_str()
                        .ok_or_else(|| parse_error("path", 0, "file path must be valid UTF-8"))?;
                    joined.joinpath(segment)?
                }
            };
        }
        Ok(joined)
    }

    /// Return whether the last path segment is a private name.
    ///
    /// A private name begins with a dot - `.git`, `.venv`, `.DS_Store` - which
    /// every operating system and tool treats as hidden. A listing excludes
    /// these unless a caller asks for them, so walking a tree does not wander
    /// into version control or virtual environments by default.
    pub fn is_private(&self) -> bool {
        self.file_name().is_some_and(|name| name.starts_with('.'))
    }

    /// Return whether this URL locates something that exists right now.
    ///
    /// A non-local URL is never reported as existing, because answering would
    /// require a network round trip this accessor does not make.
    pub fn exists(&self) -> bool {
        self.clone().into_path().is_ok_and(|path| path.exists())
    }

    /// Return whether this URL locates an existing local directory.
    pub fn is_dir(&self) -> bool {
        self.clone().into_path().is_ok_and(|path| path.is_dir())
    }

    /// Return whether this URL locates an existing local regular file.
    pub fn is_file(&self) -> bool {
        self.clone().into_path().is_ok_and(|path| path.is_file())
    }

    /// Report the MIME type of the local entry this URL locates.
    ///
    /// An existing directory is [`MimeType::DIRECTORY`]; anything else is
    /// identified from the name, falling back to [`MimeType::FILE`] locally and
    /// to [`Self::mime_type`] for a remote URL.
    pub fn local_mime_type(&self) -> MimeType {
        match self.clone().into_path() {
            Ok(path) => MimeType::from_local_path(path),
            Err(_) => self.mime_type(),
        }
    }

    /// Return the required URL scheme.
    pub fn scheme(&self) -> &Scheme {
        self.0.scheme()
    }

    /// Return the concrete URL authority.
    pub fn authority(&self) -> &Authority {
        self.0.authority()
    }

    /// Return the authority user name, if present.
    pub fn user(&self) -> Option<&str> {
        self.0.user()
    }

    /// Return the authority password, preserving any later colons.
    pub fn password(&self) -> Option<&str> {
        self.0.password()
    }

    /// Return the network hostname under the URI's S3-aware rules.
    pub fn hostname(&self) -> Option<&str> {
        self.0.hostname()
    }

    /// Return the S3 bucket name when this is an `s3` URL.
    pub fn bucket(&self) -> Option<&str> {
        self.0.bucket()
    }

    /// Infer an AWS region from a recognized S3 hostname.
    pub fn region(&self) -> Option<&str> {
        self.0.region()
    }

    /// Return the concrete URL path.
    pub fn path(&self) -> &UriPath {
        self.0.path()
    }

    /// Return URL query text without `?`.
    pub fn query(&self) -> Option<&str> {
        self.0.query()
    }

    /// Return URL fragment text without `#`.
    pub fn fragment(&self) -> Option<&str> {
        self.0.fragment()
    }

    /// Iterate over non-empty URL path segments without allocating.
    pub fn path_segments(&self) -> PathSegments<'_> {
        self.0.path_segments()
    }

    /// Return the next path segment and cursor for an owning FFI iterator.
    pub fn next_path_segment(&self, cursor: usize) -> Option<(usize, &str)> {
        self.0.next_path_segment(cursor)
    }

    /// Return the last URL filename segment.
    pub fn file_name(&self) -> Option<&str> {
        self.0.file_name()
    }

    /// Return the URL filename without its final non-empty extension.
    pub fn stem(&self) -> Option<&str> {
        self.0.stem()
    }

    /// Return the final URL filename extension.
    pub fn extension(&self) -> Option<&str> {
        self.0.extension()
    }

    /// Iterate over compound URL filename extensions without allocating.
    pub fn extensions(&self) -> Extensions<'_> {
        self.0.extensions()
    }

    /// Infer the final URL suffix MIME type, defaulting to octet-stream.
    pub fn mime_type(&self) -> MimeType {
        self.0.mime_type()
    }

    /// Infer the URL base MIME type and ordered transparent encodings.
    pub fn media_type(&self) -> MediaType {
        self.0.media_type()
    }

    /// Replace the final URL filename segment, or append one to its path.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_file_name(&mut self, value: &str) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_file_name(value)?;
        self.replace_uri(candidate)
    }

    /// Replace the URL filename stem while preserving its final extension.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_stem(&mut self, value: &str) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_stem(value)?;
        self.replace_uri(candidate)
    }

    /// Add or replace the final URL filename extension.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_extension(&mut self, value: &str) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_extension(value)?;
        self.replace_uri(candidate)
    }

    /// Replace the complete compound URL filename extension chain.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_extensions<I, S>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut candidate = self.0.clone();
        candidate.set_extensions(values)?;
        self.replace_uri(candidate)
    }

    /// Add or replace the final URL suffix using a MIME preferred extension.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_mime_type(&mut self, value: MimeType) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_mime_type(value)?;
        self.replace_uri(candidate)
    }

    /// Replace the full URL suffix chain from a base MIME type and encodings.
    ///
    /// An error leaves the URL unchanged.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_media_type(value)?;
        self.replace_uri(candidate)
    }

    /// Remove the final URL filename extension and report whether one existed.
    pub fn remove_extension(&mut self) -> bool {
        self.0.remove_extension()
    }

    /// Remove every URL filename extension and report whether any existed.
    pub fn clear_extensions(&mut self) -> bool {
        self.0.clear_extensions()
    }

    fn replace_uri(&mut self, candidate: Uri) -> Result<()> {
        *self = Self::from_uri(candidate)?;
        Ok(())
    }

    /// Return the resolved, non-empty path segments with `.`/`..` applied.
    pub fn parts(&self) -> Vec<&str> {
        self.0.parts()
    }

    /// Return this URL with `value` appended to its path.
    ///
    /// # Errors
    ///
    /// Returns an error when the joined path is invalid for this URL.
    pub fn joinpath(&self, value: &str) -> Result<Self> {
        Self::from_uri(self.0.joinpath(value)?)
    }

    /// Return this URL addressing its containing path, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        Self::from_uri(self.0.parent()?).ok()
    }

    /// Iterate from the immediate parent up to the root, yielding whole URLs.
    pub fn parents(&self) -> UrlParents<'_> {
        UrlParents {
            inner: self.0.parents(),
        }
    }

    /// Return the IANA-registered default port for this URL's scheme.
    pub fn default_port(&self) -> Option<u16> {
        self.0.default_port()
    }

    /// Return a deterministic cross-language hash of the canonical URL.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

/// An iterator from a URL's immediate parent up to its root.
///
/// Created by [`Url::parents`].
#[derive(Clone, Debug)]
pub struct UrlParents<'a> {
    inner: UriParents<'a>,
}

impl Iterator for UrlParents<'_> {
    type Item = Url;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let candidate = self.inner.next()?;
            if let Ok(url) = Url::from_uri(candidate) {
                return Some(url);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.inner.size_hint().1)
    }
}

impl Div<&str> for &Url {
    type Output = Result<Url>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl Div<&str> for Url {
    type Output = Result<Self>;

    fn div(self, value: &str) -> Self::Output {
        self.joinpath(value)
    }
}

impl FusedIterator for UrlParents<'_> {}

impl FromStr for Url {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_uri(Uri::from_str(value)?)
    }
}

impl TryFrom<Uri> for Url {
    type Error = Error;

    fn try_from(value: Uri) -> Result<Self> {
        Self::from_uri(value)
    }
}

impl TryFrom<&Uri> for Url {
    type Error = Error;

    fn try_from(value: &Uri) -> Result<Self> {
        Self::from_uri(value.clone())
    }
}

impl TryFrom<&Path> for Url {
    type Error = Error;

    fn try_from(value: &Path) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<PathBuf> for Url {
    type Error = Error;

    fn try_from(value: PathBuf) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<&PathBuf> for Url {
    type Error = Error;

    fn try_from(value: &PathBuf) -> Result<Self> {
        Self::from_path(value)
    }
}

impl TryFrom<Url> for PathBuf {
    type Error = Error;

    fn try_from(value: Url) -> Result<Self> {
        value.into_path()
    }
}

impl TryFrom<&Url> for PathBuf {
    type Error = Error;

    fn try_from(value: &Url) -> Result<Self> {
        value.clone().into_path()
    }
}

impl From<Url> for Uri {
    fn from(value: Url) -> Self {
        value.into_uri()
    }
}

impl From<&Url> for Uri {
    fn from(value: &Url) -> Self {
        value.clone().into_uri()
    }
}

impl AsRef<Uri> for Url {
    fn as_ref(&self) -> &Uri {
        &self.0
    }
}

impl AsRef<UriPath> for Url {
    fn as_ref(&self) -> &UriPath {
        self.path()
    }
}

impl fmt::Display for Url {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'a> IntoIterator for &'a Url {
    type Item = &'a str;
    type IntoIter = PathSegments<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.path_segments()
    }
}

impl Serialize for Url {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Url {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_uri(Uri::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A validated RFC-style URN backed directly by a canonical [`Uri`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Urn(Uri);

impl Urn {
    /// Parse and validate a URN.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Validate and wrap an existing URI as a URN.
    pub fn from_uri(mut value: Uri) -> Result<Self> {
        if value.scheme() != &Scheme::URN {
            return Err(parse_error("urn", 0, "URN scheme must be `urn`"));
        }
        if value.has_authority() || !value.authority().is_empty() {
            return Err(parse_error("urn", 4, "URN must not contain an authority"));
        }
        let Some((namespace, namespace_specific)) = value.path().as_str().split_once(':') else {
            return Err(parse_error(
                "urn",
                4,
                "URN must contain a namespace and namespace-specific string",
            ));
        };
        validate_namespace(namespace, 4)?;
        if namespace_specific.is_empty() {
            return Err(parse_error(
                "urn",
                5 + namespace.len(),
                "URN namespace-specific string must not be empty",
            ));
        }
        validate_urn_query(value.query(), 5 + value.path().as_str().len())?;
        if namespace.bytes().any(|byte| byte.is_ascii_uppercase()) {
            let mut path = SmolStrBuilder::new();
            path.push_str(&namespace.to_ascii_lowercase());
            path.push(':');
            path.push_str(namespace_specific);
            value.path = UriPath(path.into());
        }
        Ok(Self(value))
    }

    /// Deserialize a URN from structural JSON.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Consume this URN and serialize it as structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Consume this URN and return its URI without allocating.
    pub fn into_uri(self) -> Uri {
        self.0
    }

    /// Return the required `urn` scheme.
    pub fn scheme(&self) -> &Scheme {
        self.0.scheme()
    }

    /// Return the concrete empty URN authority.
    pub fn authority(&self) -> &Authority {
        self.0.authority()
    }

    /// Return the concrete URN path (`namespace:namespace-specific`).
    pub fn path(&self) -> &UriPath {
        self.0.path()
    }

    /// Return optional URN query text without `?`.
    pub fn query(&self) -> Option<&str> {
        self.0.query()
    }

    /// Return optional URN fragment text without `#`.
    pub fn fragment(&self) -> Option<&str> {
        self.0.fragment()
    }

    /// Return the canonical lowercase URN namespace identifier.
    pub fn namespace(&self) -> &str {
        self.0
            .path()
            .as_str()
            .split_once(':')
            .map_or("", |(namespace, _)| namespace)
    }

    /// Return the URN namespace-specific string.
    pub fn namespace_specific(&self) -> &str {
        self.0
            .path()
            .as_str()
            .split_once(':')
            .map_or("", |(_, namespace_specific)| namespace_specific)
    }

    /// Iterate over non-empty URN path segments without allocating.
    pub fn path_segments(&self) -> PathSegments<'_> {
        self.0.path_segments()
    }

    /// Return the next path segment and cursor for an owning FFI iterator.
    pub fn next_path_segment(&self, cursor: usize) -> Option<(usize, &str)> {
        self.0.next_path_segment(cursor)
    }

    /// Return the last filename segment in the namespace-specific string.
    pub fn file_name(&self) -> Option<&str> {
        file_name_from_path(self.namespace_specific())
    }

    /// Return the namespace-specific filename without its final extension.
    pub fn stem(&self) -> Option<&str> {
        self.file_name().map(stem_from_file_name)
    }

    /// Return the final namespace-specific filename extension.
    pub fn extension(&self) -> Option<&str> {
        self.file_name().and_then(extension_from_file_name)
    }

    /// Iterate over namespace-specific filename extensions without allocating.
    pub fn extensions(&self) -> Extensions<'_> {
        extensions_from_file_name(self.file_name())
    }

    /// Infer the final namespace-specific MIME type, defaulting to octet-stream.
    pub fn mime_type(&self) -> MimeType {
        self.extension()
            .and_then(|extension| MimeType::from_extension(extension).ok())
            .unwrap_or_default()
    }

    /// Infer the namespace-specific media type and transparent encodings.
    pub fn media_type(&self) -> MediaType {
        MediaType::from_file_name(self.file_name().unwrap_or(""))
    }

    /// Replace the final URN path filename segment.
    ///
    /// The resulting value must retain valid URN namespace syntax. An error
    /// leaves the URN unchanged.
    pub fn set_file_name(&mut self, value: &str) -> Result<()> {
        let mut path = self.resource_path();
        path.set_file_name(value)?;
        self.replace_resource_path(path)
    }

    /// Replace the URN path filename stem while preserving its final extension.
    ///
    /// An error leaves the URN unchanged.
    pub fn set_stem(&mut self, value: &str) -> Result<()> {
        let mut path = self.resource_path();
        path.set_stem(value)?;
        self.replace_resource_path(path)
    }

    /// Add or replace the final URN path filename extension.
    ///
    /// An error leaves the URN unchanged.
    pub fn set_extension(&mut self, value: &str) -> Result<()> {
        let mut path = self.resource_path();
        path.set_extension(value)?;
        self.replace_resource_path(path)
    }

    /// Replace the complete compound URN path extension chain.
    ///
    /// An error leaves the URN unchanged.
    pub fn set_extensions<I, S>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut path = self.resource_path();
        path.set_extensions(values)?;
        self.replace_resource_path(path)
    }

    /// Add or replace the final namespace-specific suffix using a MIME type.
    ///
    /// An error leaves the URN unchanged.
    pub fn set_mime_type(&mut self, value: MimeType) -> Result<()> {
        let mut path = self.resource_path();
        path.set_mime_type(value)?;
        self.replace_resource_path(path)
    }

    /// Replace the namespace-specific suffix chain from a media type.
    ///
    /// An error leaves the URN unchanged.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let mut path = self.resource_path();
        path.set_media_type(value)?;
        self.replace_resource_path(path)
    }

    /// Remove the final URN path extension and report whether one existed.
    pub fn remove_extension(&mut self) -> bool {
        let mut path = self.resource_path();
        if !path.remove_extension() {
            return false;
        }
        self.replace_resource_path(path).is_ok()
    }

    /// Remove every URN path extension and report whether any existed.
    pub fn clear_extensions(&mut self) -> bool {
        let mut path = self.resource_path();
        if !path.clear_extensions() {
            return false;
        }
        self.replace_resource_path(path).is_ok()
    }

    fn resource_path(&self) -> UriPath {
        UriPath(SmolStr::new(self.namespace_specific()))
    }

    fn replace_resource_path(&mut self, path: UriPath) -> Result<()> {
        let mut full_path = SmolStrBuilder::new();
        full_path.push_str(self.namespace());
        full_path.push(':');
        full_path.push_str(path.as_str());
        let mut candidate = self.0.clone();
        candidate.path = UriPath(full_path.into());
        *self = Self::from_uri(candidate)?;
        Ok(())
    }

    /// Return a deterministic cross-language hash of the canonical URN.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

fn validate_urn_query(query: Option<&str>, base: usize) -> Result<()> {
    let Some(query) = query else {
        return Ok(());
    };
    let Some(kind) = query.as_bytes().first() else {
        return Err(parse_error(
            "urn",
            base,
            "URN query must start with `?+` or `?=` and contain a component",
        ));
    };
    match kind {
        b'+' => {
            let components = &query[1..];
            let (resolution, urn_query) = components
                .split_once("?=")
                .map_or((components, None), |(resolution, urn_query)| {
                    (resolution, Some(urn_query))
                });
            if resolution.is_empty() || urn_query.is_some_and(str::is_empty) {
                return Err(parse_error(
                    "urn",
                    base + 1,
                    "URN resolution and query components must not be empty",
                ));
            }
            if resolution.contains("?+")
                || resolution.contains("?=")
                || urn_query
                    .is_some_and(|component| component.contains("?+") || component.contains("?="))
            {
                return Err(parse_error(
                    "urn",
                    base + 1,
                    "URN resolution/query component marker may appear only once and in order",
                ));
            }
        }
        b'=' => {
            let urn_query = &query[1..];
            if urn_query.is_empty() || urn_query.contains("?+") || urn_query.contains("?=") {
                return Err(parse_error(
                    "urn",
                    base + 1,
                    "URN query component must be non-empty and contain one marker",
                ));
            }
        }
        _ => {
            return Err(parse_error(
                "urn",
                base,
                "URN query must start with `?+` or `?=`",
            ));
        }
    }
    Ok(())
}

fn validate_namespace(namespace: &str, base: usize) -> Result<()> {
    if !(2..=32).contains(&namespace.len()) {
        return Err(parse_error(
            "urn",
            base + namespace.len().min(32),
            "URN namespace must contain between 2 and 32 ASCII characters",
        ));
    }
    let bytes = namespace.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(parse_error(
            "urn",
            base,
            "URN namespace must be alphanumeric with interior hyphens only",
        ));
    }
    if !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(parse_error(
            "urn",
            base + bytes.len() - 1,
            "URN namespace must be alphanumeric with interior hyphens only",
        ));
    }
    if let Some(position) = bytes
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'-'))
    {
        return Err(parse_error(
            "urn",
            base + position,
            "URN namespace must be alphanumeric with interior hyphens only",
        ));
    }
    if namespace.eq_ignore_ascii_case("urn") {
        return Err(parse_error(
            "urn",
            base,
            "the `urn` namespace identifier is reserved",
        ));
    }
    Ok(())
}

impl FromStr for Urn {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_uri(Uri::from_str(value)?)
    }
}

impl TryFrom<Uri> for Urn {
    type Error = Error;

    fn try_from(value: Uri) -> Result<Self> {
        Self::from_uri(value)
    }
}

impl TryFrom<&Uri> for Urn {
    type Error = Error;

    fn try_from(value: &Uri) -> Result<Self> {
        Self::from_uri(value.clone())
    }
}

impl From<Urn> for Uri {
    fn from(value: Urn) -> Self {
        value.into_uri()
    }
}

impl From<&Urn> for Uri {
    fn from(value: &Urn) -> Self {
        value.clone().into_uri()
    }
}

impl AsRef<Uri> for Urn {
    fn as_ref(&self) -> &Uri {
        &self.0
    }
}

impl AsRef<UriPath> for Urn {
    fn as_ref(&self) -> &UriPath {
        self.path()
    }
}

impl fmt::Display for Urn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'a> IntoIterator for &'a Urn {
    type Item = &'a str;
    type IntoIter = PathSegments<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.path_segments()
    }
}

impl Serialize for Urn {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Urn {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_uri(Uri::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{Authority, Scheme, Uri, UriPath, Url, Urn};

    #[test]
    fn canonical_uri_round_trip() {
        let uri = Uri::from_str("HTTPS://example.test/a%2fb?q=x#part").unwrap();
        assert_eq!(uri.to_string(), "https://example.test/a%2Fb?q=x#part");
        assert_eq!(Uri::from_str(&uri.to_string()).unwrap(), uri);
    }

    #[test]
    fn components_validate_and_serialize_as_strings() {
        let scheme = Scheme::from_str("FILE").unwrap();
        let authority = Authority::from_str("").unwrap();
        let path = UriPath::from_str("/tmp/a.csv").unwrap();
        assert_eq!(serde_json::to_string(&scheme).unwrap(), "\"file\"");
        assert_eq!(authority.as_str(), "");
        assert_eq!(path.extension(), Some("csv"));
    }

    #[test]
    fn specialized_values_validate() {
        assert!(Url::from_str("https://example.test").is_ok());
        assert!(Url::from_str("https:///path").is_err());
        let urn = Urn::from_str("URN:ISBN:9780131103627").unwrap();
        assert_eq!(urn.to_string(), "urn:isbn:9780131103627");
    }
}
