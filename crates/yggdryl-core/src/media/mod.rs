//! Media (MIME) type detection: the [`MimeType`] enum (backed by a mutable global
//! registry of extensions/magic bytes) and the [`MediaType`] stack.
//!
//! - [`MimeType`] is an enum of common, individual MIME types (with an
//!   [`Other`](MimeType::Other) escape hatch). Each type's canonical string, file
//!   extensions and magic-byte signatures live in a **global registry** that can
//!   be extended or trimmed at runtime ([`MimeType::register`] /
//!   [`MimeType::unregister`]).
//! - [`MediaType`] is an ordered stack of [`MimeType`]s describing a layered
//!   file, so `data.csv.gz` becomes `MediaType([MimeType::Csv, MimeType::Gzip])`.
//!
//! ```
//! use yggdryl_core::{MediaType, MimeType};
//!
//! assert_eq!(MimeType::from_str("application/json").unwrap(), MimeType::Json);
//! assert_eq!(MimeType::from_str("zstd").unwrap(), MimeType::Zstd); // short name
//! assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
//! assert_eq!(MimeType::from_magic(b"PK\x03\x04..."), Some(MimeType::Zip));
//!
//! let stack = MediaType::from_path("data.csv.gz");
//! assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
//! ```

use std::fmt;

mod media_type;
mod mime;

pub use media_type::MediaType;
pub use mime::{MimeType, Signature};

/// Error returned when a media or MIME type cannot be interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// The input was empty.
    Empty,
    /// The input was not a `type/subtype` MIME form.
    Invalid(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::Empty => write!(f, "media type is empty"),
            MediaError::Invalid(value) => write!(f, "media type '{value}' is not 'type/subtype'"),
        }
    }
}

