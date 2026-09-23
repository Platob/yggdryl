//! `IOBase` conformance: what every handle must answer, and what it costs.
//!
//! Any handle reads through one reader and writes through three explicit
//! intents. Held record batches are zero-copy adapters over those primitives.

#[path = "support/counting.rs"]
mod counting;

#[path = "iobase/bytes.rs"]
mod bytes;
#[path = "iobase/lifecycle.rs"]
mod lifecycle;
#[path = "iobase/transfer.rs"]
mod transfer;
