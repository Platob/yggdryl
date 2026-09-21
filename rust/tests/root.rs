//! One test file per file at the crate root, under `tests/root/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the files the
//! crate root itself holds. A test reaches the crate through `yggdryl::`;
//! where what it pins is not reachable that way, it reaches
//! `yggdryl::internals`, which exists only under the `internals` feature, and
//! the file that reaches it is declared behind that feature here.

#[cfg(feature = "internals")]
#[path = "root/arithmetic.rs"]
mod arithmetic;
#[cfg(feature = "internals")]
#[path = "root/bytestream.rs"]
mod bytestream;
#[cfg(feature = "internals")]
#[path = "root/decimal.rs"]
mod decimal;
#[cfg(feature = "internals")]
#[path = "root/diff.rs"]
mod diff;
#[path = "root/error.rs"]
mod error;
#[path = "root/lib.rs"]
mod lib;
#[path = "root/listing.rs"]
mod listing;
#[cfg(feature = "internals")]
#[path = "root/merge.rs"]
mod merge;
#[cfg(feature = "internals")]
#[path = "root/metadata.rs"]
mod metadata;
#[cfg(feature = "internals")]
#[path = "root/parallel.rs"]
mod parallel;
#[cfg(feature = "internals")]
#[path = "root/path.rs"]
mod path;
#[cfg(feature = "internals")]
#[path = "root/protocol.rs"]
mod protocol;
#[cfg(feature = "internals")]
#[path = "root/scalar.rs"]
mod scalar;
#[path = "root/string.rs"]
mod string;
#[cfg(feature = "internals")]
#[path = "root/temporal.rs"]
mod temporal;
#[cfg(feature = "internals")]
#[path = "root/timezone.rs"]
mod timezone;
#[cfg(feature = "internals")]
#[path = "root/utf8.rs"]
mod utf8;
#[cfg(feature = "internals")]
#[path = "root/valuestream.rs"]
mod valuestream;
#[cfg(feature = "internals")]
#[path = "root/variant.rs"]
mod variant;
#[cfg(feature = "internals")]
#[path = "root/version.rs"]
mod version;