impl std::error::Error for MediaError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToOutput;

    #[test]
    fn mime_edge_cases() {
        // Malformed MIME strings.
        assert!(MimeType::from_str("/").is_err());
        assert!(MimeType::from_str("application/").is_err());
        assert!(MimeType::from_str("/json").is_err());
        // Case-insensitive lookup (the upper-case path of `from_mime`).
        assert_eq!(MimeType::from_mime("TEXT/CSV"), MimeType::Csv);
        assert_eq!(
            MimeType::from_str("Application/JSON").unwrap(),
            MimeType::Json
        );
        // Leading dots and case on extensions are normalised.
        assert_eq!(MimeType::from_extension("...GZ"), Some(MimeType::Gzip));
        assert_eq!(MimeType::from_extension(""), None);
    }

    #[test]
    fn media_type_edge_cases() {
        // Trailing dot, empty, and extension-less names yield an empty stack.
        assert!(MediaType::from_path("a.").is_empty());
        assert!(MediaType::from_path("").is_empty());
        assert!(MediaType::from_path("/a/b/").is_empty());
        // A leading dot makes the first segment a dotfile stem.
        assert_eq!(MediaType::from_path(".tar.gz").types(), [MimeType::Gzip]);
        // A directory that looks like an extension is ignored (only the name).
        assert!(MediaType::from_path("/srv/json/file").is_empty());
        // from_str on an empty input errors, unlike from_path.
        assert_eq!(MediaType::from_str(""), Err(MediaError::Empty));
    }

    #[test]
    fn parses_and_splits_mime() {
        let m = MimeType::from_str("application/json").unwrap();
        assert_eq!(m, MimeType::Json);
        assert_eq!(m.mime(), "application/json");
        assert_eq!(m.type_(), "application");
        assert_eq!(m.subtype(), "json");
        // Parameters are dropped; case is normalised.
        assert_eq!(
            MimeType::from_str("Text/HTML; charset=utf-8").unwrap(),
            MimeType::Html
        );
    }

    #[test]
    fn unknown_becomes_other() {
        let m = MimeType::from_str("application/x-custom").unwrap();
        assert_eq!(m, MimeType::Other("application/x-custom".to_string()));
        assert!(!m.is_known());
        assert_eq!(m.subtype(), "x-custom");
        assert_eq!(m.extension(), None);
    }

    #[test]
    fn errors() {
        assert_eq!(MimeType::from_str(""), Err(MediaError::Empty));
        assert_eq!(
            MimeType::from_str("notamime"),
            Err(MediaError::Invalid("notamime".to_string()))
        );
        // A well-formed but unknown type is kept as `Other`.
        assert_eq!(
            MimeType::from_str("application/x-unknown").unwrap(),
            MimeType::Other("application/x-unknown".to_string())
        );
    }

    #[test]
    fn from_extension_maps_known_types() {
        assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
        assert_eq!(MimeType::from_extension(".GZ"), Some(MimeType::Gzip));
        assert_eq!(MimeType::from_extension("jpeg"), Some(MimeType::Jpeg));
        assert_eq!(MimeType::from_extension("nope"), None);
    }

    #[test]
    fn from_magic_sniffs_content() {
        assert_eq!(
            MimeType::from_magic(b"PAR1\x15\x04"),
            Some(MimeType::Parquet)
        );
        assert_eq!(
            MimeType::from_magic(b"ARROW1\x00\x00"),
            Some(MimeType::Arrow)
        );
        assert_eq!(MimeType::from_magic(b"PK\x03\x04\x14"), Some(MimeType::Zip));
        assert_eq!(
            MimeType::from_magic(b"\x1f\x8b\x08\x00"),
            Some(MimeType::Gzip)
        );
        assert_eq!(
            MimeType::from_magic(b"\x89PNG\r\n\x1a\n\x00"),
            Some(MimeType::Png)
        );
        // Offset-based signatures: tar's `ustar` lives at byte 257.
        let mut tar = vec![0u8; 270];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(MimeType::from_magic(&tar), Some(MimeType::Tar));
        assert_eq!(MimeType::from_magic(b"not magic"), None);
    }

    #[test]
    fn media_type_is_an_ordered_stack() {
        let stack = MediaType::from_path("data.csv.gz");
        assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
        assert_eq!(stack.first(), Some(&MimeType::Csv));
        assert_eq!(stack.last(), Some(&MimeType::Gzip));
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.to_str(true), "csv.gz");
        // Unknown extensions are skipped; nested dirs and dotfiles handled.
        assert_eq!(
            MediaType::from_path("/srv/dump.bak.parquet").types(),
            [MimeType::Parquet]
        );
        assert!(MediaType::from_path("/etc/.bashrc").is_empty());
        assert!(MediaType::from_path("/usr/bin/env").is_empty());
    }

    #[test]
    fn media_type_explicit_and_round_trip() {
        let stack = MediaType::new(vec![MimeType::Csv, MimeType::Gzip]);
        assert_eq!(stack, MediaType::from_path("x.csv.gz"));
        assert_eq!(
            MediaType::from_str("a/b/c.tar.gz").unwrap().to_str(true),
            "tar.gz"
        );
    }

    #[test]
    fn defaults_to_octet_stream() {
        assert_eq!(MimeType::default(), MimeType::OctetStream);
        assert_eq!(MimeType::default().mime(), "application/octet-stream");
        assert_eq!(MediaType::default().types(), [MimeType::OctetStream]);
        // The conventional fallback for failed inference.
        assert_eq!(
            MimeType::from_extension("nope").unwrap_or_default(),
            MimeType::OctetStream
        );
        assert_eq!(
            MimeType::from_path("notes").unwrap_or_default().mime(),
            "application/octet-stream"
        );
    }

    #[test]
    fn from_str_accepts_short_names() {
        // Short names resolve via extension or subtype, case-insensitively.
        assert_eq!(MimeType::from_str("json").unwrap(), MimeType::Json);
        assert_eq!(MimeType::from_str("gzip").unwrap(), MimeType::Gzip); // extension alias
        assert_eq!(MimeType::from_str("gz").unwrap(), MimeType::Gzip); // extension
        assert_eq!(MimeType::from_str("zstd").unwrap(), MimeType::Zstd); // subtype
        assert_eq!(MimeType::from_str("PNG").unwrap(), MimeType::Png);
        assert!(MimeType::from_str("nope").is_err());
        // A full `type/subtype` still parses as before.
        assert_eq!(
            MimeType::from_str("application/json").unwrap(),
            MimeType::Json
        );
        // MediaType: a bare name is a single-element stack; paths stay stacks.
        assert_eq!(
            MediaType::from_str("gzip").unwrap().types(),
            [MimeType::Gzip]
        );
        assert_eq!(
            MediaType::from_str("zstd").unwrap().types(),
            [MimeType::Zstd]
        );
        assert!(MediaType::from_str("nope").unwrap().is_empty());
        assert_eq!(
            MediaType::from_str("a/b/data.csv.gz").unwrap().types(),
            [MimeType::Csv, MimeType::Gzip]
        );
    }

    #[test]
    fn convenient_from_constructors() {
        // MimeType: straight from `type`/`subtype` parts, no string parse.
        assert_eq!(MimeType::from_parts("text", "csv"), MimeType::Csv);
        assert_eq!(
            MimeType::from_parts("application", "x-foo"),
            MimeType::Other("application/x-foo".to_string())
        );
        // MimeType: a single (outermost) type from a path.
        assert_eq!(MimeType::from_path("data.csv.gz"), Some(MimeType::Gzip));
        assert_eq!(MimeType::from_path("notes"), None);
        // MediaType: from one or many extensions, and from a mapping.
        assert_eq!(MediaType::from_extension("json").types(), [MimeType::Json]);
        assert_eq!(
            MediaType::from_extensions(&["csv", "nope", "gz"]).types(),
            [MimeType::Csv, MimeType::Gzip]
        );
        // to_mapping/from_mapping round-trip via the `types` key (MIME list).
        let stack = MediaType::from_path("a/b.csv.gz");
        assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
        assert_eq!(
            stack.to_mapping().get("types"),
            Some(&"text/csv,application/gzip".to_string())
        );
        assert_eq!(MediaType::from_mapping(&stack.to_mapping()).unwrap(), stack);
    }

    #[test]
    fn registry_add_and_remove() {
        // A custom type is unknown until registered.
        assert_eq!(MimeType::from_extension("ygg"), None);
        MimeType::register(
            "application/x-yggdryl",
            &["ygg"],
            &[Signature::prefix(b"YGG1")],
        );
        let m = MimeType::from_extension("ygg").unwrap();
        assert_eq!(m, MimeType::Other("application/x-yggdryl".to_string()));
        assert_eq!(m.extensions(), vec!["ygg".to_string()]);
        assert_eq!(
            MimeType::from_magic(b"YGG1\x00"),
            Some(MimeType::Other("application/x-yggdryl".to_string()))
        );
        // Unregistering removes it again.
        assert!(MimeType::unregister("application/x-yggdryl"));
        assert_eq!(MimeType::from_extension("ygg"), None);
        assert!(!MimeType::unregister("application/x-yggdryl"));
    }

    #[test]
    fn round_trips_through_mapping() {
        let m = MimeType::Svg;
        assert_eq!(m.mime(), "image/svg+xml");
        let map = m.to_mapping();
        assert_eq!(map.get("type"), Some(&"image".to_string()));
        assert_eq!(map.get("subtype"), Some(&"svg+xml".to_string()));
        assert_eq!(MimeType::from_mapping(&map).unwrap(), m);
    }

    #[test]
    fn extensions_and_to_str() {
        assert_eq!(MimeType::Jpeg.extensions(), vec!["jpg", "jpeg"]);
        assert_eq!(MimeType::Jpeg.extension(), Some("jpg".to_string()));
        assert!(MimeType::Other("x/y".to_string()).extensions().is_empty());
        assert_eq!(MimeType::Gzip.to_str(true), "application/gzip");
    }
}
