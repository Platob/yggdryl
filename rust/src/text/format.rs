use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{Error, MimeType, Result};

/// A byte-oriented structured-data format understood by Yggdryl codecs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// One JSON value.
    Json,
    /// Newline-delimited JSON values.
    JsonLines,
    /// One or more YAML documents.
    Yaml,
    /// One TOML document.
    Toml,
}

impl Format {
    /// Every format in canonical order.
    pub const ALL: [Self; 4] = [Self::Json, Self::JsonLines, Self::Yaml, Self::Toml];

    /// Parse a format name or conventional extension.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Infer a format from an extension, with or without its leading dot.
    pub fn from_extension(extension: &str) -> Result<Self> {
        let extension = extension.strip_prefix('.').unwrap_or(extension);
        Self::from_str(extension)
    }

    /// Infer a format from the final extension of a path.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Error::Codec {
                format: "format",
                position: path.as_os_str().len(),
                reason: "path has no supported UTF-8 extension".into(),
            })?;
        Self::from_extension(extension)
    }

    /// Return the canonical format spelling without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::JsonLines => "json_lines",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
        }
    }

    /// Return the preferred file extension without a leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::JsonLines => "jsonl",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
        }
    }

    /// Return the canonical MIME type for this structured-text format.
    /// Name the format a MIME type describes.
    ///
    /// This is the exact reverse of [`Self::mime_type`], so the two cannot
    /// drift apart.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the accepted vocabulary and the input.
    pub fn from_mime_type(mime_type: &MimeType) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.mime_type() == *mime_type)
            .ok_or_else(|| Error::Parse {
                target: "structured text format",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "expected one of {}, got {mime_type}",
                    Self::ALL
                        .iter()
                        .map(|format| format.mime_type().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }

    pub const fn mime_type(self) -> MimeType {
        match self {
            Self::Json => MimeType::JSON,
            Self::JsonLines => MimeType::JSON_LINES,
            Self::Yaml => MimeType::YAML,
            Self::Toml => MimeType::TOML,
        }
    }
}

impl FromStr for Format {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("json_lines") || value.eq_ignore_ascii_case("json-lines") {
            return Ok(Self::JsonLines);
        }
        MimeType::from_str(value)
            .ok()
            .and_then(|mime| mime.format())
            .ok_or_else(|| Error::Codec {
                format: "format",
                position: 0,
                reason: "expected json, jsonl/ndjson, yaml/yml, or toml".into(),
            })
    }
}

impl AsRef<str> for Format {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Format {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Format {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}
