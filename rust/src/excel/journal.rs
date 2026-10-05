//! Undo and redo: the edits a workbook took, each beside the edit undoing
//! it, bounded in entries and in bytes.
//!
//! [`Journal::record`] takes an applied edit's inverse out of its
//! [`Applied`] - the rest of the answer stays the caller's - clearing what
//! could be redone; [`Journal::undo`] applies the newest inverse and keeps the
//! inverse of that for [`Journal::redo`] - so a redo gives back exactly
//! what the edit made, a sheet it added under the key it had - and a redo
//! keeps the inverse of itself to undo again. The oldest entries go first
//! once either bound is passed, and an edit whose inverse alone passes the
//! byte bound is not undoable: it clears the journal, as an edit Excel
//! cannot undo clears its undo list.

use std::collections::VecDeque;

use smol_str::SmolStr;

use crate::Result;

use super::edit::{Applied, Edit};
use super::workbook::Workbook;

/// How many edits the workbook service keeps to undo.
pub const DEFAULT_JOURNAL_ENTRIES: usize = 100;

/// How many bytes of inverses the workbook service keeps.
pub const DEFAULT_JOURNAL_BYTES: usize = 64 * 1024 * 1024;

/// One edit kept: the edit undoing it - or redoing it - what the menus
/// call it, and the bytes the edit kept holds.
#[derive(Debug)]
struct Entry {
    edit: Edit,
    label: SmolStr,
    bytes: usize,
}

/// The edits a workbook can undo and redo.
///
/// Held state: at most `entries` edits each way and their inverses,
/// together at most `bytes` by [`Edit::byte_size`]'s estimate; the reason
/// is Undo itself.
///
/// ```
/// use yggdryl::excel::{Edit, Journal, Workbook};
/// use yggdryl::Scalar;
///
/// let mut workbook = Workbook::new();
/// workbook.add_sheet("Sheet1")?;
/// let mut journal = Journal::new(100, 1 << 20);
/// let edit = Edit::SetEntries { sheet: "Sheet1".into(), entries: vec![("A1".parse()?, "42".into())] };
/// let label = edit.label();
/// let mut applied = workbook.apply(edit)?;
/// assert!(journal.record(label, &mut applied));
/// // What the edit touched stays with the caller, the inverse with the journal.
/// assert_eq!(applied.touched.len(), 1);
/// assert!(applied.inverse.is_none());
/// assert_eq!(journal.labels(), (Some("Typing '42' in A1"), None));
/// journal.undo(&mut workbook)?;
/// assert_eq!(workbook.sheet("Sheet1")?.scalar("A1".parse()?), Scalar::Null);
/// journal.redo(&mut workbook)?;
/// assert_eq!(workbook.sheet("Sheet1")?.scalar("A1".parse()?), Scalar::from(42.0));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Debug)]
pub struct Journal {
    undo: VecDeque<Entry>,
    redo: Vec<Entry>,
    entries: usize,
    bytes: usize,
}

impl Journal {
    /// A journal keeping at most `entries` edits and `bytes` of inverses.
    #[must_use]
    pub fn new(entries: usize, bytes: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            entries,
            bytes,
        }
    }

    /// Keep the edit `applied` answers for - which the menus call `label`,
    /// [`Edit::label`] of the edit before it was applied - to undo, taking
    /// its inverse out of `applied` and leaving the rest of the answer with
    /// the caller; answers whether it can be undone. An edit with no
    /// inverse, or one whose inverse alone holds more than the journal's
    /// bytes, clears the journal: nothing before it can be undone past it.
    pub fn record(&mut self, label: impl Into<SmolStr>, applied: &mut Applied) -> bool {
        self.redo.clear();
        let Some(inverse) = applied.inverse.take() else {
            self.clear();
            return false;
        };
        self.push(
            Entry {
                label: label.into(),
                edit: inverse,
                bytes: applied.bytes,
            },
            false,
        )
    }

    /// Keep the newest inverse, trimming only the far ends of reachable
    /// history. Undo and redo share one byte budget. An inverse exceeding
    /// that budget ends both histories after the successful workbook edit.
    fn push(&mut self, entry: Entry, redo: bool) -> bool {
        if self.entries == 0 || entry.bytes > self.bytes {
            self.clear();
            return false;
        }
        if redo {
            self.redo.push(entry);
        } else {
            self.undo.push_back(entry);
        }
        while self.undo.len() > self.entries
            || self.redo.len() > self.entries
            || !self.within_byte_bound()
        {
            if self.undo.len() > self.entries
                || (self.redo.len() <= self.entries && self.undo.len() > usize::from(!redo))
            {
                self.undo.pop_front();
            } else {
                // Vec's last entry is the next redo. Removing its first
                // discards the farthest future without leaving a gap.
                self.redo.remove(0);
            }
        }
        true
    }

    /// Subtract from the bound rather than summing potentially large
    /// estimates. This allocates nothing and cannot overflow.
    fn within_byte_bound(&self) -> bool {
        self.undo
            .iter()
            .chain(&self.redo)
            .try_fold(self.bytes, |remaining, entry| {
                remaining.checked_sub(entry.bytes)
            })
            .is_some()
    }

    /// Undo the newest edit kept, answering what undoing it did; `None` when
    /// there is none.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the inverse, which leaves the workbook as it
    /// was and keeps the edit to undo.
    /// A successful edit whose new inverse exceeds the byte bound clears
    /// both histories; the workbook change remains applied.
    pub fn undo(&mut self, workbook: &mut Workbook) -> Result<Option<Applied>> {
        let Some(entry) = self.undo.pop_back() else {
            return Ok(None);
        };
        match workbook.apply(entry.edit.clone()) {
            Ok(mut applied) => {
                match applied.inverse.take() {
                    Some(edit) => {
                        self.push(
                            Entry {
                                edit,
                                label: entry.label,
                                bytes: applied.bytes,
                            },
                            true,
                        );
                    }
                    None => self.clear(),
                }
                Ok(Some(applied))
            }
            Err(error) => {
                self.undo.push_back(entry);
                Err(error)
            }
        }
    }

    /// Redo the newest edit undone, answering what redoing it did; `None`
    /// when there is none.
    ///
    /// # Errors
    ///
    /// Returns the edit's refusal, which leaves the workbook as it was and
    /// keeps the edit to redo.
    /// A successful edit whose new inverse exceeds the byte bound clears
    /// both histories; the workbook change remains applied.
    pub fn redo(&mut self, workbook: &mut Workbook) -> Result<Option<Applied>> {
        let Some(entry) = self.redo.pop() else {
            return Ok(None);
        };
        let mut applied = match workbook.apply(entry.edit.clone()) {
            Ok(applied) => applied,
            Err(error) => {
                self.redo.push(entry);
                return Err(error);
            }
        };
        match applied.inverse.take() {
            Some(edit) => {
                self.push(
                    Entry {
                        edit,
                        label: entry.label,
                        bytes: applied.bytes,
                    },
                    false,
                );
            }
            None => self.clear(),
        }
        Ok(Some(applied))
    }

    /// What Undo and Redo would do, as their menus call it.
    #[must_use]
    pub fn labels(&self) -> (Option<&str>, Option<&str>) {
        (
            self.undo.back().map(|entry| entry.label.as_str()),
            self.redo.last().map(|entry| entry.label.as_str()),
        )
    }

    /// How many edits can be undone and redone.
    #[must_use]
    pub fn len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// Whether nothing can be undone or redone.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// Forget every edit: what a workbook opened afresh does.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
