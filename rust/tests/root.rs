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
#[path = "root/bbg.rs"]
mod bbg;
#[path = "root/boolean.rs"]
mod boolean;
#[path = "root/budget.rs"]
mod budget;
#[path = "root/bytes.rs"]
mod bytes;
#[path = "root/bytestream.rs"]
mod bytestream;
#[path = "root/cast.rs"]
mod cast;
#[path = "root/ccy.rs"]
mod ccy;
#[path = "root/cfi.rs"]
mod cfi;
#[path = "root/charset.rs"]
mod charset;
#[path = "root/chunked_serie.rs"]
mod chunked_serie;
#[path = "root/code.rs"]
mod code;
#[path = "root/codec.rs"]
mod codec;
#[path = "root/compatibility.rs"]
mod compatibility;
#[path = "root/cusip.rs"]
mod cusip;
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
#[path = "root/enums.rs"]
mod enums;
#[path = "root/error.rs"]
mod error;
#[path = "root/field.rs"]
mod field;
#[path = "root/figi.rs"]
mod figi;
#[path = "root/floating.rs"]
mod floating;
#[path = "root/forex.rs"]
mod forex;
#[path = "root/geospatial.rs"]
mod geospatial;
#[path = "root/gzip.rs"]
mod gzip;
#[path = "root/hostname.rs"]
mod hostname;
#[path = "root/identifier.rs"]
mod identifier;
#[path = "root/idsource.rs"]
mod idsource;
#[path = "root/idtype.rs"]
mod idtype;
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
#[path = "root/isin.rs"]
mod isin;
#[path = "root/lib.rs"]
mod lib;
#[path = "root/limit.rs"]
mod limit;
#[path = "root/listing.rs"]
mod listing;
#[path = "root/mapping.rs"]
mod mapping;
#[path = "root/marketdatakind.rs"]
mod marketdatakind;
#[path = "root/marketdatatype.rs"]
mod marketdatatype;
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
#[path = "root/ric.rs"]
mod ric;
#[path = "root/scalar.rs"]
mod scalar;
#[path = "root/scheme.rs"]
mod scheme;
#[path = "root/securityid.rs"]
mod securityid;
#[path = "root/sedol.rs"]
mod sedol;
#[path = "root/serde.rs"]
mod serde;
#[path = "root/serie.rs"]
mod serie;
#[path = "root/side.rs"]
mod side;
#[path = "root/sort_options.rs"]
mod sort_options;
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
#[path = "root/unit.rs"]
mod unit;
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
#[path = "root/window_serie.rs"]
mod window_serie;
#[path = "root/wkb.rs"]
mod wkb;
#[path = "root/zlib.rs"]
mod zlib;
#[path = "root/zstd.rs"]
mod zstd;
