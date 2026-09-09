//! Where each data record starts, under a fixed byte budget.
//!
//! A CSV record has no fixed width, so reading row *n* means knowing where it
//! begins. Holding every offset would cost eight bytes a row - eight hundred
//! megabytes for a hundred million rows - so the index is sparse: it holds one
//! anchor every `stride` rows and doubles the stride whenever it would exceed
//! [`MAX_ANCHOR_SIZE`] anchors. Memory is therefore flat at 32 KiB whatever the
//! resource's size, and reaching a row costs one positional read plus at most
//! `stride - 1` records of forward scanning.

use crate::{IOBase, Result};

use super::reader::Records;
use super::scan::Dialect;

/// The most anchors an index holds before it halves its resolution.
///
/// Eight bytes each, so the index never exceeds 32 KiB. The bound is on the
/// index, not on the resource: a bigger resource gets a coarser stride, never
/// a bigger index.
const MAX_ANCHOR_SIZE: usize = 4096;

/// A sparse map from row number to decoded byte offset.
#[derive(Clone, Debug)]
pub(crate) struct RowIndex {
    anchors: Vec<u64>,
    stride: u64,
    rows: u64,
}

impl RowIndex {
    /// Scan the resource once, recording one anchor every `stride` rows.
    pub(crate) fn build(
        handle: &(impl IOBase + ?Sized),
        dialect: &Dialect,
        data_start: u64,
    ) -> Result<Self> {
        let source = crate::media::stream::decoded_reader_at(handle, data_start)?;
        let mut records = Records::new(source, dialect.clone(), data_start);
        let mut index = Self {
            anchors: Vec::new(),
            stride: 1,
            rows: 0,
        };
        while let Some(record) = records.next_record() {
            let record = record?;
            index.observe(record.offset);
            index.rows += 1;
        }
        Ok(index)
    }

    /// Return how many data records the resource holds.
    pub(crate) const fn rows(&self) -> u64 {
        self.rows
    }

    /// Return the nearest anchor at or before `row`, and the rows to skip.
    ///
    /// `None` means the resource holds no such row.
    pub(crate) fn anchor(&self, row: u64) -> Option<(u64, u64)> {
        if row >= self.rows {
            return None;
        }
        let slot = usize::try_from(row / self.stride).ok()?;
        let offset = *self.anchors.get(slot)?;
        Some((offset, row % self.stride))
    }

    /// Record `offset` when the row it starts is an anchor row.
    fn observe(&mut self, offset: u64) {
        if self.rows % self.stride != 0 {
            return;
        }
        if self.anchors.len() == MAX_ANCHOR_SIZE {
            // Halve the resolution in place: every second anchor still names
            // the row it named, now at twice the stride.
            let mut kept = 0;
            for slot in 0..self.anchors.len() {
                if slot % 2 == 0 {
                    self.anchors[kept] = self.anchors[slot];
                    kept += 1;
                }
            }
            self.anchors.truncate(kept);
            self.stride *= 2;
            // The row that triggered the compaction may no longer be an anchor.
            if self.rows % self.stride != 0 {
                return;
            }
        }
        self.anchors.push(offset);
    }
}
