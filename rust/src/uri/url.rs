//! Hierarchical resource URLs.

use super::*;

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

    /// Return the S3 endpoint host and explicit port, excluding a virtual bucket.
    pub fn s3_endpoint(&self) -> Option<&str> {
        self.0.s3_endpoint()
    }

    /// Return the S3 bucket name when this is an `s3` URL.
    pub fn bucket(&self) -> Option<&str> {
        self.0.bucket()
    }

    /// Infer an AWS region from a recognized S3 hostname.
    pub fn region(&self) -> Option<&str> {
        self.0.region()
    }

    /// Return whether this S3 URL puts its bucket in the endpoint hostname.
    pub fn is_s3_virtual(&self) -> bool {
        self.0.is_s3_virtual()
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
