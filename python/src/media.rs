//! Native Python views of MIME and compound media types.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyTuple};
use yggdryl::{MediaType as CoreMediaType, MimeType as CoreMimeType};

use crate::uri::path_string_from_value;
use crate::{compare, normalize_index, value_error};

/// Extracts an exact native MIME wrapper or parses a MIME/extension string.
pub(crate) fn core_mime_type_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreMimeType> {
    if let Ok(value) = value.extract::<PyRef<'_, PyMimeType>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreMimeType::from_str(value).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.MimeType or MIME/extension string",
    ))
}

/// Extracts an exact native media wrapper, MIME wrapper, or parsed string.
pub(crate) fn core_media_type_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreMediaType> {
    if let Ok(value) = value.extract::<PyRef<'_, PyMediaType>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyMimeType>>() {
        return Ok(CoreMediaType::new(value.inner.clone()));
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreMediaType::from_str(value).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.MediaType, yggdryl.MimeType, or media string",
    ))
}

fn mime_types_from_iterable(value: &Bound<'_, PyAny>) -> PyResult<Vec<CoreMimeType>> {
    if value.extract::<&str>().is_ok() {
        return Err(PyTypeError::new_err(
            "encodings must be an iterable of MimeType values or strings, not one string",
        ));
    }
    value
        .try_iter()?
        .map(|item| item.and_then(|item| core_mime_type_from_value(&item)))
        .collect()
}

pub(crate) fn strings_from_iterable(
    value: &Bound<'_, PyAny>,
    label: &str,
) -> PyResult<Vec<String>> {
    if value.extract::<&str>().is_ok() {
        return Err(PyTypeError::new_err(format!(
            "{label} must be an iterable of strings, not one string"
        )));
    }
    value
        .try_iter()?
        .map(|item| item?.extract::<String>())
        .collect()
}

/// An immutable validated canonical MIME `type/subtype` value.
#[pyclass(
    name = "MimeType",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyMimeType {
    pub(crate) inner: CoreMimeType,
}

