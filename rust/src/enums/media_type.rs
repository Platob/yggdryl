use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::MimeType;
use crate::{Error, Result, stable_hash_display};

/// A base MIME type and the ordered transparent encodings applied to it.
///
/// Encoding order matches HTTP `Content-Encoding`: the first item was applied
/// first and the final item is the outermost filename suffix. Clones share the
/// encoding collection, while the empty/default value has no collection
/// allocation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MediaType {
    base: MimeType,
    encodings: Option<Arc<Vec<MimeType>>>,
}

impl MediaType {
    /// Construct an unencoded media type from its base MIME type.
    pub fn new(base: MimeType) -> Self {
        Self {
            base,
            encodings: None,
        }
    }

    /// Construct a media type from a base and ordered encoding MIME values.
    ///
    /// Every encoding must satisfy [`MimeType::is_encoding`]. An error reports
    /// the offending zero-based encoding position.
    pub fn from_parts<I>(base: MimeType, encodings: I) -> Result<Self>
    where
        I: IntoIterator<Item = MimeType>,
    {
        Ok(Self {
            base,
            encodings: validate_encodings(encodings)?,
        })
    }

    /// Parse a canonical media description, MIME name, or filename/path.
    ///
    /// Encoded canonical display uses
    /// `base/type;encodings=application/gzip,application/zstd`. Plain MIME
    /// names construct an unencoded value. Other strings use compound filename
    /// inference and therefore default to `application/octet-stream` when no
    /// recognized base suffix exists.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Infer a media type from one filename extension.
    ///
    /// Transparent encoding extensions retain the default octet-stream base.
    /// Common compound aliases such as `tgz`, `tbz2`, `txz`, and `svgz` infer
    /// both their base and encoding.
    pub fn from_extension(extension: &str) -> Self {
        Self::from_extensions(std::iter::once(extension))
    }

    /// Infer a media type from filename extensions in left-to-right order.
    ///
    /// Inference retains only a trailing chain of known transparent encodings.
    /// The nearest preceding known non-encoding suffix becomes the base. An
    /// unknown suffix hides earlier information, and a missing base defaults to
    /// [`MimeType::OCTET_STREAM`].
    pub fn from_extensions<I, S>(extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut base = None;
        let mut encodings: Option<Vec<MimeType>> = None;

        for extension in extensions {
            let extension = extension.as_ref();
            if let Some((compound_base, compound_encoding)) = compound_extension(extension) {
                base = Some(compound_base);
                encodings = Some(vec![compound_encoding]);
                continue;
            }
            match MimeType::from_extension(extension) {
                Ok(mime) if mime.is_encoding() => {
                    encodings.get_or_insert_with(Vec::new).push(mime);
                }
                Ok(mime) => {
                    base = Some(mime);
                    encodings = None;
                }
                Err(_) => {
                    base = None;
                    encodings = None;
                }
            }
        }

        Self {
            base: base.unwrap_or_default(),
            encodings: encodings.filter(|values| !values.is_empty()).map(Arc::new),
        }
    }

    /// Infer a media type from a UTF-8 filename without allocating path text.
    pub fn from_file_name(file_name: &str) -> Self {
        let file_name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let search = file_name.strip_prefix('.').unwrap_or(file_name);
        let Some(first_dot) = search.find('.') else {
            return Self::default();
        };
        let extensions = &search[first_dot + 1..];
        if extensions.is_empty() {
            return Self::default();
        }
        Self::from_extensions(extensions.split('.').filter(|value| !value.is_empty()))
    }

