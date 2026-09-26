//! Hierarchical resource URLs.

use std::hash::Hash;

use super::*;

/// A validated URL backed directly by a canonical [`Uri`].
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Url(Uri);

impl PartialOrd for Url {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Url {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Url {
    /// Parse and validate a URL.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Convert a file path to a canonical `file:` URL.
    ///
    /// A path that names no root is joined onto the process's working
    /// directory, read once here. A `file:` URL is absolute by definition, so
    /// that is the only URL a relative path has, and it is the one every host
    /// filesystem call resolves it to; [`Uri::from_path`] is the door that
    /// keeps such a path relative.
    ///
    /// # Errors
    ///
    /// Returns [`Uri::from_path`]'s refusal, or the working directory's own
    /// failure when the process cannot read one.
    pub fn from_path(value: impl AsRef<Path>) -> Result<Self> {
        Uri::from_path(value.as_ref())?.locator()
    }

    /// Validate and wrap an existing URI as a URL.
    ///
    /// This is the strict door: a name is refused as a name rather than
    /// resolved. [`Uri::locator`] is what answers where a name is.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the URI is a name, or carries no
    /// hierarchical authority.
    pub fn from_uri(value: Uri) -> Result<Self> {
        if value.scheme() == &Scheme::URN {
            return Err(parse_error("url", 0, "URN values are not URLs"));
        }
        if value.scheme() == &Scheme::ARN {
            return Err(parse_error("url", 0, "ARN values are not URLs"));
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

    /// Read one location a caller named: a URL, or a path to root.
    ///
    /// This is the door for text a caller typed to say *where* - the argument
    /// behind a binding's `location` parameter - and it is deliberately not
    /// [`Self::from_str`]. Text carrying a scheme is a URL and a malformed one
    /// is refused as a URL rather than read as a file named after it; text
    /// carrying none is a path and reaches [`Self::from_path`], working
    /// directory and all. Text naming a resource rather than a place - a URN,
    /// an ARN - resolves through [`Uri::locator`], because a caller saying
    /// *where* by name is still saying where.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(
    ///     Url::from_location("arn:aws:s3:::trades/2026/part.parquet")?.to_string(),
    ///     "s3://trades/2026/part.parquet"
    /// );
    /// assert!(
    ///     Url::from_location("urn:lake:trades:part.parquet")?
    ///         .to_string()
    ///         .ends_with("/lake/trades/part.parquet")
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Stored data takes the other door. A `url` column holding `data/x` would
    /// otherwise name a different file on every machine that read it, so
    /// [`DataType::url`](crate::DataType::url) keeps the strict parse.
    ///
    /// # Errors
    ///
    /// Returns the URL parse failure, or [`Self::from_path`]'s.
    pub fn from_location(value: &str) -> Result<Self> {
        // `Uri::from_str` already decides URL against path: text naming no
        // usable scheme is read as a filesystem path and left relative, and
        // `Uri::locator` is the one door that says where any identifier is -
        // a name, a relative path, or a location that is already one.
        Uri::from_str(value)?.locator()
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
    /// This is [`Path::join`] for URLs, so it takes a *platform* path and each
    /// component is a name rather than URI text: what a segment cannot carry
    /// literally is percent-encoded, which is what lets a real filename -
    /// `100%.csv`, `café.csv`, `a b.csv` - be joined at all.
    /// [`joinpath`](Self::joinpath) is the door for text that is already URI
    /// syntax. `.` and `..` resolve exactly as [`joinpath`](Self::joinpath)
    /// resolves them, and an absolute `path` replaces this URL's path rather
    /// than extending it.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Url::from_str("file:///lake")?;
    ///
    /// assert_eq!(lake.join_path("100%.csv")?.to_string(), "file:///lake/100%25.csv");
    /// assert_eq!(
    ///     lake.join_path("year=2024/a b.csv")?.to_string(),
    ///     "file:///lake/year=2024/a%20b.csv"
    /// );
    /// // A name is one segment: its separators are escaped, never promoted.
    /// assert_eq!(lake.join_path("a/b")?, lake.joinpath("a")?.joinpath("b")?);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a component is not valid UTF-8, or when the joined
    /// path is not valid for this URL.
    pub fn join_path(&self, path: impl AsRef<Path>) -> Result<Self> {
        use std::path::Component;

        let path = path.as_ref();
        let mut joined = self.clone();
        for component in path.components() {
            joined = match component {
                // An absolute path replaces this URL outright, so the whole
                // of it is converted and the components after the root are
                // already in that result.
                Component::Prefix(_) | Component::RootDir => return Self::from_path(path),
                Component::CurDir => continue,
                Component::ParentDir => joined.joinpath("..")?,
                Component::Normal(segment) => {
                    let segment = segment
                        .to_str()
                        .ok_or_else(|| parse_error("path", 0, "file path must be valid UTF-8"))?;
                    joined.joinpath(&percent_encode_segment(segment))?
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
    /// into version control or virtual environments by default. The last
    /// segment is what is judged, so a container spelled `.git/` is as private
    /// as `.git`.
    pub fn is_private(&self) -> bool {
        self.path()
            .segments()
            .next_back()
            .is_some_and(|name| name.starts_with('.'))
    }

    /// Return whether the path ends in `/`, which names a container.
    ///
    /// A location spelled `lake/` says what it is before anything looks: the
    /// generic roles answer [`IOKind::Directory`](crate::IOKind::Directory)
    /// from it without touching the backing store, and a child resolved by
    /// such a name is a folder handle rather than a leaf. The root path `/`
    /// counts, because a root can only hold others.
    pub fn has_trailing_slash(&self) -> bool {
        self.path().has_trailing_slash()
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

    /// Return the network hostname under the URI's store-aware rules.
    pub fn hostname(&self) -> Option<&str> {
        self.0.hostname()
    }

    /// Return the store endpoint host and explicit port, excluding a virtual
    /// container.
    pub fn store_endpoint(&self) -> Option<&str> {
        self.0.store_endpoint()
    }

    /// Return the container name when this URL addresses an object store, per
    /// [`Uri::bucket`].
    pub fn bucket(&self) -> Option<&str> {
        self.0.bucket()
    }

    /// Return the Azure storage account this URL names, per [`Uri::account`].
    pub fn account(&self) -> Option<&str> {
        self.0.account()
    }

    /// Return the object key when this URL addresses an object store, per
    /// [`Uri::key`].
    pub fn key(&self) -> Option<&str> {
        self.0.key()
    }

    /// Infer a region from a recognized store hostname.
    pub fn region(&self) -> Option<&str> {
        self.0.region()
    }

    /// Return whether this URL puts its container in the endpoint hostname.
    pub fn is_virtual_hosted(&self) -> bool {
        self.0.is_virtual_hosted()
    }

    /// Return the concrete URL path.
    pub fn path(&self) -> &UriPath {
        self.0.path()
    }

    /// Return the URL path as text, decoding its escapes when asked.
    ///
    /// # Errors
    ///
    /// As [`Uri::path_text`].
    pub fn path_text(&self, decode: bool) -> Result<Cow<'_, str>> {
        self.0.path_text(decode)
    }

    /// Return URL query text without `?`, decoding its escapes when asked.
    ///
    /// # Errors
    ///
    /// As [`Uri::query`].
    pub fn query(&self, decode: bool) -> Result<Option<Cow<'_, str>>> {
        self.0.query(decode)
    }

    /// Address the URL query as the `key=value` pairs it spells.
    ///
    /// # Errors
    ///
    /// As [`Uri::parameters`].
    pub fn parameters(&self, decode: bool) -> Result<Parameters<'_>> {
        self.0.parameters(decode)
    }

    /// Replace the URL query with the pairs `parameters` holds.
    ///
    /// # Errors
    ///
    /// As [`Uri::set_parameters`].
    pub fn set_parameters(&mut self, parameters: &Parameters<'_>) -> Result<()> {
        self.0.set_parameters(parameters)?;
        Ok(())
    }

    /// Replace the URL query text, or clear it with `None`.
    ///
    /// # Errors
    ///
    /// As [`Uri::set_query`].
    pub fn set_query(&mut self, query: Option<&str>) -> Result<()> {
        self.0.set_query(query)?;
        Ok(())
    }

    /// Replace or remove the URL fragment, from text that is not URI syntax.
    ///
    /// This is how a location inside a resource is addressed: an archive
    /// member's path lives in the fragment, so `file:///lake/day.zip` and
    /// `file:///lake/day.zip#trades/eu.csv` name the archive and one member of
    /// it without either becoming a path segment of the other.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut url = Url::from_str("file:///lake/day.zip")?;
    /// url.set_fragment(Some("trades/eu ndx.csv"))?;
    ///
    /// assert_eq!(url.to_string(), "file:///lake/day.zip#trades/eu%20ndx.csv");
    /// assert_eq!(url.fragment(true)?.as_deref(), Some("trades/eu ndx.csv"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// As [`Uri::set_fragment`].
    pub fn set_fragment(&mut self, fragment: Option<&str>) -> Result<()> {
        let mut candidate = self.0.clone();
        candidate.set_fragment(fragment)?;
        self.replace_uri(candidate)
    }

    /// Return URL fragment text without `#`, decoding its escapes when asked.
    ///
    /// # Errors
    ///
    /// As [`Uri::fragment`].
    pub fn fragment(&self, decode: bool) -> Result<Option<Cow<'_, str>>> {
        self.0.fragment(decode)
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

    /// Resolve a URI reference against this URL, as RFC 3986 section 5.2
    /// does.
    ///
    /// This is what a `Location` header, a `Link` target or a `next` URL in
    /// a document is read through: the reference is whatever the server
    /// wrote, and this URL is where it was read. An absolute reference is
    /// itself; `//host/path` keeps the scheme; `/path` replaces the path; a
    /// relative path merges onto this path with its dot segments removed;
    /// `?q` keeps the path; `#f` keeps the path and the query; an empty
    /// reference is this URL without its fragment. A reference spelling this
    /// URL's own scheme without an authority - `http:g` - is read the
    /// backward-compatible way the RFC describes, as the relative reference
    /// after the scheme.
    ///
    /// ```
    /// use yggdryl::Url;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let page = Url::from_str("https://api.example.com/v1/items?page=1")?;
    ///
    /// assert_eq!(
    ///     page.join_reference("../users?after=x")?.to_string(),
    ///     "https://api.example.com/users?after=x"
    /// );
    /// assert_eq!(
    ///     page.join_reference("//cdn.example.com/a.json")?.to_string(),
    ///     "https://cdn.example.com/a.json"
    /// );
    /// assert_eq!(
    ///     page.join_reference("#top")?.to_string(),
    ///     "https://api.example.com/v1/items?page=1#top"
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a parse error naming the byte of the reference that is not URI
    /// syntax, or the reason an absolute reference is not a URL - a scheme
    /// with no authority, such as `mailto:`, names no location this type can
    /// hold.
    pub fn join_reference(&self, reference: &str) -> Result<Self> {
        const TARGET: &str = "url reference";

        let (before_fragment, fragment) = match reference.find('#') {
            Some(position) => (&reference[..position], Some(&reference[position + 1..])),
            None => (reference, None),
        };
        let (hierarchy, query) = match before_fragment.find('?') {
            Some(position) => (
                &before_fragment[..position],
                Some(&before_fragment[position + 1..]),
            ),
            None => (before_fragment, None),
        };
        if let Some(fragment) = fragment {
            let offset = before_fragment.len() + 1;
            validate_component(fragment, TARGET, offset, is_query_fragment_byte)?;
        }
        if let Some(query) = query {
            validate_component(query, TARGET, hierarchy.len() + 1, is_query_fragment_byte)?;
        }

        // A reference with a scheme is absolute - unless it is this URL's own
        // scheme with no authority behind it, which the RFC reads as the
        // relative reference that follows the scheme.
        if let Some(scheme_end) = reference_scheme_end(hierarchy) {
            let same_scheme = hierarchy[..scheme_end].eq_ignore_ascii_case(self.scheme().as_str());
            if same_scheme && !hierarchy[scheme_end + 1..].starts_with("//") {
                return self.join_reference(&reference[scheme_end + 1..]);
            }
            return Self::from_str(reference).map_err(|error| {
                offset_parse_error(error, TARGET, 0, "an absolute reference must be a URL")
            });
        }

        let mut composed = String::with_capacity(
            self.scheme().as_str().len()
                + self.authority().as_str().len()
                + self.path().as_str().len()
                + reference.len()
                + 4,
        );
        composed.push_str(self.scheme().as_str());
        composed.push_str("://");
        if let Some(network) = hierarchy.strip_prefix("//") {
            let path_start = network.find('/').unwrap_or(network.len());
            let (authority, path) = network.split_at(path_start);
            validate_component(authority, TARGET, 2, is_authority_byte)?;
            validate_component(path, TARGET, 2 + path_start, is_path_byte)?;
            composed.push_str(authority);
            composed.push_str(&remove_dot_segments(path));
        } else {
            validate_component(hierarchy, TARGET, 0, is_path_byte)?;
            composed.push_str(self.authority().as_str());
            if hierarchy.is_empty() {
                composed.push_str(self.path().as_str());
            } else if hierarchy.starts_with('/') {
                composed.push_str(&remove_dot_segments(hierarchy));
            } else {
                // Merge: the reference replaces everything after the last
                // slash of this path, and a URL with no path at all has an
                // authority, so the reference lands under its root.
                let base = self.path().as_str();
                let kept = base.rfind('/').map_or(0, |slash| slash + 1);
                let mut merged = String::with_capacity(kept + hierarchy.len() + 1);
                if base.is_empty() {
                    merged.push('/');
                }
                merged.push_str(&base[..kept]);
                merged.push_str(hierarchy);
                composed.push_str(&remove_dot_segments(&merged));
            }
        }
        match query {
            Some(query) => {
                composed.push('?');
                composed.push_str(query);
            }
            None if hierarchy.is_empty() => {
                if let Some(query) = self.0.query(false)? {
                    composed.push('?');
                    composed.push_str(&query);
                }
            }
            None => {}
        }
        if let Some(fragment) = fragment {
            composed.push('#');
            composed.push_str(fragment);
        }
        Self::from_str(&composed).map_err(|error| {
            offset_parse_error(error, TARGET, 0, "the reference does not resolve to a URL")
        })
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

/// Where the scheme of a URI reference ends, when it carries one.
///
/// RFC 3986 section 3.1: a letter, then letters, digits, `+`, `-` and `.`,
/// then the colon - and nothing before that colon may be a slash, because a
/// colon inside a path segment is part of the segment.
fn reference_scheme_end(hierarchy: &str) -> Option<usize> {
    let end = hierarchy.find(':')?;
    let scheme = &hierarchy.as_bytes()[..end];
    let well_formed = scheme.first().is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
    well_formed.then_some(end)
}

/// RFC 3986 section 5.2.4, letter for letter.
///
/// Every dot segment goes, including the ones a relative reference climbs
/// past the root with, and every empty segment stays: `//` inside a path is
/// two separators the reference wrote, and `a/./` keeps the trailing slash
/// that says a container is meant. [`UriPath::normalize`] is the other
/// reading, for a path a caller composed rather than one a server wrote.
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut output = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(rest) = input
            .strip_prefix("../")
            .or_else(|| input.strip_prefix("./"))
        {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") {
            input = &input[3..];
            pop_segment(&mut output);
        } else if input == "/.." {
            input = "/";
            pop_segment(&mut output);
        } else if input == "." || input == ".." {
            input = "";
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..]
                .find('/')
                .map_or(input.len(), |slash| start + slash);
            output.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    output
}

/// Drop the last segment of `output` and the slash before it, if any.
fn pop_segment(output: &mut String) {
    match output.rfind('/') {
        Some(slash) => output.truncate(slash),
        None => output.clear(),
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/uri/url.rs` pins and a caller cannot reach.
    //!
    //! The canonical rendering a reader shares across its rows is
    //! crate-internal, so whether a mutation clears it is pinned here. The
    //! rendering belongs to the canonical [`Uri`](crate::Uri) a URL narrows,
    //! and a mutation reaches it through that value.

    use super::Url;
    use crate::Str;

    /// The URL's canonical rendering as the lazily cached shared string.
    pub fn shared_text(url: &Url) -> &Str {
        <Url as AsRef<crate::Uri>>::as_ref(url).shared_text()
    }
}
