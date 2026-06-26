//! The [`MediaType`] stack — an ordered `Vec<MimeType>` describing a layered
//! file, e.g. `data.csv.gz` → `[Csv, Gzip]`.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::media::mime::resolve_name;
use crate::{Mapping, MediaError, MimeType};

/// An ordered stack of [`MimeType`]s describing a layered file, innermost content
/// first. Parsing `data.csv.gz` yields `MediaType([MimeType::Csv, MimeType::Gzip])`.
///
/// ```
/// use yggdryl_core::{MediaType, MimeType};
///
/// let stack = MediaType::from_path("/tmp/data.csv.gz");
/// assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
/// assert_eq!(stack.first(), Some(&MimeType::Csv));
/// assert_eq!(stack.last(), Some(&MimeType::Gzip));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaType {
    types: Vec<MimeType>,
}

/// Splits a file name into its extensions, e.g. `"a.csv.gz"` → `["csv", "gz"]` and
/// `".bashrc"` → `[]` (a leading dot starts a dotfile, not an extension).
fn name_extensions(name: &str) -> Vec<&str> {
    let after_first_dot = if name.len() > 1 {
        name[1..].find('.').map(|i| i + 2)
    } else {
        None
    };
    match after_first_dot {
        Some(idx) => name[idx..].split('.').filter(|s| !s.is_empty()).collect(),
        None => Vec::new(),
    }
}

impl MediaType {
    /// Builds a [`MediaType`] from an ordered list of [`MimeType`]s.
    pub fn new(types: Vec<MimeType>) -> MediaType {
        MediaType { types }
    }

    /// Builds the stack from an ordered list of file extensions, keeping those that
    /// resolve in the registry (unknown extensions are skipped, with a `warn`).
    /// `["csv", "gz"]` yields `[Csv, Gzip]`.
    pub fn from_extensions(extensions: &[&str]) -> MediaType {
        let mut types = Vec::new();
        for ext in extensions {
            if let Some(mime) = MimeType::from_extension(ext) {
                types.push(mime);
            } else {
                log_event!(warn, "MediaType: skipped unknown extension {ext:?}");
            }
        }
        MediaType { types }
    }

    /// Builds a single-type stack from one file extension (empty if unknown).
    pub fn from_extension(extension: &str) -> MediaType {
        MediaType::from_extensions(&[extension])
    }

    /// Builds the stack from a path's file name, mapping each `.`-extension that
    /// resolves in the registry (unknown extensions are skipped). `data.csv.gz`
    /// yields `[Csv, Gzip]`.
    pub fn from_path(path: &str) -> MediaType {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let media = MediaType::from_extensions(&name_extensions(name));
        log_event!(
            debug,
            "MediaType::from_path {path:?} -> {} type(s)",
            media.types.len()
        );
        media
    }

    /// The ordered [`MimeType`]s, innermost content first.
    pub fn types(&self) -> &[MimeType] {
        &self.types
    }

    /// The innermost (content) type, e.g. `Csv` for `data.csv.gz`.
    pub fn first(&self) -> Option<&MimeType> {
        self.types.first()
    }

    /// The outermost (container) type, e.g. `Gzip` for `data.csv.gz`.
    pub fn last(&self) -> Option<&MimeType> {
        self.types.last()
    }

    /// The number of types in the stack.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the stack is empty (no known extension was found).
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

impl Default for MediaType {
    /// The fallback stack: a single [`MimeType::OctetStream`]
    /// (`application/octet-stream`), used when no type can be inferred.
    fn default() -> MediaType {
        MediaType::new(vec![MimeType::default()])
    }
}

/// String/mapping parsers.
impl MediaType {
    /// Parses a path or file name into its [`MimeType`] stack (see
    /// [`from_path`](MediaType::from_path)), or resolves a bare name like `"gzip"`
    /// or `"json"` to a single-type stack. Only an empty input is an error.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<MediaType, MediaError> {
        if input.is_empty() {
            return Err(MediaError::Empty);
        }
        // A bare token (no path separators or dots) is a single MIME name.
        if !input.contains(['/', '\\', '.']) {
            return Ok(match resolve_name(input) {
                Some(mime) => MediaType::new(vec![mime]),
                None => {
                    log_event!(
                        warn,
                        "MediaType::from_str: unknown name {input:?}, empty stack"
                    );
                    MediaType::new(Vec::new())
                }
            });
        }
        Ok(MediaType::from_path(input))
    }

    /// Builds the stack from a [`Mapping`]; reads the `types` key, a comma-
    /// separated list of MIME strings (the inverse of
    /// [`to_mapping`](MediaType::to_mapping)).
    pub fn from_mapping(fields: &Mapping) -> Result<MediaType, MediaError> {
        let types = fields
            .get("types")
            .map(|list| {
                list.split(',')
                    .filter(|t| !t.is_empty())
                    .map(MimeType::from_mime)
                    .collect()
            })
            .unwrap_or_default();
        Ok(MediaType::new(types))
    }
}

impl fmt::Display for MediaType {
    /// Renders the canonical extension chain, e.g. `"csv.gz"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_str(true))
    }
}

/// Component rendering: the inverse of the `from_str` / `from_mapping` parsers.
impl MediaType {
    /// Renders the canonical extension chain, e.g. `"csv.gz"` (the inverse of
    /// [`from_path`](MediaType::from_path) for canonical extensions).
    pub fn to_str(&self, _encode: bool) -> String {
        self.types
            .iter()
            .filter_map(MimeType::extension)
            .collect::<Vec<_>>()
            .join(".")
    }

    /// The inverse of [`from_mapping`](MediaType::from_mapping): a single `types`
    /// key holding the comma-separated MIME strings (e.g. `"text/csv,application/gzip"`).
    pub fn to_mapping(&self) -> Mapping {
        let types = self
            .types
            .iter()
            .map(MimeType::mime)
            .collect::<Vec<_>>()
            .join(",");
        Mapping::from([("types".to_string(), types)])
    }
}

/// Serialises as a sequence of MIME strings (e.g. `["text/csv","application/gzip"]`)
/// — lossless, since each [`MimeType`] round-trips through its canonical string.
#[cfg(feature = "serde")]
impl serde::Serialize for MediaType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(&self.types)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MediaType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<MediaType, D::Error> {
        let types = <Vec<MimeType> as serde::Deserialize>::deserialize(deserializer)?;
        Ok(MediaType::new(types))
    }
}
