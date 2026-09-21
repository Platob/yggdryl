//! One test file per file at the crate root, under `tests/root/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the files the
//! crate root itself holds. A test reaches the crate through `yggdryl::`;
//! where what it pins is not reachable that way, it reaches
//! `yggdryl::internals`, which exists only under the `internals` feature, and
//! the file that reaches it is declared behind that feature here.

#[path = "support/counting.rs"]
mod counting;

#[cfg(feature = "internals")]
#[path = "root/arithmetic.rs"]
mod arithmetic;
#[path = "root/ascii.rs"]
mod ascii;
#[path = "root/boolean.rs"]
mod boolean;
#[path = "root/budget.rs"]
mod budget;
#[path = "root/bytes.rs"]
mod bytes;
#[cfg(feature = "internals")]
#[path = "root/bytestream.rs"]
mod bytestream;
#[path = "root/cast.rs"]
mod cast;
#[path = "root/cfi_code.rs"]
mod cfi_code;
#[path = "root/charset.rs"]
mod charset;
#[path = "root/code.rs"]
mod code;
#[path = "root/codec.rs"]
mod codec;
#[path = "root/compatibility.rs"]
mod compatibility;
#[path = "root/cusip_code.rs"]
mod cusip_code;
#[path = "root/datatype.rs"]
mod datatype;
#[path = "root/datatype_id.rs"]
mod datatype_id;
#[path = "root/datatype_kind.rs"]
mod datatype_kind;
#[path = "root/date.rs"]
mod date;
#[path = "root/datetime.rs"]
mod datetime;
#[path = "root/decimal.rs"]
mod decimal;
#[path = "root/default.rs"]
mod default;
#[path = "root/diff.rs"]
mod diff;
#[path = "root/digest.rs"]
mod digest;
#[path = "root/duration.rs"]
mod duration;
#[path = "root/edge_algorithm.rs"]
mod edge_algorithm;
#[path = "root/enumeration.rs"]
mod enumeration;
#[path = "root/error.rs"]
mod error;
#[path = "root/field.rs"]
mod field;
#[path = "root/figi_code.rs"]
mod figi_code;
#[path = "root/floating.rs"]
mod floating;
#[path = "root/geospatial.rs"]
mod geospatial;
#[path = "root/gzip.rs"]
mod gzip;
#[path = "root/int256.rs"]
mod int256;
#[path = "root/integer.rs"]
mod integer;
#[path = "root/interval.rs"]
mod interval;
#[path = "root/iobase.rs"]
mod iobase;
#[path = "root/iocursor.rs"]
mod iocursor;
#[path = "root/iokind.rs"]
mod iokind;
#[path = "root/iomedia.rs"]
mod iomedia;
#[path = "root/iomode.rs"]
mod iomode;
#[path = "root/lib.rs"]
mod lib;
#[path = "root/listing.rs"]
mod listing;
#[path = "root/mapping.rs"]
mod mapping;
#[path = "root/media_type.rs"]
mod media_type;
#[path = "root/merge.rs"]
mod merge;
#[path = "root/metadata.rs"]
mod metadata;
#[path = "root/mime_type.rs"]
mod mime_type;
#[cfg(feature = "internals")]
#[path = "root/parallel.rs"]
mod parallel;
#[path = "root/parser.rs"]
mod parser;
#[cfg(feature = "internals")]
#[path = "root/path.rs"]
mod path;
#[path = "root/protocol.rs"]
mod protocol;
#[path = "root/regex.rs"]
mod regex;
#[path = "root/scalar.rs"]
mod scalar;
#[path = "root/scheme.rs"]
mod scheme;
#[path = "root/sedol_code.rs"]
mod sedol_code;
#[path = "root/serde.rs"]
mod serde;
#[path = "root/state.rs"]
mod state;
#[path = "root/string.rs"]
mod string;
#[path = "root/structure.rs"]
mod structure;
#[path = "root/temporal.rs"]
mod temporal;
#[path = "root/time.rs"]
mod time;
#[path = "root/time_unit.rs"]
mod time_unit;
#[path = "root/timeinforce.rs"]
mod timeinforce;
#[path = "root/timezone.rs"]
mod timezone;
#[path = "root/typed.rs"]
mod typed;
#[path = "root/union.rs"]
mod union;
#[path = "root/utf8.rs"]
mod utf8;
#[path = "root/uuid.rs"]
mod uuid;
#[path = "root/valuestream.rs"]
mod valuestream;
#[path = "root/variant.rs"]
mod variant;
#[path = "root/version.rs"]
mod version;
#[path = "root/vocabulary.rs"]
mod vocabulary;
#[path = "root/wkb.rs"]
mod wkb;
#[path = "root/zlib.rs"]
mod zlib;
#[path = "root/zstd.rs"]
mod zstd;
