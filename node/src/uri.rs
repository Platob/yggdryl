//! Node.js views of the native URI, URL, URN, and ARN domain.

use napi::bindgen_prelude::{ClassInstance, Either5, Error, Result};
use napi_derive::napi;
use yggdryl::{Arn as CoreArn, Uri as CoreUri, Url as CoreUrl, Urn as CoreUrn};

use crate::enums::{
    JsMediaType, JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input,
    mime_type_from_input,
};
use crate::{napi_error, ordering_value};

fn path_string_from_core(value: std::path::PathBuf) -> Result<String> {
    value.into_os_string().into_string().map_err(|_| {
        Error::from_reason("file URI path cannot be represented as a JavaScript string")
    })
}

/// One Hive partition column and the value a path segment gives it.
#[napi(object)]
pub struct PartitionEntry {
    /// The partition column named by the directory.
    pub column: String,
    /// The value that directory assigns it.
    pub value: String,
}

/// Project the core's partition pairs as the object JavaScript reads.
pub(crate) fn partition_entries(pairs: Vec<(String, String)>) -> Vec<PartitionEntry> {
    pairs
        .into_iter()
        .map(|(column, value)| PartitionEntry { column, value })
        .collect()
}

fn uri_from_input(
    value: Either5<
        ClassInstance<'_, JsUri>,
        ClassInstance<'_, JsUrl>,
        ClassInstance<'_, JsUrn>,
        ClassInstance<'_, JsArn>,
        String,
    >,
) -> Result<CoreUri> {
    match value {
        Either5::A(value) => Ok(value.inner.clone()),
        Either5::B(value) => Ok(value.inner.clone().into_uri()),
        Either5::C(value) => Ok(value.inner.clone().into_uri()),
        Either5::D(value) => Ok(value.inner.clone().into_uri()),
        Either5::E(value) => CoreUri::from_str(&value).map_err(napi_error),
    }
}

/// A normalized URI backed by the validated Rust core.
#[napi(js_name = "Uri")]
pub struct JsUri {
    pub(crate) inner: CoreUri,
}

impl Clone for JsUri {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsUri {
    fn from_core(inner: CoreUri) -> Self {
        Self { inner }
    }

    fn path_segment_at(&self, index: usize) -> Option<String> {
        self.inner.path_segments().nth(index).map(ToOwned::to_owned)
    }
}

#[napi]
impl JsUri {
    /// Parse a URI expression or cheaply clone another native `Uri`.
    #[napi(constructor)]
    pub fn new(
        value: Either5<
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        uri_from_input(value).map(Self::from_core)
    }

    /// Infer a URI from a native wrapper or URI-expression string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(
        value: Either5<
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        uri_from_input(value).map(Self::from_core)
    }

    /// Parse and normalize an absolute URI.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreUri::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Convert a local, Windows-drive, or UNC path into a normalized file URI.
    #[napi(factory)]
    pub fn from_path(value: String) -> Result<Self> {
        CoreUri::from_path(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Validate and project this general URI as a hierarchical URL.
    #[napi]
    pub fn into_url(&self) -> Result<JsUrl> {
        self.inner
            .clone()
            .into_url()
            .map(JsUrl::from_core)
            .map_err(napi_error)
    }

    /// Validate and project this general URI as a URN.
    #[napi]
    pub fn into_urn(&self) -> Result<JsUrn> {
        self.inner
            .clone()
            .into_urn()
            .map(JsUrn::from_core)
            .map_err(napi_error)
    }

    /// Validate and project this general URI as an ARN.
    #[napi]
    pub fn into_arn(&self) -> Result<JsArn> {
        self.inner
            .clone()
            .into_arn()
            .map(JsArn::from_core)
            .map_err(napi_error)
    }

    /// The location this identifier names, as a `Url`.
    ///
    /// A location locates itself; a name resolves to where it is - a URN under
    /// the process working directory, an Amazon S3 ARN to the `s3:` URL its
    /// bucket and key address - which is what lets any identifier be handed to
    /// a reader as the thing to open.
    #[napi]
    pub fn locator(&self) -> Result<JsUrl> {
        self.inner
            .locator()
            .map(JsUrl::from_core)
            .map_err(napi_error)
    }

    /// Decode this file URI as a host-independent forward-slash path.
    #[napi]
    pub fn into_path(&self) -> Result<String> {
        self.inner
            .clone()
            .into_path()
            .map_err(napi_error)
            .and_then(path_string_from_core)
    }

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Normalized, non-empty URI scheme.
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().as_str().to_owned()
    }

    /// Normalized URI authority, empty when the URI has no authority.
    #[napi(getter)]
    pub fn authority(&self) -> String {
        self.inner.authority().as_str().to_owned()
    }

    /// User name from URI user information.
    #[napi(getter)]
    pub fn user(&self) -> Option<String> {
        self.inner.user().map(ToOwned::to_owned)
    }

    /// Password after the first user-information colon.
    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(ToOwned::to_owned)
    }