    /// Infer a media type from all compound suffixes of a path.
    ///
    /// The filename is borrowed directly. A non-UTF-8 filename returns an
    /// error; a missing filename or suffix returns the default media type.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let Some(file_name) = path.file_name() else {
            return Ok(Self::default());
        };
        let file_name = file_name.to_str().ok_or_else(|| Error::Parse {
            target: "media path",
            position: path.as_os_str().len(),
            reason: "path filename is not valid UTF-8".into(),
        })?;
        Ok(Self::from_file_name(file_name))
    }

    /// Parse `Content-Type` and `Content-Encoding` header values together.
    ///
    /// A missing content type defaults to octet-stream. Encodings are parsed in
    /// listed/application order. Empty, `identity`, and unknown coding entries
    /// are rejected.
    pub fn from_content_headers(
        content_type: Option<&str>,
        content_encoding: Option<&str>,
    ) -> Result<Self> {
        let base = content_type
            .map(MimeType::from_content_type)
            .transpose()?
            .unwrap_or_default();
        let Some(content_encoding) = content_encoding else {
            return Ok(Self::new(base));
        };
        if content_encoding.trim_matches([' ', '\t']).is_empty() {
            return Err(Error::Parse {
                target: "content encoding",
                position: 0,
                reason: "Content-Encoding must contain at least one coding".into(),
            });
        }
        let encodings = content_encoding
            .split(',')
            .map(MimeType::from_content_coding)
            .collect::<Result<Vec<_>>>()?;
        Self::from_parts(base, encodings)
    }

    /// Borrow the underlying, unencoded MIME type.
    pub const fn base(&self) -> &MimeType {
        &self.base
    }

    /// Borrow the ordered encoding MIME values.
    pub fn encodings(&self) -> &[MimeType] {
        self.encodings.as_deref().map_or(&[], Vec::as_slice)
    }

    /// Borrow one encoding by zero-based application order.
    pub fn get_encoding(&self, index: usize) -> Option<&MimeType> {
        self.encodings().get(index)
    }

    /// Borrow the final, outermost encoding.
    pub fn encoding(&self) -> Option<&MimeType> {
        self.encodings().last()
    }

    /// Return the number of applied encodings.
    pub fn encoding_len(&self) -> usize {
        self.encodings().len()
    }

    /// Return whether at least one transparent encoding is present.
    pub const fn is_encoded(&self) -> bool {
        self.encodings.is_some()
    }

    /// Return the final preferred filename extension.
    pub fn extension(&self) -> Option<&'static str> {
        self.encoding()
            .and_then(MimeType::extension)
            .or_else(|| self.base.extension())
    }

    /// Iterate over preferred base and encoding extensions in filename order.
    pub fn extensions(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.base.extension())
            .chain(self.encodings().iter().map(MimeType::extension))
            .flatten()
    }

    /// Replace the base MIME type.
    pub fn set_base(&mut self, base: MimeType) {
        self.base = base;
    }

    /// Atomically replace the ordered encoding collection.
    ///
    /// An error leaves this value unchanged.
    pub fn set_encodings<I>(&mut self, encodings: I) -> Result<()>
    where
        I: IntoIterator<Item = MimeType>,
    {
        let encodings = validate_encodings(encodings)?;
        if self.encodings.as_deref() != encodings.as_deref() {
            self.encodings = encodings;
        }
        Ok(())
    }

    /// Append one outer encoding after validating its MIME category.
    pub fn push_encoding(&mut self, encoding: MimeType) -> Result<()> {
        if !encoding.is_encoding() {
            return Err(invalid_encoding(self.encoding_len()));
        }
        Arc::make_mut(self.encodings.get_or_insert_with(|| Arc::new(Vec::new()))).push(encoding);
        Ok(())
    }

    /// Remove every encoding, returning whether any existed.
    pub fn clear_encodings(&mut self) -> bool {
        self.encodings.take().is_some()
    }

    /// Consume this value and replace its base MIME type.
    pub fn with_base(mut self, base: MimeType) -> Self {
        self.set_base(base);
        self
    }

    /// Consume this value and atomically replace its encodings.
    pub fn try_with_encodings<I>(mut self, encodings: I) -> Result<Self>
    where
        I: IntoIterator<Item = MimeType>,
    {
        self.set_encodings(encodings)?;
        Ok(self)
    }

    /// Return whether the base has the `application` top-level type.
    pub fn is_application(&self) -> bool {
        self.base.is_application()
    }

    /// Return whether the base has the `audio` top-level type.
    pub fn is_audio(&self) -> bool {
        self.base.is_audio()
    }

    /// Return whether the base has the `font` top-level type.
    pub fn is_font(&self) -> bool {
        self.base.is_font()
    }

    /// Return whether the base has the `haptics` top-level type.
    pub fn is_haptics(&self) -> bool {
        self.base.is_haptics()
    }

    /// Return whether the base has the `image` top-level type.
    pub fn is_image(&self) -> bool {
        self.base.is_image()
    }

    /// Return whether the base has the `message` top-level type.
    pub fn is_message(&self) -> bool {
        self.base.is_message()
    }

    /// Return whether the base has the `model` top-level type.
    pub fn is_model(&self) -> bool {
        self.base.is_model()
    }

    /// Return whether the base has the `multipart` top-level type.
    pub fn is_multipart(&self) -> bool {
        self.base.is_multipart()
    }

    /// Return whether the base has the `text` top-level type.
    pub fn is_text(&self) -> bool {
        self.base.is_text()
    }

    /// Return whether the base has the `video` top-level type.
    pub fn is_video(&self) -> bool {
        self.base.is_video()
    }

    /// Return whether the underlying representation is textual.
    pub fn is_textual(&self) -> bool {
        self.base.is_textual()
    }

    /// Return whether the underlying representation is structured.
    pub fn is_structured(&self) -> bool {
        self.base.is_structured()
    }

    /// Return whether the underlying representation is tabular.
    pub const fn is_tabular(&self) -> bool {
        self.base.is_tabular()
    }

    /// Return whether the base MIME value itself denotes an encoding.
    pub const fn is_encoding(&self) -> bool {
        self.base.is_encoding()
    }

    /// Return whether the base is an archive/container.
    pub const fn is_archive(&self) -> bool {
        self.base.is_archive()
    }

    /// Return whether the encoded wire representation is certainly binary.
    pub fn is_binary(&self) -> bool {
        self.is_encoded() || self.base.is_binary()
    }

    /// Return a deterministic cross-language hash of the canonical display.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

