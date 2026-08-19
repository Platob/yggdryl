//! Shared, allocation-conscious value enums and protocol vocabulary.

pub(crate) mod codec;
mod datatype_id;
mod datatype_kind;
mod edge_algorithm;
mod io_kind;
mod magic;
mod media_type;
mod mime_type;
mod scheme;
mod time_unit;
pub(crate) mod timezone;
mod union_mode;

pub use codec::{Codec, Encoder, Level};
pub use datatype_id::DataTypeId;
pub use datatype_kind::DataTypeKind;
pub use edge_algorithm::EdgeAlgorithm;
pub use io_kind::IOKind;
pub use magic::MAGIC_PROBE_LEN;
pub use media_type::MediaType;
pub use mime_type::MimeType;
pub use scheme::Scheme;
pub use time_unit::TimeUnit;
pub use timezone::Timezone;
pub use union_mode::UnionMode;