    /// Network hostname under the core store-aware rules.
    #[napi(getter)]
    pub fn hostname(&self) -> Option<String> {
        self.inner.hostname().map(ToOwned::to_owned)
    }

    /// Container name - a bucket on S3 and Google, a container on Azure - or
    /// `null` for a scheme that names no store.
    #[napi(getter)]
    pub fn bucket(&self) -> Option<String> {
        self.inner.bucket().map(ToOwned::to_owned)
    }

    /// Azure storage account this location names, when it names one.
    #[napi(getter)]
    pub fn account(&self) -> Option<String> {
        self.inner.account().map(ToOwned::to_owned)
    }

    /// Object key: the path below the container, as the path spells it.
    #[napi(getter)]
    pub fn key(&self) -> Option<String> {
        self.inner.key().map(ToOwned::to_owned)
    }

    /// Region inferred from a recognized store hostname.
    #[napi(getter)]
    pub fn region(&self) -> Option<String> {
        self.inner.region().map(ToOwned::to_owned)
    }

    /// Store endpoint host with its explicit port, without a virtual container.
    #[napi(getter)]
    pub fn store_endpoint(&self) -> Option<String> {
        self.inner.store_endpoint().map(ToOwned::to_owned)
    }

    /// Whether this location writes its container into the hostname.
    #[napi]
    pub fn is_virtual_hosted(&self) -> bool {
        self.inner.is_virtual_hosted()
    }

    /// Normalized slash-separated URI path.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().as_str().to_owned()
    }

    /// Query text without the leading question mark.
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner
            .query(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// Fragment text without the leading hash mark.
    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner
            .fragment(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// Last non-empty path segment.
    #[napi(getter)]
    pub fn file_name(&self) -> Option<String> {
        self.inner.file_name().map(ToOwned::to_owned)
    }

    /// Filename without its final non-empty extension.
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(ToOwned::to_owned)
    }

    /// Final filename extension without a leading dot.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// All filename extensions in lexical order from left to right.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().map(ToOwned::to_owned).collect()
    }

    /// Replace the final filename segment or append one to a directory path.
    #[napi]
    pub fn set_file_name(&mut self, value: String) -> Result<()> {
        self.inner.set_file_name(&value).map_err(napi_error)
    }

    /// Replace the filename stem while preserving its final extension.
    #[napi]
    pub fn set_stem(&mut self, value: String) -> Result<()> {
        self.inner.set_stem(&value).map_err(napi_error)
    }

    /// Add or replace the final filename extension.
    #[napi]
    pub fn set_extension(&mut self, value: String) -> Result<()> {
        self.inner.set_extension(&value).map_err(napi_error)
    }

    /// Replace the complete compound filename extension chain.
    #[napi]
    pub fn set_extensions(&mut self, values: Vec<String>) -> Result<()> {
        self.inner.set_extensions(values).map_err(napi_error)
    }

    /// Remove the final filename extension.
    #[napi]
    pub fn remove_extension(&mut self) -> bool {
        self.inner.remove_extension()
    }

    /// Remove every filename extension.
    #[napi]
    pub fn clear_extensions(&mut self) -> bool {
        self.inner.clear_extensions()
    }

    /// MIME type inferred from the final suffix.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.mime_type())
    }

    /// Base MIME type and transparent encodings inferred from all suffixes.
    #[napi(getter)]
    pub fn media_type(&self) -> JsMediaType {
        JsMediaType::from_core(self.inner.media_type())
    }

    /// Replace the final suffix using a MIME preferred extension.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .set_mime_type(mime_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Replace the full suffix chain using a media type.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Non-empty path segments in path order.
    #[napi(getter)]
    pub fn path_segments(&self) -> Vec<String> {
        self.inner.path_segments().map(ToOwned::to_owned).collect()
    }

    /// Join path components through the generic URI core.
    ///
    /// JavaScript has no `/` operator protocol for objects, so `joinPath` is
    /// the explicit spelling of Rust and Python's path-composition idiom.
    #[napi]
    pub fn join_path(&self, others: Vec<String>) -> Result<Self> {
        let mut joined = self.inner.clone();
        for other in &others {
            joined = joined.joinpath(other).map_err(napi_error)?;
        }
        Ok(Self::from_core(joined))
    }

    /// Number of path segments exposed through iteration.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.path_segments().count()).unwrap_or(u32::MAX)
    }

    /// Return a path segment at an Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<String> {
        let len = i64::from(self.length());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved)
            .ok()
            .and_then(|index| self.path_segment_at(index))
    }

    /// Exact normalized native equality.
    #[napi]
    pub fn equals(&self, other: &JsUri) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsUri) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of the canonical URI.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}

