//! One test file per file in `rust/src/text/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the text folder.
//! A test reaches the crate through `yggdryl::`; where what it pins is not
//! reachable that way, it reaches `yggdryl::internals`, which exists only
//! under the `internals` feature, and the file that reaches it is declared
//! behind that feature here.

#[path = "text/arrow.rs"]
mod arrow;
#[path = "text/batch.rs"]
mod batch;
#[path = "text/bytes.rs"]
mod bytes;
#[path = "text/codec.rs"]
mod codec;
#[cfg(feature = "internals")]
#[path = "text/display.rs"]
mod display;
#[path = "text/entry.rs"]
mod entry;
#[path = "text/format.rs"]
mod format;
#[path = "text/handle.rs"]
mod handle;
#[cfg(feature = "internals")]
#[path = "text/io.rs"]
mod io;
#[path = "text/leading.rs"]
mod leading;
#[path = "text/limits.rs"]
mod limits;
#[path = "text/line.rs"]
mod line;
#[cfg(feature = "internals")]
#[path = "text/loading.rs"]
mod loading;
#[path = "text/mod_.rs"]
mod mod_;
#[path = "text/options.rs"]
mod options;
#[path = "text/placeholder.rs"]
mod placeholder;
#[path = "text/plan.rs"]
mod plan;
#[cfg(feature = "internals")]
#[path = "text/position.rs"]
mod position;
#[path = "text/reader.rs"]
mod reader;
#[path = "text/sep.rs"]
mod sep;
#[path = "text/typed.rs"]
mod typed;
