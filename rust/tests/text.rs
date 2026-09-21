//! One test file per file in `rust/src/text/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the text folder.
//! A test reaches the crate through `yggdryl::`; where what it pins is not
//! reachable that way, it reaches `yggdryl::internals`, which exists only
//! under the `internals` feature, and the file that reaches it is declared
//! behind that feature here.

#[path = "text/batch.rs"]
mod batch;
#[path = "text/codec.rs"]
mod codec;
#[cfg(feature = "internals")]
#[path = "text/display.rs"]
mod display;
#[path = "text/format.rs"]
mod format;
#[cfg(feature = "internals")]
#[path = "text/io.rs"]
mod io;
#[path = "text/json.rs"]
mod json;
#[cfg(feature = "internals")]
#[path = "text/line.rs"]
mod line;
#[cfg(feature = "internals")]
#[path = "text/loading.rs"]
mod loading;
#[path = "text/placeholder.rs"]
mod placeholder;
#[cfg(feature = "internals")]
#[path = "text/position.rs"]
mod position;
#[cfg(feature = "internals")]
#[path = "text/reader.rs"]
mod reader;
#[path = "text/runtime_format.rs"]
mod runtime_format;
#[path = "text/structured.rs"]
mod structured;
#[path = "text/toml.rs"]
mod toml;
#[path = "text/value.rs"]
mod value;
#[path = "text/yaml.rs"]
mod yaml;