/// Read one location a caller named: a URL, a name, or text naming either.
///
/// An identifier crosses through `locator`, so a name opens as well as a
/// location does: a URN resolves to the path it spells and an Amazon S3 ARN to
/// the `s3:` URL it addresses. Text carrying a scheme is a URL and text
/// carrying none is a path rooted at the working directory.
pub(crate) fn url_from_input(
    value: Either5<
        ClassInstance<'_, JsUrl>,
        ClassInstance<'_, JsUri>,
        ClassInstance<'_, JsUrn>,
        ClassInstance<'_, JsArn>,
        String,
    >,
) -> Result<CoreUrl> {
    match value {
        Either5::A(value) => Ok(value.inner.clone()),
        Either5::B(value) => value.inner.locator().map_err(napi_error),
        Either5::C(value) => value.inner.locator().map_err(napi_error),
        Either5::D(value) => value.inner.locator().map_err(napi_error),
        Either5::E(value) => CoreUrl::from_location(&value).map_err(napi_error),
    }
}

/// A normalized hierarchical URL backed by the validated Rust core.
#[napi(js_name = "Url")]
pub struct JsUrl {
    pub(crate) inner: CoreUrl,
}

impl Clone for JsUrl {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsUrl {
    pub(crate) fn from_core(inner: CoreUrl) -> Self {
        Self { inner }
    }

    fn path_segment_at(&self, index: usize) -> Option<String> {
        self.inner.path_segments().nth(index).map(ToOwned::to_owned)
    }
}

#[napi]
impl JsUrl {
    /// Parse/convert a `Url`, any identifier naming one, or location text.
    #[napi(constructor)]
    pub fn new(
        value: Either5<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        url_from_input(value).map(Self::from_core)
    }

    /// Infer a URL from any native identifier wrapper or location text.
    #[napi(factory, js_name = "from")]
    pub fn from_js(
        value: Either5<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        url_from_input(value).map(Self::from_core)
    }

    /// Parse and validate a hierarchical URL.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreUrl::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Convert a local, Windows-drive, or UNC path into a normalized file URL.
    #[napi(factory)]
    pub fn from_path(value: String) -> Result<Self> {
        CoreUrl::from_path(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Decode this file URL as a host-independent forward-slash path.
    #[napi]
    pub fn into_path(&self) -> Result<String> {
        self.inner
            .clone()
            .into_path()
            .map_err(napi_error)
            .and_then(path_string_from_core)
    }

    /// Validate and convert a native `Uri` into a URL.
    ///
    /// This is the strict door: it answers the URL a URI already is, and
    /// refuses a name rather than resolving it the way the constructor does.
    #[napi(factory)]
    pub fn from_uri(value: &JsUri) -> Result<Self> {
        CoreUrl::from_uri(value.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Normalized, non-empty URL scheme.
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().as_str().to_owned()
    }

    /// Normalized URL authority, empty for a local file URL.
    #[napi(getter)]
    pub fn authority(&self) -> String {
        self.inner.authority().as_str().to_owned()
    }

    /// User name from URL user information.
    #[napi(getter)]
    pub fn user(&self) -> Option<String> {
        self.inner.user().map(ToOwned::to_owned)
    }

    /// Password after the first user-information colon.
    #[napi(getter)]
    pub fn password(&self) -> Option<String> {
        self.inner.password().map(ToOwned::to_owned)
    }

    /// Network hostname under the core store-aware rules.
    #[napi(getter)]
    pub fn hostname(&self) -> Option<String> {
        self.inner.hostname().map(ToOwned::to_owned)
    }

    /// Container name - a bucket on S3 and Google, a container on Azure - or
    /// `null` for a scheme that names no store.
    #[napi(getter)]
    pub fn bucket(&self) -> Option<String> {
        self.inner.bucket().map(ToOwned::to_owned)
    }

    /// Azure storage account this location names, when it names one.
    #[napi(getter)]
    pub fn account(&self) -> Option<String> {
        self.inner.account().map(ToOwned::to_owned)
    }

    /// Object key: the path below the container, as the path spells it.
    #[napi(getter)]
    pub fn key(&self) -> Option<String> {
        self.inner.key().map(ToOwned::to_owned)
    }

    /// Region inferred from a recognized store hostname.
    #[napi(getter)]
    pub fn region(&self) -> Option<String> {
        self.inner.region().map(ToOwned::to_owned)
    }

    /// Store endpoint host with its explicit port, without a virtual container.
    #[napi(getter)]
    pub fn store_endpoint(&self) -> Option<String> {
        self.inner.store_endpoint().map(ToOwned::to_owned)
    }

    /// Whether this location writes its container into the hostname.
    #[napi]
    pub fn is_virtual_hosted(&self) -> bool {
        self.inner.is_virtual_hosted()
    }

    /// Normalized slash-separated URL path.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().as_str().to_owned()
    }

    /// Query text without the leading question mark.
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner
            .query(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// Fragment text without the leading hash mark.
    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner
            .fragment(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// Last non-empty path segment.
    #[napi(getter)]
    pub fn file_name(&self) -> Option<String> {
        self.inner.file_name().map(ToOwned::to_owned)
    }

    /// Filename without its final non-empty extension.
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(ToOwned::to_owned)
    }

    /// Final filename extension without a leading dot.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// All filename extensions in lexical order from left to right.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().map(ToOwned::to_owned).collect()
    }

    /// Replace the final filename segment or append one to a directory path.
    #[napi]
    pub fn set_file_name(&mut self, value: String) -> Result<()> {
        self.inner.set_file_name(&value).map_err(napi_error)
    }

    /// Replace the filename stem while preserving its final extension.
    #[napi]
    pub fn set_stem(&mut self, value: String) -> Result<()> {
        self.inner.set_stem(&value).map_err(napi_error)
    }

    /// Add or replace the final filename extension.
    #[napi]
    pub fn set_extension(&mut self, value: String) -> Result<()> {
        self.inner.set_extension(&value).map_err(napi_error)
    }

    /// Replace the complete compound filename extension chain.
    #[napi]
    pub fn set_extensions(&mut self, values: Vec<String>) -> Result<()> {
        self.inner.set_extensions(values).map_err(napi_error)
    }

    /// Remove the final filename extension.
    #[napi]
    pub fn remove_extension(&mut self) -> bool {
        self.inner.remove_extension()
    }

    /// Remove every filename extension.
    #[napi]
    pub fn clear_extensions(&mut self) -> bool {
        self.inner.clear_extensions()
    }

    /// MIME type inferred from the final suffix.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.mime_type())
    }

    /// Base MIME type and transparent encodings inferred from all suffixes.
    #[napi(getter)]
    pub fn media_type(&self) -> JsMediaType {
        JsMediaType::from_core(self.inner.media_type())
    }

    /// Replace the final suffix using a MIME preferred extension.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .set_mime_type(mime_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Replace the full suffix chain using a media type.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Non-empty path segments in path order.
    #[napi(getter)]
    pub fn path_segments(&self) -> Vec<String> {
        self.inner.path_segments().map(ToOwned::to_owned).collect()
    }

    /// Number of path segments exposed through iteration.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.path_segments().count()).unwrap_or(u32::MAX)
    }