impl Default for MediaType {
    fn default() -> Self {
        Self::new(MimeType::default())
    }
}

impl From<MimeType> for MediaType {
    fn from(value: MimeType) -> Self {
        Self::new(value)
    }
}

impl FromStr for MediaType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed_start = value.trim_start_matches([' ', '\t']);
        let leading_offset = value.len() - trimmed_start.len();
        let value = trimmed_start.trim_end_matches([' ', '\t']);
        if let Some((base, encoded_raw)) = value.split_once(';') {
            let encoded_start = encoded_raw.trim_start_matches([' ', '\t']);
            let encoded_offset =
                leading_offset + base.len() + 1 + (encoded_raw.len() - encoded_start.len());
            let encoded = encoded_start.trim_end_matches([' ', '\t']);
            let Some((name, values)) = encoded.split_once('=') else {
                return Err(media_parse_error(
                    encoded_offset,
                    "expected encodings= after media base",
                ));
            };
            if !name
                .trim_matches([' ', '\t'])
                .eq_ignore_ascii_case("encodings")
            {
                return Err(media_parse_error(
                    encoded_offset,
                    "only the encodings media attribute is supported",
                ));
            }
            let values_offset = encoded_offset + name.len() + 1;
            if values.is_empty() {
                return Err(media_parse_error(
                    values_offset,
                    "encodings list must not be empty",
                ));
            }
            let base = MimeType::from_str(base.trim_matches([' ', '\t']))
                .map_err(|error| shift_parse_error(error, leading_offset))?;
            let mut encodings = Vec::new();
            let mut token_offset = 0;
            for token in values.split(',') {
                let encoding = MimeType::from_str(token)
                    .map_err(|error| shift_parse_error(error, values_offset + token_offset))?;
                if !encoding.is_encoding() {
                    return Err(media_parse_error(
                        values_offset + token_offset,
                        "expected a MIME type whose is_encoding predicate is true",
                    ));
                }
                encodings.push(encoding);
                token_offset += token.len() + 1;
            }
            return Self::from_parts(base, encodings);
        }

        let mime = MimeType::from_str(value);
        if looks_like_path(value, mime.as_ref().ok()) {
            return Ok(Self::from_file_name(value));
        }
        match mime {
            Ok(mime) => Ok(Self::new(mime)),
            Err(_) if !value.contains('/') => Ok(Self::from_file_name(value)),
            Err(error) => Err(shift_parse_error(error, leading_offset)),
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.base.fmt(formatter)?;
        if self.is_encoded() {
            formatter.write_str(";encodings=")?;
            for (index, encoding) in self.encodings().iter().enumerate() {
                if index > 0 {
                    formatter.write_str(",")?;
                }
                encoding.fmt(formatter)?;
            }
        }
        Ok(())
    }
}

