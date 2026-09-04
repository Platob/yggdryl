//! Resource identifiers and their shared component model.

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

use crate::{Error, Result, stable_hash_display};
use crate::{MediaType, MimeType, Scheme};

mod authority;
mod extensions;
mod glob;
mod hive;
mod parser;
mod path;
pub(crate) mod pattern;
mod url;
mod urn;

pub use authority::Authority;
pub use extensions::Extensions;
pub use path::{Parents, PathSegments, UriParents, UriPath};
pub use url::{Url, UrlParents};
pub use urn::Urn;

use authority::*;
use extensions::*;
use parser::*;
use path::file_name_from_path;

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

#[cfg(test)]
mod tests;