    /// Return a path segment at an Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<String> {
        let len = i64::from(self.length());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved)
            .ok()
            .and_then(|index| self.path_segment_at(index))
    }

    /// The location this identifier names, as a `Url`.
    ///
    /// A location locates itself, so this answers an equal `Url`. It is here
    /// because `locator` is the one door every identifier answers - a caller
    /// holding a `Uri`, a `Urn`, an `Arn`, or a `Url` asks the same question
    /// of each and is handed the thing to open.
    #[napi]
    pub fn locator(&self) -> Result<JsUrl> {
        self.inner
            .clone()
            .into_uri()
            .locator()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Convert this URL to its general URI representation.
    #[napi]
    pub fn into_uri(&self) -> JsUri {
        JsUri::from_core(self.inner.clone().into_uri())
    }

    // ---------------------------------------------------------------------
    // Path compatibility.
    //
    // A URL is a path with a scheme, so it answers the same questions
    // `node:path` answers, under the names JavaScript already uses for them.
    // Code written against a local directory runs against a location in any
    // backend, and every answer comes from the core implementation rather than
    // from a second one written in JavaScript.
    // ---------------------------------------------------------------------

    /// The final path component, as `path.basename`.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.file_name().unwrap_or_default().to_owned()
    }

    /// The final extension with its leading dot, as `path.extname`.
    #[napi(getter)]
    pub fn suffix(&self) -> String {
        self.inner
            .extension()
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default()
    }

    /// Every extension with leading dots, outermost last.
    #[napi(getter)]
    pub fn suffixes(&self) -> Vec<String> {
        self.inner
            .extensions()
            .map(|extension| format!(".{extension}"))
            .collect()
    }

    /// The path components, as `Url.pathSegments` under the path name.
    #[napi(getter)]
    pub fn parts(&self) -> Vec<String> {
        self.path_segments()
    }

    /// The containing location, as `path.dirname`.
    ///
    /// A location at the root is its own parent, which is what `dirname` does.
    #[napi(getter)]
    pub fn parent(&self) -> JsUrl {
        Self::from_core(self.inner.parent().unwrap_or_else(|| self.inner.clone()))
    }

    /// Every containing location, closest first.
    #[napi(getter)]
    pub fn parents(&self) -> Vec<JsUrl> {
        self.inner.parents().map(Self::from_core).collect()
    }

    /// Join path components onto this location, as `path.join`.
    #[napi]
    pub fn joinpath(&self, others: Vec<String>) -> Result<Self> {
        let mut joined = self.inner.clone();
        for other in &others {
            joined = joined.joinpath(other).map_err(napi_error)?;
        }
        Ok(Self::from_core(joined))
    }

    /// This location with a different final component.
    #[napi]
    pub fn with_name(&self, value: String) -> Result<Self> {
        let mut renamed = self.inner.clone();
        renamed.set_file_name(&value).map_err(napi_error)?;
        Ok(Self::from_core(renamed))
    }

    /// This location with a different stem.
    #[napi]
    pub fn with_stem(&self, value: String) -> Result<Self> {
        let mut renamed = self.inner.clone();
        renamed.set_stem(&value).map_err(napi_error)?;
        Ok(Self::from_core(renamed))
    }

    /// This location with a different final extension.
    ///
    /// The leading dot is optional, and an empty suffix removes the extension.
    #[napi]
    pub fn with_suffix(&self, value: String) -> Result<Self> {
        let mut renamed = self.inner.clone();
        let suffix = value.strip_prefix('.').unwrap_or(&value);
        if suffix.is_empty() {
            renamed.remove_extension();
        } else {
            renamed.set_extension(suffix).map_err(napi_error)?;
        }
        Ok(Self::from_core(renamed))
    }

    /// A URL path is always absolute, as `path.isAbsolute` reports it.
    #[napi]
    #[allow(clippy::unused_self)]
    pub fn is_absolute(&self) -> bool {
        true
    }

    /// The path in POSIX form, as `path.posix` spells it.
    #[napi]
    pub fn as_posix(&self) -> String {
        self.inner.path().as_str().to_owned()
    }

    /// The whole location as text, as `url.href`.
    #[napi]
    pub fn as_uri(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether this location matches `pattern`.
    ///
    /// A pattern with no separator matches the name at any depth; one with a
    /// separator is anchored at the path root.
    #[napi(js_name = "match")]
    pub fn matches(&self, pattern: String) -> bool {
        self.inner.matches_glob(&pattern)
    }

    /// Return whether the whole path matches `pattern`.
    #[napi]
    pub fn full_match(&self, pattern: String) -> bool {
        self.inner.matches_glob(&pattern)
    }

    /// Return whether this location is a glob pattern rather than one name.
    #[napi]
    pub fn is_glob(&self) -> bool {
        self.inner.is_glob()
    }

    /// Return this location relative to `other`.
    ///
    /// Throws when this location is not below `other`, because an unrelated
    /// pair has no relative spelling.
    #[napi]
    pub fn relative_to(
        &self,
        other: Either5<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<String> {
        let root = url_from_input(other)?;
        self.inner
            .segments_under(&root)
            .map(|segments| segments.join("/"))
            .ok_or_else(|| napi_error(format!("{} is not in the subpath of {root}", self.inner)))
    }

    /// Return whether this location is below `other`.
    #[napi]
    pub fn is_relative_to(
        &self,
        other: Either5<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<bool> {
        Ok(self.inner.segments_under(&url_from_input(other)?).is_some())
    }

    /// Return whether something exists here now, as `fs.existsSync`.
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// Return whether this location is a directory, as `Stats.isDirectory`.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Return whether this location is a regular file, as `Stats.isFile`.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Return whether the name begins with a dot, so a listing may skip it.
    #[napi]
    pub fn is_private(&self) -> bool {
        self.inner.is_private()
    }

    /// The Hive partition pairs this location's path spells out.
    #[napi(getter)]
    pub fn partitions(&self) -> Vec<PartitionEntry> {
        partition_entries(self.inner.hive_partitions())
    }

    /// The value of one Hive partition column, when the path has it.
    #[napi]
    pub fn partition(&self, column: String) -> Option<String> {
        self.inner.hive_partition(&column)
    }

    /// Exact normalized native equality.
    #[napi]
    pub fn equals(&self, other: &JsUrl) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsUrl) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of the canonical URL.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}

fn urn_from_input(
    value: Either5<
        ClassInstance<'_, JsUrn>,
        ClassInstance<'_, JsUri>,
        ClassInstance<'_, JsUrl>,
        ClassInstance<'_, JsArn>,
        String,
    >,
) -> Result<CoreUrn> {
    match value {
        Either5::A(value) => Ok(value.inner.clone()),
        Either5::B(value) => CoreUrn::from_uri(value.inner.clone()).map_err(napi_error),
        Either5::C(value) => CoreUrn::from_uri(value.inner.clone().into_uri()).map_err(napi_error),
        Either5::D(value) => CoreUrn::from_uri(value.inner.clone().into_uri()).map_err(napi_error),
        Either5::E(value) => CoreUrn::from_str(&value).map_err(napi_error),
    }
}

/// A normalized URN backed by the validated Rust core.
#[napi(js_name = "Urn")]
pub struct JsUrn {
    pub(crate) inner: CoreUrn,
}

impl Clone for JsUrn {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsUrn {
    fn from_core(inner: CoreUrn) -> Self {
        Self { inner }
    }

    fn path_segment_at(&self, index: usize) -> Option<String> {
        self.inner.path_segments().nth(index).map(ToOwned::to_owned)
    }
}

#[napi]
impl JsUrn {
    /// Parse/convert a `Urn`, `Uri`, or URN string.
    #[napi(constructor)]
    pub fn new(
        value: Either5<
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        urn_from_input(value).map(Self::from_core)
    }

    /// Infer a URN from a native URN/URI wrapper or string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(
        value: Either5<
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<Self> {
        urn_from_input(value).map(Self::from_core)
    }

    /// Parse and validate a URN.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreUrn::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Validate and convert a native `Uri` into a URN.
    #[napi(factory)]
    pub fn from_uri(value: &JsUri) -> Result<Self> {
        CoreUrn::from_uri(value.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The fixed lowercase `urn` scheme.
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().as_str().to_owned()
    }

    /// URN authority, which is canonically empty.
    #[napi(getter)]
    pub fn authority(&self) -> String {
        self.inner.authority().as_str().to_owned()
    }

    /// Normalized URN path containing its namespace and namespace-specific string.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().as_str().to_owned()
    }

    /// URN query text without the leading question mark.
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner
            .query(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// URN fragment text without the leading hash mark.
    #[napi(getter)]
    pub fn fragment(&self) -> Option<String> {
        self.inner
            .fragment(false)
            .unwrap_or_default()
            .map(std::borrow::Cow::into_owned)
    }

    /// Last non-empty path segment.
    #[napi(getter)]
    pub fn file_name(&self) -> Option<String> {
        self.inner.file_name().map(ToOwned::to_owned)
    }

    /// Namespace-specific filename without its final non-empty extension.
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(ToOwned::to_owned)
    }

    /// Final filename-style extension without a leading dot.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// All filename-style extensions in lexical order from left to right.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().map(ToOwned::to_owned).collect()
    }

    /// Replace the final namespace-specific filename segment.
    #[napi]
    pub fn set_file_name(&mut self, value: String) -> Result<()> {
        self.inner.set_file_name(&value).map_err(napi_error)
    }

    /// Replace the namespace-specific filename stem.
    #[napi]
    pub fn set_stem(&mut self, value: String) -> Result<()> {
        self.inner.set_stem(&value).map_err(napi_error)
    }

    /// Add or replace the final namespace-specific filename extension.
    #[napi]
    pub fn set_extension(&mut self, value: String) -> Result<()> {
        self.inner.set_extension(&value).map_err(napi_error)
    }

    /// Replace the complete namespace-specific filename extension chain.
    #[napi]
    pub fn set_extensions(&mut self, values: Vec<String>) -> Result<()> {
        self.inner.set_extensions(values).map_err(napi_error)
    }

    /// Remove the final namespace-specific filename extension.
    #[napi]
    pub fn remove_extension(&mut self) -> bool {
        self.inner.remove_extension()
    }

    /// Remove every namespace-specific filename extension.
    #[napi]
    pub fn clear_extensions(&mut self) -> bool {
        self.inner.clear_extensions()
    }

    /// MIME type inferred from the final suffix.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.mime_type())
    }

    /// Base MIME type and transparent encodings inferred from all suffixes.
    #[napi(getter)]
    pub fn media_type(&self) -> JsMediaType {
        JsMediaType::from_core(self.inner.media_type())
    }

    /// Replace the final suffix using a MIME preferred extension.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .set_mime_type(mime_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Replace the full suffix chain using a media type.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Non-empty path segments in path order.
    #[napi(getter)]
    pub fn path_segments(&self) -> Vec<String> {
        self.inner.path_segments().map(ToOwned::to_owned).collect()
    }

    /// Number of path segments exposed through iteration.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.path_segments().count()).unwrap_or(u32::MAX)
    }

    /// Return a path segment at an Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<String> {
        let len = i64::from(self.length());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved)
            .ok()
            .and_then(|index| self.path_segment_at(index))
    }

    /// Normalized URN namespace identifier.
    #[napi(getter)]
    pub fn namespace(&self) -> String {
        self.inner.namespace().to_owned()
    }

    /// Namespace-specific string.
    #[napi(getter)]
    pub fn namespace_specific(&self) -> String {
        self.inner.namespace_specific().to_owned()
    }

    /// The relative path this name spells.
    ///
    /// The namespace leads it and the namespace-specific string's `:`
    /// separators are the ones after it, so `urn:lake:trades:2026:part.parquet`
    /// spells `lake/trades/2026/part.parquet`.
    #[napi]
    pub fn locator_path(&self) -> Result<String> {
        self.inner
            .locator_path()
            .map(|path| path.as_str().to_owned())
            .map_err(napi_error)
    }

    /// Resolve this name under `base`, answering where it is.
    ///
    /// The path `locatorPath` spells is joined onto `base`, so one base turns a
    /// whole namespace of names into locations without either being rewritten.
    #[napi]
    pub fn resolve(
        &self,
        base: Either5<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            ClassInstance<'_, JsArn>,
            String,
        >,
    ) -> Result<JsUrl> {
        self.inner
            .resolve(&url_from_input(base)?)
            .map(JsUrl::from_core)
            .map_err(napi_error)
    }

    /// Resolve this name under the process working directory.
    ///
    /// This is what makes a name openable with no base named: the path the name
    /// spells is relative, and the working directory is the root every relative
    /// path is read against.
    #[napi]
    pub fn locator(&self) -> Result<JsUrl> {
        self.inner
            .locator()
            .map(JsUrl::from_core)
            .map_err(napi_error)
    }

    /// Convert this URN to its general URI representation.
    #[napi]
    pub fn into_uri(&self) -> JsUri {
        JsUri::from_core(self.inner.clone().into_uri())
    }

    /// Exact normalized native equality.
    #[napi]
    pub fn equals(&self, other: &JsUrn) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsUrn) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of the canonical URN.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}

