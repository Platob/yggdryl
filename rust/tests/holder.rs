//! Storage-handle integration tests.

#[path = "support/counting.rs"]
mod counting;
#[cfg(feature = "s3")]
#[path = "support/server.rs"]
mod server;

#[path = "holder/buffer.rs"]
mod buffer;
#[path = "holder/buffered/mod_.rs"]
mod buffered;
#[path = "holder/mod_.rs"]
mod mod_;
