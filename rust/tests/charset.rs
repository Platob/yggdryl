//! Character-encoding integration tests.

#[path = "charset/codecs.rs"]
mod codecs;
#[path = "charset/handles.rs"]
mod handles;
#[cfg(feature = "internals")]
#[path = "charset/reader.rs"]
mod reader;
#[path = "charset/records.rs"]
mod records;
#[path = "charset/vocabulary.rs"]
mod vocabulary;