fn arn_from_input(
    value: Either5<
        ClassInstance<'_, JsArn>,
        ClassInstance<'_, JsUri>,
        ClassInstance<'_, JsUrl>,
        ClassInstance<'_, JsUrn>,
        String,
    >,
) -> Result<CoreArn> {
    match value {
        Either5::A(value) => Ok(value.inner.clone()),
        Either5::B(value) => CoreArn::from_uri(value.inner.clone()).map_err(napi_error),
        Either5::C(value) => CoreArn::from_uri(value.inner.clone().into_uri()).map_err(napi_error),
        Either5::D(value) => CoreArn::from_uri(value.inner.clone().into_uri()).map_err(napi_error),
        Either5::E(value) => CoreArn::from_str(&value).map_err(napi_error),
    }
}

/// A normalized AWS ARN backed by the validated Rust core.
#[napi(js_name = "Arn")]
pub struct JsArn {
    pub(crate) inner: CoreArn,
}

impl Clone for JsArn {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsArn {
    fn from_core(inner: CoreArn) -> Self {
        Self { inner }
    }

    fn path_segment_at(&self, index: usize) -> Option<String> {
        self.inner.path_segments().nth(index).map(ToOwned::to_owned)
    }
}

#[napi]
impl JsArn {
    /// Parse/convert an `Arn`, `Uri`, or ARN string.
    #[napi(constructor)]
    pub fn new(
        value: Either5<
            ClassInstance<'_, JsArn>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUrn>,
            String,
        >,
    ) -> Result<Self> {
        arn_from_input(value).map(Self::from_core)
    }

