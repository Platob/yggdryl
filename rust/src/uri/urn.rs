//! Uniform resource names.

use super::*;

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
