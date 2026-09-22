//! One test file per file under `rust/src/serie/`, mirrored file for file.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the column
//! leaves and the Arrow door. A test reaches the crate through `yggdryl::`;
//! the buffer edits the leaves share are reached through
//! `yggdryl::internals::serie_layout`, which exists only under the
//! `internals` feature, and that file is declared behind it here.

#[path = "serie/arrow.rs"]
mod arrow;
#[path = "serie/boolean.rs"]
mod boolean;
#[path = "serie/bytes.rs"]
mod bytes;
#[path = "serie/enums.rs"]
mod enums;
#[cfg(feature = "internals")]
#[path = "serie/layout.rs"]
mod layout;
#[path = "serie/mapping.rs"]
mod mapping;
#[path = "serie/null.rs"]
mod null;
#[path = "serie/primitive.rs"]
mod primitive;
#[path = "serie/runend.rs"]
mod runend;
#[path = "serie/sequence.rs"]
mod sequence;
#[path = "serie/string.rs"]
mod string;
#[path = "serie/structure.rs"]
mod structure;
#[path = "serie/union.rs"]
mod union;
#[path = "serie/variant.rs"]
mod variant;
