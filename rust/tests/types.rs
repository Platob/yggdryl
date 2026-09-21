//! Type-system integration tests, one module per `types/` subtree.

#[path = "types/batch_cast.rs"]
mod batch_cast;
#[path = "types/bytes.rs"]
mod bytes;
#[path = "types/cast.rs"]
mod cast;
#[path = "types/datatype/mod.rs"]
mod datatype;
#[path = "types/decimal.rs"]
mod decimal;
#[path = "types/default_scalar.rs"]
mod default_scalar;
#[path = "types/enums/mod.rs"]
mod enums;
#[path = "types/families.rs"]
mod families;
#[path = "types/field/mod.rs"]
mod field;
#[path = "types/geospatial/mod.rs"]
mod geospatial;
#[path = "types/i256.rs"]
mod i256;
#[path = "types/logical.rs"]
mod logical;
#[path = "types/media.rs"]
mod media;
#[path = "types/merge.rs"]
mod merge;
#[path = "types/metadata.rs"]
mod metadata;
#[path = "types/regex.rs"]
mod regex;
#[path = "types/scalar.rs"]
mod scalar;
#[path = "types/serie.rs"]
mod serie;
#[path = "types/strict_cast.rs"]
mod strict_cast;
#[path = "types/string_enum.rs"]
mod string_enum;
#[path = "types/strings.rs"]
mod strings;
#[path = "types/temporal.rs"]
mod temporal;
#[path = "types/timezone.rs"]
mod timezone;
#[path = "types/typed/mod.rs"]
mod typed;
#[path = "types/uri.rs"]
mod uri;
#[path = "types/url.rs"]
mod url;
#[path = "types/uuid.rs"]
mod uuid;
#[path = "types/value.rs"]
mod value;
#[path = "types/value_bounds.rs"]
mod value_bounds;
#[path = "types/valuestream.rs"]
mod valuestream;
#[path = "types/variant.rs"]
mod variant;
#[path = "types/version.rs"]
mod version;
#[path = "types/vocabulary.rs"]
mod vocabulary;
