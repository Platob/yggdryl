//! `rust/src/s3/google/mod.rs`: the files the Google dialect owns,
//! declared together.

#[cfg(all(feature = "s3", feature = "internals"))]
#[path = "token.rs"]
mod token;