impl Serialize for MediaType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MediaType", 2)?;
        state.serialize_field("base", &self.base)?;
        state.serialize_field("encodings", self.encodings())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for MediaType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Representation {
            base: MimeType,
            #[serde(default)]
            encodings: Vec<MimeType>,
        }

        let value = Representation::deserialize(deserializer)?;
        Self::from_parts(value.base, value.encodings).map_err(serde::de::Error::custom)
    }
}

impl<'a> IntoIterator for &'a MediaType {
    type Item = &'a MimeType;
    type IntoIter = std::slice::Iter<'a, MimeType>;

    fn into_iter(self) -> Self::IntoIter {
        self.encodings().iter()
    }
}

fn validate_encodings<I>(encodings: I) -> Result<Option<Arc<Vec<MimeType>>>>
where
    I: IntoIterator<Item = MimeType>,
{
    let mut values = Vec::new();
    for (index, encoding) in encodings.into_iter().enumerate() {
        if !encoding.is_encoding() {
            return Err(invalid_encoding(index));
        }
        values.push(encoding);
    }
    Ok((!values.is_empty()).then(|| Arc::new(values)))
}

fn invalid_encoding(position: usize) -> Error {
    Error::Parse {
        target: "media type encoding",
        position,
        reason: "expected a MIME type whose is_encoding predicate is true".into(),
    }
}

fn media_parse_error(position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target: "media type",
        position,
        reason: reason.into(),
    }
}

fn shift_parse_error(error: Error, offset: usize) -> Error {
    match error {
        Error::Parse {
            target,
            position,
            reason,
        } => Error::Parse {
            target,
            position: position + offset,
            reason,
        },
        error => error,
    }
}

fn looks_like_path(value: &str, parsed_mime: Option<&MimeType>) -> bool {
    if value.contains('\\') || value.contains("://") || value.starts_with(['/', '.']) {
        return true;
    }
    let Some((_, file_name)) = value.rsplit_once('/') else {
        return value.contains('.');
    };
    if !file_name.contains('.') {
        return false;
    }

    // A relative path with one slash can also satisfy MIME's open syntactic
    // `type/subtype` grammar. Preserve registered top-level media spellings,
    // while treating an unregistered leading path segment as the resource path
    // promised by `MediaType::from_str`. Fully ambiguous values can still use
    // `MimeType::from_str` or `MediaType::from_path` explicitly.
    parsed_mime.is_none_or(|mime| {
        !(mime.is_application()
            || mime.is_audio()
            || mime.is_font()
            || mime.is_haptics()
            || mime.is_image()
            || mime.is_message()
            || mime.is_model()
            || mime.is_multipart()
            || mime.is_text()
            || mime.is_video()
            || mime.top_level() == "example")
    })
}

fn compound_extension(value: &str) -> Option<(MimeType, MimeType)> {
    let value = value.trim_matches([' ', '\t']);
    let value = value.strip_prefix('.').unwrap_or(value);
    if value.eq_ignore_ascii_case("tgz") {
        Some((MimeType::TAR, MimeType::GZIP))
    } else if value.eq_ignore_ascii_case("tbz") || value.eq_ignore_ascii_case("tbz2") {
        Some((MimeType::TAR, MimeType::BZIP2))
    } else if value.eq_ignore_ascii_case("txz") {
        Some((MimeType::TAR, MimeType::XZ))
    } else if value.eq_ignore_ascii_case("tzst") {
        Some((MimeType::TAR, MimeType::ZSTD))
    } else if value.eq_ignore_ascii_case("svgz") {
        Some((MimeType::SVG, MimeType::GZIP))
    } else {
        None
    }
}
