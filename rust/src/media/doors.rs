//! The doors a medium implemented outside the crate answers
//! [`IOMedia`](crate::IOMedia) through: what the core's own wrappers call.
//!
//! `Csv`, `Excel` and the other wrappers implement `IOMedia` over these -
//! the handle's own options where a caller gave none, the options a
//! whole-media dimension is read under, a container's field and row count,
//! the retained record read, and the shaped overwrite, append and merge
//! every leaf write runs - and a registered encoding's wrapper
//! ([`RegisteredMedia`](super::RegisteredMedia)) implements the same methods
//! the same way, so the rules every medium shares - the write modes, the
//! cadence, the limits, the rows an `IOResult` counts - keep one owner.

pub use crate::iobase::{
    append_arrow_reader_default, leaf_writer, merge_arrow_reader_default,
    overwrite_arrow_reader_default_with_field,
};
pub use crate::iomedia::{
    container_field, container_row_size, dimension_options, own_options, read_record_serie,
};
