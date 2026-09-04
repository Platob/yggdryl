//! URI paths and parent iteration.

use super::*;

/// A validated slash-separated URI path.
///
/// The path is always a concrete value. Empty paths are represented by the
/// empty string rather than an optional value.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UriPath(pub(super) SmolStr);

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

pub(super) fn file_name_from_path(value: &str) -> Option<&str> {
    if value.is_empty() || value.ends_with('/') {
        return None;
    }
    value.rsplit('/').next().filter(|name| !name.is_empty())
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

/// An iterator from a URI's immediate parent up to its root.
///
/// Created by [`Uri::parents`]. Each item preserves the source scheme,
/// authority, query, and fragment.
#[derive(Clone, Debug)]
pub struct UriParents<'a> {
    pub(super) source: &'a Uri,
    pub(super) paths: Parents,
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

impl AsRef<UriPath> for Uri {
    fn as_ref(&self) -> &UriPath {
        self.path()
    }
}
