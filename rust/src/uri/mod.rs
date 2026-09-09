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
mod parameters;
mod parser;
mod path;
pub(crate) mod pattern;
mod url;
mod urn;

pub use authority::Authority;
pub use extensions::Extensions;
pub(crate) use hive::hive_partitions_of;
pub use parameters::Parameters;
pub(crate) use parser::{percent_decode, percent_encode_segment};
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
        // Either separator roots a path: `\\data` is the Windows spelling of
        // `/data`, so both reach the same absolute `file:///data` rather than
        // one of them landing in the authority-less `file:/data` form.
        let local_absolute = value.starts_with(['/', '\\']);
        let (path_input, prefix_slash) = if local_absolute {
            (value.trim_start_matches(['/', '\\']), true)
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
        if let Some(query) = self.query.as_deref() {
            validate_component(query, "uri query", 0, is_query_fragment_byte)?;
        }
        if let Some(fragment) = self.fragment.as_deref() {
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
    /// For an object store, an authority or first path part ending in `.com`,
    /// `.io`, or `.net` is a hostname, as is one carrying a port, spelled as an
    /// IP literal, or named `localhost`; any other first part is a container
    /// name.
    pub fn hostname(&self) -> Option<&str> {
        if let Some(location) = self.store_location() {
            return location.hostname;
        }
        (!self.authority.is_empty()).then(|| self.authority.host())
    }

    /// Return the store endpoint host and explicit port, excluding a virtual
    /// container.
    pub fn store_endpoint(&self) -> Option<&str> {
        self.store_location().and_then(|location| location.endpoint)
    }

    /// Return the container name when this URI addresses an object store.
    ///
    /// The bucket on Amazon S3 and Google Cloud Storage, the container on Azure
    /// Blob Storage: one name, because it is one position in the location.
    pub fn bucket(&self) -> Option<&str> {
        self.store_location().and_then(|location| location.bucket)
    }

    /// Return the Azure storage account when the location names one.
    ///
    /// `abfss://data@trades.dfs.core.windows.net/lake` names `trades`; a bare
    /// `az://data/lake` names none, and the client is configured with it.
    pub fn account(&self) -> Option<&str> {
        self.store_location().and_then(|location| location.account)
    }

    /// Infer a region from a recognized store hostname.
    ///
    /// This borrows the region from the URI and performs no network lookup.
    /// Only AWS and Google's regional endpoints state one.
    pub fn region(&self) -> Option<&str> {
        self.store_location().and_then(|location| location.region)
    }

    /// Return whether a store URI puts its container in the endpoint hostname.
    pub fn is_virtual_hosted(&self) -> bool {
        self.store_location()
            .is_some_and(|location| location.virtual_addressing)
    }

    /// Return the object key when this URI addresses an object store.
    ///
    /// The key is the path below the container, spelled as the path spells it:
    /// percent escapes stay escaped and a trailing slash stays, so a prefix
    /// reads as `lake/` and the container root as `""`. Decoding the escapes is
    /// the storage client's business, because a key can hold what a URI path
    /// cannot.
    ///
    /// ```
    /// use yggdryl::Uri;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(Uri::from_str("s3://trades/2026/part.parquet")?.key(), Some("2026/part.parquet"));
    /// assert_eq!(Uri::from_str("gs://trades/2026/")?.key(), Some("2026/"));
    /// assert_eq!(Uri::from_str("s3://trades/")?.key(), Some(""));
    /// assert_eq!(
    ///     Uri::from_str("s3://s3.eu-west-3.amazonaws.com/trades/part.parquet")?.key(),
    ///     Some("part.parquet")
    /// );
    /// assert_eq!(
    ///     Uri::from_str("abfss://data@trades.dfs.core.windows.net/lake/part.parquet")?.key(),
    ///     Some("lake/part.parquet")
    /// );
    /// assert_eq!(Uri::from_str("https://example.com/part.parquet")?.key(), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn key(&self) -> Option<&str> {
        self.store_location().map(|location| location.key)
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
    ///
    /// `decode` chooses which text: the query's own bytes, or the text its
    /// percent escapes stand for. Decoding borrows unless an escape is there
    /// to decode, and it decodes the component as text - `%26` becomes a
    /// literal `&`, not a new pair - so [`parameters`](Self::parameters) is
    /// what reads a query as its pairs.
    ///
    /// # Errors
    ///
    /// Returns a parse error when `decode` is set and an escape does not stand
    /// for UTF-8.
    pub fn query(&self, decode: bool) -> Result<Option<Cow<'_, str>>> {
        self.query
            .as_deref()
            .map(|query| decoded_component(query, decode, "uri query"))
            .transpose()
    }

    /// Return the path as text, decoding its escapes when asked.
    ///
    /// The path keeps its own syntax: `%2F` inside a segment decodes to a
    /// literal `/` in the returned text rather than to a segment boundary, so
    /// [`path_segments`](Self::path_segments) stays the way to walk structure.
    ///
    /// # Errors
    ///
    /// Returns a parse error when `decode` is set and an escape does not stand
    /// for UTF-8.
    pub fn path_text(&self, decode: bool) -> Result<Cow<'_, str>> {
        self.path.text(decode)
    }

    /// Address the query as the `key=value` pairs it spells.
    ///
    /// The view borrows this URI, so it reads without copying the query, and
    /// [`set_parameters`](Self::set_parameters) is what writes an edited view
    /// back. A URI with no query answers with an empty view rather than an
    /// error, because "no pairs" is what no query means.
    ///
    /// # Errors
    ///
    /// Returns a parse error when `decode` is set and an escape does not stand
    /// for UTF-8.
    pub fn parameters(&self, decode: bool) -> Result<Parameters<'_>> {
        Parameters::from_query(self.query.as_deref().unwrap_or_default(), decode)
    }

    /// Replace the query with the pairs `parameters` holds.
    ///
    /// A view holding no pair clears the query. An error leaves the URI
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Returns the validation failure of the query the pairs spell.
    pub fn set_parameters(&mut self, parameters: &Parameters<'_>) -> Result<()> {
        self.set_query(parameters.into_query().as_deref())
    }

    /// Replace the query text, or clear it with `None`.
    ///
    /// The value is the component itself, without `?`. An error leaves the URI
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the text is not a valid query component.
    pub fn set_query(&mut self, query: Option<&str>) -> Result<()> {
        // Validation and canonical form are one step here, as they are at every
        // other ingest: a component that skipped `%2f` -> `%2F` would compare,
        // order, and hash as a different URI from the same text parsed.
        let query = validate_optional_component(
            query.map(SmolStr::from),
            "uri query",
            is_query_fragment_byte,
        )?;
        let mut candidate = self.clone();
        candidate.query = query;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Replace or remove the fragment, from text that is not URI syntax.
    ///
    /// A fragment is one opaque component rather than a composite of parts, so
    /// this takes the value itself and percent-encodes what the syntax cannot
    /// carry - which is what makes [`fragment(true)`](Self::fragment) read
    /// back exactly what was set, for a value that was never URI text to begin
    /// with. [`set_query`](Self::set_query) takes formed query text instead,
    /// because a query is that composite and [`Parameters`](crate::uri::Parameters)
    /// is how its parts are set.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the resulting URI is not valid. An error
    /// leaves the URI unchanged.
    pub fn set_fragment(&mut self, fragment: Option<&str>) -> Result<()> {
        let fragment = fragment.map(|fragment| {
            SmolStr::new(percent_encode(fragment, is_query_fragment_byte).as_ref())
        });
        let fragment =
            validate_optional_component(fragment, "uri fragment", is_query_fragment_byte)?;
        let mut candidate = self.clone();
        candidate.fragment = fragment;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Return fragment text without `#`, if it was present.
    ///
    /// `decode` reads as it does on [`query`](Self::query).
    ///
    /// # Errors
    ///
    /// Returns a parse error when `decode` is set and an escape does not stand
    /// for UTF-8.
    pub fn fragment(&self, decode: bool) -> Result<Option<Cow<'_, str>>> {
        self.fragment
            .as_deref()
            .map(|fragment| decoded_component(fragment, decode, "uri fragment"))
            .transpose()
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
            authority: SmolStr,
            path: UriPath,
            has_authority: bool,
            query: Option<SmolStr>,
            fragment: Option<SmolStr>,
        }

        let value = Representation::deserialize(deserializer)?;
        let drive_authority = value.scheme == Scheme::FILE
            && is_file_drive_authority(value.authority.as_str(), value.path.as_str());
        let authority = match Authority::from_str(value.authority.as_str()) {
            Ok(authority) => authority,
            Err(_) if drive_authority => Authority(value.authority),
            Err(error) => return Err(D::Error::custom(error)),
        };
        Self::from_parts_with_authority(
            value.scheme,
            authority,
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
