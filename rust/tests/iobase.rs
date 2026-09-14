//! `IOBase` conformance: what every handle must answer, and what it costs.

#[path = "support/counting.rs"]
mod counting;

#[path = "iobase/applying.rs"]
mod applying;
#[path = "iobase/buffered_handle.rs"]
mod buffered_handle;
#[path = "iobase/conformance.rs"]
mod conformance;
#[path = "iobase/cursor.rs"]
mod cursor;
#[path = "iobase/laziness.rs"]
mod laziness;
#[path = "iobase/lifecycle.rs"]
mod lifecycle;
#[path = "iobase/positional.rs"]
mod positional;
/// Any handle reads through one reader and writes through three explicit
/// intents. Held record batches are zero-copy adapters over those primitives.
#[cfg(feature = "arrow")]
#[path = "iobase/records/mod.rs"]
mod records;
#[path = "iobase/shape.rs"]
mod shape;