impl PyMimeType {
    pub(crate) fn from_core(inner: CoreMimeType) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMimeType {
    #[classattr]
    #[pyo3(name = "OCTET_STREAM")]
    fn octet_stream_constant() -> Self {
        Self::from_core(CoreMimeType::OCTET_STREAM)
    }

    #[classattr]
    #[pyo3(name = "JSON")]
    fn json_constant() -> Self {
        Self::from_core(CoreMimeType::JSON)
    }

    #[classattr]
    #[pyo3(name = "JSON_LINES")]
    fn json_lines_constant() -> Self {
        Self::from_core(CoreMimeType::JSON_LINES)
    }

    #[classattr]
    #[pyo3(name = "YAML")]
    fn yaml_constant() -> Self {
        Self::from_core(CoreMimeType::YAML)
    }

    #[classattr]
    #[pyo3(name = "TOML")]
    fn toml_constant() -> Self {
        Self::from_core(CoreMimeType::TOML)
    }

    #[classattr]
    #[pyo3(name = "CSV")]
    fn csv_constant() -> Self {
        Self::from_core(CoreMimeType::CSV)
    }

    #[classattr]
    #[pyo3(name = "TSV")]
    fn tsv_constant() -> Self {
        Self::from_core(CoreMimeType::TSV)
    }

    #[classattr]
    #[pyo3(name = "PARQUET")]
    fn parquet_constant() -> Self {
        Self::from_core(CoreMimeType::PARQUET)
    }

    #[classattr]
    #[pyo3(name = "ARROW_FILE")]
    fn arrow_file_constant() -> Self {
        Self::from_core(CoreMimeType::ARROW_FILE)
    }

    #[classattr]
    #[pyo3(name = "ARROW_STREAM")]
    fn arrow_stream_constant() -> Self {
        Self::from_core(CoreMimeType::ARROW_STREAM)
    }

    #[classattr]
    #[pyo3(name = "AVRO")]
    fn avro_constant() -> Self {
        Self::from_core(CoreMimeType::AVRO)
    }

    #[classattr]
    #[pyo3(name = "ORC")]
    fn orc_constant() -> Self {
        Self::from_core(CoreMimeType::ORC)
    }

    #[classattr]
    #[pyo3(name = "PLAIN_TEXT")]
    fn plain_text_constant() -> Self {
        Self::from_core(CoreMimeType::PLAIN_TEXT)
    }

    #[classattr]
    #[pyo3(name = "MARKDOWN")]
    fn markdown_constant() -> Self {
        Self::from_core(CoreMimeType::MARKDOWN)
    }

    #[classattr]
    #[pyo3(name = "HTML")]
    fn html_constant() -> Self {
        Self::from_core(CoreMimeType::HTML)
    }

    #[classattr]
    #[pyo3(name = "CSS")]
    fn css_constant() -> Self {
        Self::from_core(CoreMimeType::CSS)
    }

    #[classattr]
    #[pyo3(name = "JAVASCRIPT")]
    fn javascript_constant() -> Self {
        Self::from_core(CoreMimeType::JAVASCRIPT)
    }

    #[classattr]
    #[pyo3(name = "XML")]
    fn xml_constant() -> Self {
        Self::from_core(CoreMimeType::XML)
    }

    #[classattr]
    #[pyo3(name = "PDF")]
    fn pdf_constant() -> Self {
        Self::from_core(CoreMimeType::PDF)
    }

    #[classattr]
    #[pyo3(name = "CBOR")]
    fn cbor_constant() -> Self {
        Self::from_core(CoreMimeType::CBOR)
    }

    #[classattr]
    #[pyo3(name = "MESSAGE_PACK")]
    fn message_pack_constant() -> Self {
        Self::from_core(CoreMimeType::MESSAGE_PACK)
    }

    #[classattr]
    #[pyo3(name = "PROTOBUF")]
    fn protobuf_constant() -> Self {
        Self::from_core(CoreMimeType::PROTOBUF)
    }

    #[classattr]
    #[pyo3(name = "SQLITE3")]
    fn sqlite3_constant() -> Self {
        Self::from_core(CoreMimeType::SQLITE3)
    }

    #[classattr]
    #[pyo3(name = "PNG")]
    fn png_constant() -> Self {
        Self::from_core(CoreMimeType::PNG)
    }

    #[classattr]
    #[pyo3(name = "JPEG")]
    fn jpeg_constant() -> Self {
        Self::from_core(CoreMimeType::JPEG)
    }

    #[classattr]
    #[pyo3(name = "GIF")]
    fn gif_constant() -> Self {
        Self::from_core(CoreMimeType::GIF)
    }

    #[classattr]
    #[pyo3(name = "WEBP")]
    fn webp_constant() -> Self {
        Self::from_core(CoreMimeType::WEBP)
    }

    #[classattr]
    #[pyo3(name = "SVG")]
    fn svg_constant() -> Self {
        Self::from_core(CoreMimeType::SVG)
    }

    #[classattr]
    #[pyo3(name = "MP3")]
    fn mp3_constant() -> Self {
        Self::from_core(CoreMimeType::MP3)
    }

    #[classattr]
    #[pyo3(name = "WAV")]
    fn wav_constant() -> Self {
        Self::from_core(CoreMimeType::WAV)
    }

    #[classattr]
    #[pyo3(name = "OGG")]
    fn ogg_constant() -> Self {
        Self::from_core(CoreMimeType::OGG)
    }

    #[classattr]
    #[pyo3(name = "FLAC")]
    fn flac_constant() -> Self {
        Self::from_core(CoreMimeType::FLAC)
    }

    #[classattr]
    #[pyo3(name = "MP4")]
    fn mp4_constant() -> Self {
        Self::from_core(CoreMimeType::MP4)
    }

    #[classattr]
    #[pyo3(name = "WEBM")]
    fn webm_constant() -> Self {
        Self::from_core(CoreMimeType::WEBM)
    }

    #[classattr]
    #[pyo3(name = "WOFF")]
    fn woff_constant() -> Self {
        Self::from_core(CoreMimeType::WOFF)
    }

    #[classattr]
    #[pyo3(name = "WOFF2")]
    fn woff2_constant() -> Self {
        Self::from_core(CoreMimeType::WOFF2)
    }

    #[classattr]
    #[pyo3(name = "TTF")]
    fn ttf_constant() -> Self {
        Self::from_core(CoreMimeType::TTF)
    }

    #[classattr]
    #[pyo3(name = "OTF")]
    fn otf_constant() -> Self {
        Self::from_core(CoreMimeType::OTF)
    }

    #[classattr]
    #[pyo3(name = "XLS")]
    fn xls_constant() -> Self {
        Self::from_core(CoreMimeType::XLS)
    }

    #[classattr]
    #[pyo3(name = "XLSX")]
    fn xlsx_constant() -> Self {
        Self::from_core(CoreMimeType::XLSX)
    }

    #[classattr]
    #[pyo3(name = "ODS")]
    fn ods_constant() -> Self {
        Self::from_core(CoreMimeType::ODS)
    }

    #[classattr]
    #[pyo3(name = "DOC")]
    fn doc_constant() -> Self {
        Self::from_core(CoreMimeType::DOC)
    }

    #[classattr]
    #[pyo3(name = "DOCX")]
    fn docx_constant() -> Self {
        Self::from_core(CoreMimeType::DOCX)
    }

    #[classattr]
    #[pyo3(name = "GZIP")]
    fn gzip_constant() -> Self {
        Self::from_core(CoreMimeType::GZIP)
    }

    #[classattr]
    #[pyo3(name = "ZSTD")]
    fn zstd_constant() -> Self {
        Self::from_core(CoreMimeType::ZSTD)
    }

    #[classattr]
    #[pyo3(name = "BROTLI")]
    fn brotli_constant() -> Self {
        Self::from_core(CoreMimeType::BROTLI)
    }

    #[classattr]
    #[pyo3(name = "ZLIB")]
    fn zlib_constant() -> Self {
        Self::from_core(CoreMimeType::ZLIB)
    }

    #[classattr]
    #[pyo3(name = "COMPRESS")]
    fn compress_constant() -> Self {
        Self::from_core(CoreMimeType::COMPRESS)
    }

    #[classattr]
    #[pyo3(name = "BZIP2")]
    fn bzip2_constant() -> Self {
        Self::from_core(CoreMimeType::BZIP2)
    }

    #[classattr]
    #[pyo3(name = "XZ")]
    fn xz_constant() -> Self {
        Self::from_core(CoreMimeType::XZ)
    }

    #[classattr]
    #[pyo3(name = "LZ4")]
    fn lz4_constant() -> Self {
        Self::from_core(CoreMimeType::LZ4)
    }

    #[classattr]
    #[pyo3(name = "SNAPPY")]
    fn snappy_constant() -> Self {
        Self::from_core(CoreMimeType::SNAPPY)
    }

    #[classattr]
    #[pyo3(name = "ZIP")]
    fn zip_constant() -> Self {
        Self::from_core(CoreMimeType::ZIP)
    }

    #[classattr]
    #[pyo3(name = "SEVEN_ZIP")]
    fn seven_zip_constant() -> Self {
        Self::from_core(CoreMimeType::SEVEN_ZIP)
    }

    #[classattr]
    #[pyo3(name = "RAR")]
    fn rar_constant() -> Self {
        Self::from_core(CoreMimeType::RAR)
    }

    #[classattr]
    #[pyo3(name = "TAR")]
    fn tar_constant() -> Self {
        Self::from_core(CoreMimeType::TAR)
    }

    #[new]
    #[pyo3(signature = (value=None))]
    fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        value.map_or_else(
            || Ok(Self::from_core(CoreMimeType::default())),
            |value| core_mime_type_from_value(value).map(Self::from_core),
        )
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_mime_type_from_value(value).map(Self::from_core)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreMimeType::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_extension(value: &str) -> PyResult<Self> {
        CoreMimeType::from_extension(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_path(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreMimeType::from_path(path_string_from_value(value)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_content_type(value: &str) -> PyResult<Self> {
        CoreMimeType::from_content_type(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_content_coding(value: &str) -> PyResult<Self> {
        CoreMimeType::from_content_coding(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        serde_json::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner).map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.to_json()
    }

    #[getter]
    fn top_level(&self) -> &str {
        self.inner.top_level()
    }

    #[getter]
    fn subtype(&self) -> &str {
        self.inner.subtype()
    }

    #[getter]
    fn structured_suffix(&self) -> Option<&str> {
        self.inner.structured_suffix()
    }

    #[getter]
    fn extension(&self) -> Option<&'static str> {
        self.inner.extension()
    }

    #[getter]
    fn content_coding(&self) -> Option<&'static str> {
        self.inner.content_coding()
    }

    #[getter]
    fn format(&self) -> Option<&'static str> {
        self.inner.format().map(yggdryl::Format::as_str)
    }

    fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    fn is_application(&self) -> bool {
        self.inner.is_application()
    }

    fn is_audio(&self) -> bool {
        self.inner.is_audio()
    }

    fn is_font(&self) -> bool {
        self.inner.is_font()
    }

    fn is_haptics(&self) -> bool {
        self.inner.is_haptics()
    }

    fn is_image(&self) -> bool {
        self.inner.is_image()
    }

    fn is_message(&self) -> bool {
        self.inner.is_message()
    }

    fn is_model(&self) -> bool {
        self.inner.is_model()
    }

    fn is_multipart(&self) -> bool {
        self.inner.is_multipart()
    }

    fn is_text(&self) -> bool {
        self.inner.is_text()
    }

    fn is_video(&self) -> bool {
        self.inner.is_video()
    }

    fn is_textual(&self) -> bool {
        self.inner.is_textual()
    }

    fn is_structured(&self) -> bool {
        self.inner.is_structured()
    }

    fn is_tabular(&self) -> bool {
        self.inner.is_tabular()
    }

    fn is_encoding(&self) -> bool {
        self.inner.is_encoding()
    }

    fn is_archive(&self) -> bool {
        self.inner.is_archive()
    }

    fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __str__(&self) -> &str {
        self.inner.as_str()
    }

    fn __repr__(&self) -> String {
        format!("MimeType.from_str({:?})", self.inner.as_str())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __hash__(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// A base MIME type plus ordered transparent encodings.
#[pyclass(name = "MediaType", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyMediaType {
    pub(crate) inner: CoreMediaType,
}

impl PyMediaType {
    pub(crate) fn from_core(inner: CoreMediaType) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyMediaType {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    #[new]
    #[pyo3(signature = (value=None))]
    fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        value.map_or_else(
            || Ok(Self::from_core(CoreMediaType::default())),
            |value| core_media_type_from_value(value).map(Self::from_core),
        )
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_media_type_from_value(value).map(Self::from_core)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreMediaType::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_parts(base: &Bound<'_, PyAny>, encodings: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreMediaType::from_parts(
            core_mime_type_from_value(base)?,
            mime_types_from_iterable(encodings)?,
        )
        .map(Self::from_core)
        .map_err(value_error)
    }

    #[staticmethod]
    fn from_extension(value: &str) -> Self {
        Self::from_core(CoreMediaType::from_extension(value))
    }

    #[staticmethod]
    fn from_extensions(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_core(CoreMediaType::from_extensions(
            strings_from_iterable(value, "extensions")?,
        )))
    }

    #[staticmethod]
    fn from_file_name(value: &str) -> Self {
        Self::from_core(CoreMediaType::from_file_name(value))
    }

    #[staticmethod]
    fn from_path(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        CoreMediaType::from_path(path_string_from_value(value)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    #[pyo3(signature = (content_type=None, content_encoding=None))]
    fn from_content_headers(
        content_type: Option<&str>,
        content_encoding: Option<&str>,
    ) -> PyResult<Self> {
        CoreMediaType::from_content_headers(content_type, content_encoding)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        serde_json::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner).map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.to_json()
    }

    #[getter]
    fn base(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.base().clone())
    }

    #[getter]
    fn encodings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.inner
                .encodings()
                .iter()
                .cloned()
                .map(PyMimeType::from_core),
        )
    }

    #[getter]
    fn encoding(&self) -> Option<PyMimeType> {
        self.inner.encoding().cloned().map(PyMimeType::from_core)
    }

    #[getter]
    fn extension(&self) -> Option<&'static str> {
        self.inner.extension()
    }

    #[getter]
    fn extensions(&self) -> Vec<&'static str> {
        self.inner.extensions().collect()
    }

    fn set_base(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.set_base(core_mime_type_from_value(value)?);
        Ok(())
    }

    fn set_encodings(&mut self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .set_encodings(mime_types_from_iterable(values)?)
            .map_err(value_error)
    }

    fn push_encoding(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .push_encoding(core_mime_type_from_value(value)?)
            .map_err(value_error)
    }

    fn clear_encodings(&mut self) -> bool {
        self.inner.clear_encodings()
    }

    fn is_encoded(&self) -> bool {
        self.inner.is_encoded()
    }

    fn is_application(&self) -> bool {
        self.inner.is_application()
    }

    fn is_audio(&self) -> bool {
        self.inner.is_audio()
    }

    fn is_font(&self) -> bool {
        self.inner.is_font()
    }

    fn is_haptics(&self) -> bool {
        self.inner.is_haptics()
    }

    fn is_image(&self) -> bool {
        self.inner.is_image()
    }

    fn is_message(&self) -> bool {
        self.inner.is_message()
    }

    fn is_model(&self) -> bool {
        self.inner.is_model()
    }

    fn is_multipart(&self) -> bool {
        self.inner.is_multipart()
    }

    fn is_text(&self) -> bool {
        self.inner.is_text()
    }

    fn is_video(&self) -> bool {
        self.inner.is_video()
    }

    fn is_textual(&self) -> bool {
        self.inner.is_textual()
    }

    fn is_structured(&self) -> bool {
        self.inner.is_structured()
    }

    fn is_tabular(&self) -> bool {
        self.inner.is_tabular()
    }

    fn is_encoding(&self) -> bool {
        self.inner.is_encoding()
    }

    fn is_archive(&self) -> bool {
        self.inner.is_archive()
    }

    fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __len__(&self) -> usize {
        self.inner.encoding_len()
    }

    fn __iter__(&self) -> PyMediaTypeIterator {
        PyMediaTypeIterator {
            inner: self.inner.clone(),
            index: 0,
        }
    }

    fn __getitem__(&self, index: isize) -> PyResult<PyMimeType> {
        let index = normalize_index(index, self.inner.encoding_len())
            .ok_or_else(|| PyIndexError::new_err(index))?;
        self.inner
            .get_encoding(index)
            .cloned()
            .map(PyMimeType::from_core)
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> bool {
        core_mime_type_from_value(value).is_ok_and(|value| self.inner.encodings().contains(&value))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("MediaType.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Owning single-pass iterator over a media value's ordered encodings.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyMediaTypeIterator {
    inner: CoreMediaType,
    index: usize,
}

#[pymethods]
impl PyMediaTypeIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyMimeType> {
        let value = self
            .inner
            .get_encoding(self.index)
            .cloned()
            .map(PyMimeType::from_core);
        if value.is_some() {
            self.index += 1;
        }
        value
    }

    fn __length_hint__(&self) -> usize {
        self.inner.encoding_len().saturating_sub(self.index)
    }
}
