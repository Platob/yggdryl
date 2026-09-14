use smol_str::SmolStr;

use super::{IORecordOptions, RecordOptions};
use crate::media::ipc::IpcOptions;
use crate::{Field, Filter, Level, Selector};

impl IORecordOptions for RecordOptions {
    fn name(&self) -> &str {
        match self {
            Self::Ipc(options) => options.name(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.name(),
            Self::Avro(options) => options.name(),
            Self::Text(options) => options.name(),
        }
    }

    fn set_name(&mut self, name: SmolStr) {
        match self {
            Self::Ipc(options) => options.set_name(name),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_name(name),
            Self::Avro(options) => options.set_name(name),
            Self::Text(options) => options.set_name(name),
        }
    }

    fn declared(&self) -> Option<&Field> {
        match self {
            Self::Ipc(options) => options.declared(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.declared(),
            Self::Avro(options) => options.declared(),
            Self::Text(options) => options.declared(),
        }
    }

    fn set_declared(&mut self, field: Option<Field>) {
        match self {
            Self::Ipc(options) => options.set_declared(field),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_declared(field),
            Self::Avro(options) => options.set_declared(field),
            Self::Text(options) => options.set_declared(field),
        }
    }

    fn merge_by(&self) -> &Selector {
        match self {
            Self::Ipc(options) => options.merge_by(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.merge_by(),
            Self::Avro(options) => options.merge_by(),
            Self::Text(options) => options.merge_by(),
        }
    }

    fn set_merge_by(&mut self, merge_by: Selector) {
        match self {
            Self::Ipc(options) => options.set_merge_by(merge_by),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_merge_by(merge_by),
            Self::Avro(options) => options.set_merge_by(merge_by),
            Self::Text(options) => options.set_merge_by(merge_by),
        }
    }

    fn filter(&self) -> &Filter {
        match self {
            Self::Ipc(options) => options.filter(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.filter(),
            Self::Avro(options) => options.filter(),
            Self::Text(options) => options.filter(),
        }
    }

    fn set_filter(&mut self, filter: Filter) {
        match self {
            Self::Ipc(options) => options.set_filter(filter),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_filter(filter),
            Self::Avro(options) => options.set_filter(filter),
            Self::Text(options) => options.set_filter(filter),
        }
    }

    fn selector(&self) -> &Selector {
        match self {
            Self::Ipc(options) => options.selector(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.selector(),
            Self::Avro(options) => options.selector(),
            Self::Text(options) => options.selector(),
        }
    }

    fn set_selector(&mut self, selector: Selector) {
        match self {
            Self::Ipc(options) => options.set_selector(selector),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_selector(selector),
            Self::Avro(options) => options.set_selector(selector),
            Self::Text(options) => options.set_selector(selector),
        }
    }

    fn safe(&self) -> bool {
        match self {
            Self::Ipc(options) => options.safe(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.safe(),
            Self::Avro(options) => options.safe(),
            Self::Text(options) => options.safe(),
        }
    }

    fn set_safe(&mut self, safe: bool) {
        match self {
            Self::Ipc(options) => options.set_safe(safe),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_safe(safe),
            Self::Avro(options) => options.set_safe(safe),
            Self::Text(options) => options.set_safe(safe),
        }
    }

    fn batch_byte_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.batch_byte_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_byte_size(),
            Self::Avro(options) => options.batch_byte_size(),
            Self::Text(options) => options.batch_byte_size(),
        }
    }

    fn batch_row_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.batch_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_row_size(),
            Self::Avro(options) => options.batch_row_size(),
            Self::Text(options) => options.batch_row_size(),
        }
    }

    fn set_batch_byte_size(&mut self, batch_byte_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_batch_byte_size(batch_byte_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_batch_byte_size(batch_byte_size),
            Self::Avro(options) => options.set_batch_byte_size(batch_byte_size),
            Self::Text(options) => options.set_batch_byte_size(batch_byte_size),
        }
    }

    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_batch_row_size(batch_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_batch_row_size(batch_row_size),
            Self::Avro(options) => options.set_batch_row_size(batch_row_size),
            Self::Text(options) => options.set_batch_row_size(batch_row_size),
        }
    }

    fn max_row_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_row_size(),
            Self::Avro(options) => options.max_row_size(),
            Self::Text(options) => options.max_row_size(),
        }
    }

    fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_row_size(max_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_row_size(max_row_size),
            Self::Avro(options) => options.set_max_row_size(max_row_size),
            Self::Text(options) => options.set_max_row_size(max_row_size),
        }
    }

    fn max_byte_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_byte_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_byte_size(),
            Self::Avro(options) => options.max_byte_size(),
            Self::Text(options) => options.max_byte_size(),
        }
    }

    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_byte_size(max_byte_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_byte_size(max_byte_size),
            Self::Avro(options) => options.set_max_byte_size(max_byte_size),
            Self::Text(options) => options.set_max_byte_size(max_byte_size),
        }
    }

    fn commit_row_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.commit_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.commit_row_size(),
            Self::Avro(options) => options.commit_row_size(),
            Self::Text(options) => options.commit_row_size(),
        }
    }

    fn set_commit_row_size(&mut self, commit_row_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_commit_row_size(commit_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_commit_row_size(commit_row_size),
            Self::Avro(options) => options.set_commit_row_size(commit_row_size),
            Self::Text(options) => options.set_commit_row_size(commit_row_size),
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::Ipc(options) => options.level(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.level(),
            Self::Avro(options) => options.level(),
            Self::Text(options) => options.level(),
        }
    }

    fn set_level(&mut self, level: Level) {
        match self {
            Self::Ipc(options) => options.set_level(level),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_level(level),
            Self::Avro(options) => options.set_level(level),
            Self::Text(options) => options.set_level(level),
        }
    }
}

impl From<IpcOptions> for RecordOptions {
    fn from(value: IpcOptions) -> Self {
        Self::Ipc(value)
    }
}

#[cfg(feature = "parquet")]
impl From<crate::media::parquet::ParquetOptions> for RecordOptions {
    fn from(value: crate::media::parquet::ParquetOptions) -> Self {
        Self::Parquet(value)
    }
}

impl From<crate::media::avro::AvroOptions> for RecordOptions {
    fn from(value: crate::media::avro::AvroOptions) -> Self {
        Self::Avro(value)
    }
}

impl From<crate::media::text::TextOptions> for RecordOptions {
    fn from(value: crate::media::text::TextOptions) -> Self {
        Self::Text(Box::new(value))
    }
}
