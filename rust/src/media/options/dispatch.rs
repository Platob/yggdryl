use smol_str::SmolStr;

use super::{IORecordOptions, RecordOptions};
use crate::media::ipc::IpcOptions;
use crate::{DataType, Level, Metadata};

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

    fn dtype(&self) -> Option<&DataType> {
        match self {
            Self::Ipc(options) => options.dtype(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.dtype(),
            Self::Avro(options) => options.dtype(),
            Self::Text(options) => options.dtype(),
        }
    }

    fn set_dtype(&mut self, dtype: Option<DataType>) {
        match self {
            Self::Ipc(options) => options.set_dtype(dtype),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_dtype(dtype),
            Self::Avro(options) => options.set_dtype(dtype),
            Self::Text(options) => options.set_dtype(dtype),
        }
    }

    fn metadata(&self) -> &Metadata {
        match self {
            Self::Ipc(options) => options.metadata(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.metadata(),
            Self::Avro(options) => options.metadata(),
            Self::Text(options) => options.metadata(),
        }
    }

    fn set_metadata(&mut self, metadata: Metadata) {
        match self {
            Self::Ipc(options) => options.set_metadata(metadata),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_metadata(metadata),
            Self::Avro(options) => options.set_metadata(metadata),
            Self::Text(options) => options.set_metadata(metadata),
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

    fn batch_row_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.batch_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_row_size(),
            Self::Avro(options) => options.batch_row_size(),
            Self::Text(options) => options.batch_row_size(),
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

    fn merge_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.merge_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.merge_by_names(),
            Self::Avro(options) => options.merge_by_names(),
            Self::Text(options) => options.merge_by_names(),
        }
    }

    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_merge_by_names(merge_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_merge_by_names(merge_by_names),
            Self::Avro(options) => options.set_merge_by_names(merge_by_names),
            Self::Text(options) => options.set_merge_by_names(merge_by_names),
        }
    }

    fn select_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.select_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.select_by_names(),
            Self::Avro(options) => options.select_by_names(),
            Self::Text(options) => options.select_by_names(),
        }
    }

    fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_select_by_names(select_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_select_by_names(select_by_names),
            Self::Avro(options) => options.set_select_by_names(select_by_names),
            Self::Text(options) => options.set_select_by_names(select_by_names),
        }
    }

    fn filter_partitions(&self) -> &[(String, String)] {
        match self {
            Self::Ipc(options) => options.filter_partitions(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.filter_partitions(),
            Self::Avro(options) => options.filter_partitions(),
            Self::Text(options) => options.filter_partitions(),
        }
    }

    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        match self {
            Self::Ipc(options) => options.set_filter_partitions(filter_partitions),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_filter_partitions(filter_partitions),
            Self::Avro(options) => options.set_filter_partitions(filter_partitions),
            Self::Text(options) => options.set_filter_partitions(filter_partitions),
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