    /// Infer an ARN from a native ARN/URI wrapper or string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(
        value: Either5<
            ClassInstance<'_, JsArn>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUrn>,
            String,
        >,
    ) -> Result<Self> {
        arn_from_input(value).map(Self::from_core)
    }

    /// Parse and validate an ARN.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreArn::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Build a validated ARN from its five fields.
    ///
    /// `region` and `account` are written as the empty string when the service
    /// names neither.
    #[napi(factory)]
    pub fn from_parts(
        partition: String,
        service: String,
        region: Option<String>,
        account: Option<String>,
        resource: Option<String>,
    ) -> Result<Self> {
        CoreArn::from_parts(
            &partition,
            &service,
            region.as_deref().unwrap_or_default(),
            account.as_deref().unwrap_or_default(),
            resource.as_deref().unwrap_or_default(),
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Validate and convert a native `Uri` into an ARN.
    #[napi(factory)]
    pub fn from_uri(value: &JsUri) -> Result<Self> {
        CoreArn::from_uri(value.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The fixed lowercase `arn` scheme.
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.inner.scheme().as_str().to_owned()
    }

    /// ARN authority, which is canonically empty.
    #[napi(getter)]
    pub fn authority(&self) -> String {
        self.inner.authority().as_str().to_owned()
    }

    /// Normalized ARN path containing its five fields.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.path().as_str().to_owned()
    }

    /// Last non-empty resource segment.
    #[napi(getter)]
    pub fn file_name(&self) -> Option<String> {
        self.inner.file_name().map(ToOwned::to_owned)
    }

    /// Resource filename without its final non-empty extension.
    #[napi(getter)]
    pub fn stem(&self) -> Option<String> {
        self.inner.stem().map(ToOwned::to_owned)
    }

    /// Final filename-style extension without a leading dot.
    #[napi(getter)]
    pub fn extension(&self) -> Option<String> {
        self.inner.extension().map(ToOwned::to_owned)
    }

    /// All filename-style extensions in lexical order from left to right.
    #[napi(getter)]
    pub fn extensions(&self) -> Vec<String> {
        self.inner.extensions().map(ToOwned::to_owned).collect()
    }

    /// Replace the final resource filename segment.
    #[napi]
    pub fn set_file_name(&mut self, value: String) -> Result<()> {
        self.inner.set_file_name(&value).map_err(napi_error)
    }

    /// Replace the resource filename stem.
    #[napi]
    pub fn set_stem(&mut self, value: String) -> Result<()> {
        self.inner.set_stem(&value).map_err(napi_error)
    }

    /// Add or replace the final resource filename extension.
    #[napi]
    pub fn set_extension(&mut self, value: String) -> Result<()> {
        self.inner.set_extension(&value).map_err(napi_error)
    }

    /// Replace the complete resource filename extension chain.
    #[napi]
    pub fn set_extensions(&mut self, values: Vec<String>) -> Result<()> {
        self.inner.set_extensions(values).map_err(napi_error)
    }

    /// Remove the final resource filename extension.
    #[napi]
    pub fn remove_extension(&mut self) -> bool {
        self.inner.remove_extension()
    }

    /// Remove every resource filename extension.
    #[napi]
    pub fn clear_extensions(&mut self) -> bool {
        self.inner.clear_extensions()
    }

    /// MIME type inferred from the final suffix.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.mime_type())
    }

    /// Base MIME type and transparent encodings inferred from all suffixes.
    #[napi(getter)]
    pub fn media_type(&self) -> JsMediaType {
        JsMediaType::from_core(self.inner.media_type())
    }

    /// Replace the final suffix using a MIME preferred extension.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .set_mime_type(mime_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Replace the full suffix chain using a media type.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Non-empty path segments in path order.
    #[napi(getter)]
    pub fn path_segments(&self) -> Vec<String> {
        self.inner.path_segments().map(ToOwned::to_owned).collect()
    }

    /// Number of path segments exposed through iteration.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.path_segments().count()).unwrap_or(u32::MAX)
    }

