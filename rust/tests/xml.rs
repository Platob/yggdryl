//! One test file per file under `rust/src/xml/`, under `tests/xml/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the XML codec's
//! own files. A test reaches the crate through `yggdryl::`; where what it pins
//! is not reachable that way, it reaches `yggdryl::internals`, which exists
//! only under the `internals` feature, and the file that reaches it is
//! declared behind that feature here.

#[path = "xml/element.rs"]
mod element;
#[path = "xml/mod_.rs"]
mod mod_;
#[path = "xml/parser.rs"]
mod parser;
#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "xml/scanner.rs"]
mod scanner;
#[path = "xml/wire.rs"]
mod wire;
