//! `RecordOptions` as the shared settings every medium's options answer, each
//! verb one call through the object-safe contract the held struct implements.

use smol_str::SmolStr;

use super::{IORecordOptions, RecordOptions};
use crate::ipc::IpcOptions;
use crate::{Field, Filter, Level, Selector};

impl IORecordOptions for RecordOptions {
    fn name(&self) -> &str {
        self.as_medium().name()
    }

    fn set_name(&mut self, name: SmolStr) {
        self.as_medium_mut().set_name(name);
    }

    fn declared(&self) -> Option<&Field> {
        self.as_medium().declared()
    }

    fn set_declared(&mut self, field: Option<Field>) {
        self.as_medium_mut().set_declared(field);
    }

    fn merge_by(&self) -> &Selector {
        self.as_medium().merge_by()
    }

    fn set_merge_by(&mut self, merge_by: Selector) {
        self.as_medium_mut().set_merge_by(merge_by);
    }

    fn filter(&self) -> &Filter {
        self.as_medium().filter()
    }

    fn set_filter(&mut self, filter: Filter) {
        self.as_medium_mut().set_filter(filter);
    }

    fn select(&self) -> &Selector {
        self.as_medium().select()
    }

    fn set_select(&mut self, select: Selector) {
        self.as_medium_mut().set_select(select);
    }

    fn safe(&self) -> bool {
        self.as_medium().safe()
    }

    fn set_safe(&mut self, safe: bool) {
        self.as_medium_mut().set_safe(safe);
    }

    fn batch_byte_size(&self) -> Option<u64> {
        self.as_medium().batch_byte_size()
    }

    fn batch_row_size(&self) -> Option<usize> {
        self.as_medium().batch_row_size()
    }

    fn set_batch_byte_size(&mut self, batch_byte_size: Option<u64>) {
        self.as_medium_mut().set_batch_byte_size(batch_byte_size);
    }

    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>) {
        self.as_medium_mut().set_batch_row_size(batch_row_size);
    }

    fn max_row_size(&self) -> Option<u64> {
        self.as_medium().max_row_size()
    }

    fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
        self.as_medium_mut().set_max_row_size(max_row_size);
    }

    fn row_offset(&self) -> Option<u64> {
        self.as_medium().row_offset()
    }

    fn set_row_offset(&mut self, row_offset: Option<u64>) {
        self.as_medium_mut().set_row_offset(row_offset);
    }

    fn max_byte_size(&self) -> Option<u64> {
        self.as_medium().max_byte_size()
    }

    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
        self.as_medium_mut().set_max_byte_size(max_byte_size);
    }

    fn commit_batch_num(&self) -> Option<usize> {
        self.as_medium().commit_batch_num()
    }

    fn set_commit_batch_num(&mut self, commit_batch_num: Option<usize>) {
        self.as_medium_mut().set_commit_batch_num(commit_batch_num);
    }

    fn num_threads(&self) -> Option<usize> {
        self.as_medium().num_threads()
    }

    fn set_num_threads(&mut self, num_threads: Option<usize>) {
        self.as_medium_mut().set_num_threads(num_threads);
    }

    fn level(&self) -> Level {
        self.as_medium().level()
    }

    fn set_level(&mut self, level: Level) {
        self.as_medium_mut().set_level(level);
    }
}

impl From<IpcOptions> for RecordOptions {
    fn from(value: IpcOptions) -> Self {
        Self::Ipc(value)
    }
}

impl From<crate::text::TextOptions> for RecordOptions {
    fn from(value: crate::text::TextOptions) -> Self {
        Self::Text(Box::new(value))
    }
}

impl From<crate::xmla::XmlaOptions> for RecordOptions {
    fn from(value: crate::xmla::XmlaOptions) -> Self {
        Self::Xmla(value)
    }
}

impl From<crate::csv::CsvOptions> for RecordOptions {
    fn from(value: crate::csv::CsvOptions) -> Self {
        Self::Csv(value)
    }
}