    /// Return a path segment at an Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<String> {
        let len = i64::from(self.length());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved)
            .ok()
            .and_then(|index| self.path_segment_at(index))
    }

    /// The partition: `aws`, `aws-cn`, `aws-us-gov`, or another AWS names.
    #[napi(getter)]
    pub fn partition(&self) -> String {
        self.inner.partition().to_owned()
    }

    /// The service namespace: `s3`, `iam`, `lambda`, and the rest.
    #[napi(getter)]
    pub fn service(&self) -> String {
        self.inner.service().to_owned()
    }

    /// The region, or `null` for a service that spans every region.
    #[napi(getter)]
    pub fn region(&self) -> Option<String> {
        self.inner.region().map(ToOwned::to_owned)
    }

    /// The owning account, or `null` when the ARN names none.
    #[napi(getter)]
    pub fn account(&self) -> Option<String> {
        self.inner.account().map(ToOwned::to_owned)
    }

    /// The resource field whole, its own `/` and `:` structure kept.
    #[napi(getter)]
    pub fn resource(&self) -> String {
        self.inner.resource().to_owned()
    }

    /// The separator the resource field uses, if it carries one.
    #[napi(getter)]
    pub fn resource_separator(&self) -> Option<String> {
        self.inner
            .resource_separator()
            .map(|separator| separator.to_string())
    }

    /// The resource type, when the resource names one.
    ///
    /// This is the syntactic split at the resource's first `/` or `:`. What
    /// that leading part means is the service's own business - for Amazon S3 it
    /// is the bucket, which `bucket` is the door for.
    #[napi(getter)]
    pub fn resource_type(&self) -> Option<String> {
        self.inner.resource_type().map(ToOwned::to_owned)
    }

    /// What follows the type, or the whole resource when it names no type.
    #[napi(getter)]
    pub fn resource_id(&self) -> String {
        self.inner.resource_id().to_owned()
    }

    /// The bucket an Amazon S3 ARN names.
    #[napi(getter)]
    pub fn bucket(&self) -> Option<String> {
        self.inner.bucket().map(ToOwned::to_owned)
    }

    /// The object key an Amazon S3 ARN names, below its bucket.
    #[napi(getter)]
    pub fn key(&self) -> Option<String> {
        self.inner.key().map(ToOwned::to_owned)
    }

    /// The location this name addresses, as a `Url`.
    ///
    /// An Amazon S3 ARN names a bucket and, below it, a key, which is exactly
    /// what an `s3:` URL locates. Every other service addresses something no
    /// URL locates, and is refused by name.
    #[napi]
    pub fn locator(&self) -> Result<JsUrl> {
        self.inner
            .locator()
            .map(JsUrl::from_core)
            .map_err(napi_error)
    }

    /// Convert this ARN to its general URI representation.
    #[napi]
    pub fn into_uri(&self) -> JsUri {
        JsUri::from_core(self.inner.clone().into_uri())
    }

    /// Exact normalized native equality.
    #[napi]
    pub fn equals(&self, other: &JsArn) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsArn) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of the canonical ARN.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}
