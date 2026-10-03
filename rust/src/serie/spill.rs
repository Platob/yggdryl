//! What a serie answers about where its rows live, and the one verb that
//! moves them to disk: [`Serie::resident_size`], [`Serie::is_spilled`],
//! [`Serie::spill`], and the crate-private [`Serie::settled`] every door that
//! lays a column out itself passes through.
//!
//! A spill is greedy and settles the heaviest leaves first: a serie under
//! the bound is untouched, a flat leaf over it spills whole, and a nested
//! leaf walks its children heaviest first, each under what the bound leaves
//! once the rest is counted, stopping as soon as the serie is under; a
//! nested leaf whose own buffers alone pass the bound spills whole, its
//! mapped children rewritten into the new file. A run holds values and never
//! buffers, so it is never spilled and spills nothing. The buffers come back
//! mapped read-only and every read reaches them where they lie; a failure
//! names the folder and leaves the serie as it was.

use std::sync::Arc;

use super::{Proof, Serie, column, column_mut, land};
use crate::spill::{Backing, SpillOptions, spill_array};
use crate::value::SerieValue;
use crate::{Error, Result};

impl Serie {
    /// The bytes the rows occupy in memory: [`Self::memory_size`] less what
    /// lies in a spill file's mapping.
    ///
    /// A run's values are all resident, a flat column's buffers are all on
    /// the heap or all mapped, and a nested column counts its own buffers
    /// beside its children's answer. Read off the buffers: nothing is
    /// allocated to count them.
    ///
    /// ```
    /// use yggdryl::{Scalar, Serie};
    ///
    /// let prices = Serie::new(vec![Scalar::from(1_i64), Scalar::from(2_i64)]);
    /// assert_eq!(prices.resident_size(), prices.memory_size());
    /// assert!(!prices.is_spilled());
    /// ```
    pub fn resident_size(&self) -> usize {
        column!(
            self,
            run => Self::Run(run.clone()).memory_size(),
            column => SerieValue::resident_size(column.as_ref())
        )
    }

    /// Whether the rows lie in a spill file: some bytes, none of them
    /// resident. A run and an empty column are never spilled.
    pub fn is_spilled(&self) -> bool {
        column!(
            self,
            _run => false,
            column => SerieValue::is_spilled(column.as_ref())
        )
    }

    /// Move the rows to disk until the resident bytes are under
    /// `options`' bound, answering nothing when they already are.
    ///
    /// The heaviest leaves spill first: a flat column whole, a nested one
    /// child by child under what the bound leaves for it once the rest is
    /// counted, and whole where its own buffers alone pass the bound. Every
    /// buffer is written once to a private file under
    /// [`SpillOptions::folder`] and mapped back read-only, so every later
    /// read, slice, cast and Arrow export of this serie reaches the mapping
    /// and copies nothing; a write copies the buffer it touches back to the
    /// heap once. A clone taken before the spill keeps its heap bytes until
    /// it drops; a run spills nothing; a bound of zero spills everything and
    /// [`SpillOptions::NEVER`] nothing.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, SpillOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = DataType::Int64.required_field("price");
    /// let mut prices = Serie::from_scalars(field, (0..1_024_i64).map(Scalar::from))?;
    /// let before = prices.clone();
    /// prices.spill(&SpillOptions::new().with_byte_size(0))?;
    /// assert!(prices.is_spilled());
    /// assert_eq!(prices.resident_size(), 0);
    /// assert_eq!(prices, before);
    /// assert_eq!(prices.scalar(7)?, Scalar::from(7_i64));
    /// // A write brings the rows it touches back to the heap, once.
    /// prices.push(Scalar::from(1_024_i64))?;
    /// assert!(!prices.is_spilled());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the folder when the spill file cannot be
    /// created, written or mapped, leaving the serie as it was.
    pub fn spill(&mut self, options: &SpillOptions) -> Result<()> {
        if options.is_never() {
            return Ok(());
        }
        self.spill_under(options.byte_size(), options)
    }

    /// [`Self::spill`], answering this serie so calls chain.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, SpillOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = DataType::Int64.required_field("price");
    /// let mut prices = Serie::from_scalars(field, (0..1_024_i64).map(Scalar::from))?;
    /// assert!(prices.as_spilled(&SpillOptions::new().with_byte_size(0))?.is_spilled());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::spill`]'s.
    pub fn as_spilled(&mut self, options: &SpillOptions) -> Result<&mut Self> {
        self.spill(options)?;
        Ok(self)
    }

    /// A copy of this serie spilled under `options`' bound, this one
    /// untouched: the buffers the bound leaves resident are shared, the
    /// rest written once and mapped.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, Serie, SpillOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = DataType::Int64.required_field("price");
    /// let prices = Serie::from_scalars(field, (0..1_024_i64).map(Scalar::from))?;
    /// let spilled = prices.into_spilled(&SpillOptions::new().with_byte_size(0))?;
    /// assert!(spilled.is_spilled() && !prices.is_spilled());
    /// assert_eq!(spilled, prices);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Self::spill`]'s.
    pub fn into_spilled(&self, options: &SpillOptions) -> Result<Self> {
        let mut spilled = self.clone();
        spilled.spill(options)?;
        Ok(spilled)
    }

    /// This serie settled under the process default: spilled where it passes
    /// [`SpillOptions::from_env`]'s bound, untouched otherwise.
    ///
    /// What every door the crate lays a column out at answers through, so
    /// a column the crate built is never held resident past the bound. A
    /// door that lands a caller's array or batch sharing its buffers never
    /// settles, and neither does a [`SerieReader`](crate::SerieReader)
    /// batch in flight, cast or not: a stream's bound is its batch size,
    /// and the batch is the caller's to hold or drop. A door that drains a
    /// stream into held chunks settles each chunk it keeps.
    ///
    /// # Errors
    ///
    /// [`SpillOptions::from_env`]'s refusal of the environment, and
    /// [`Self::spill`]'s.
    pub(crate) fn settled(mut self) -> Result<Self> {
        self.settle()?;
        Ok(self)
    }

    /// [`Self::settled`] in place: what a door that grew this serie where it
    /// stands answers through.
    ///
    /// # Errors
    ///
    /// [`Self::settled`]'s.
    pub(crate) fn settle(&mut self) -> Result<()> {
        self.spill(SpillOptions::from_env()?)
    }

    /// Spill this serie until its resident bytes are under `budget`: whole
    /// where it is flat or its own buffers alone pass the budget, child by
    /// child, heaviest first, otherwise.
    fn spill_under(&mut self, budget: u64, options: &SpillOptions) -> Result<()> {
        if self.as_run().is_some() {
            return Ok(());
        }
        let resident = bytes(self.resident_size());
        if resident <= budget {
            return Ok(());
        }
        if let Self::Lit(lit) = self {
            // A constant's whole content is its value: the built array is
            // what it may forget, and nothing of it is written to disk.
            Arc::make_mut(lit).forget_built();
            return Ok(());
        }
        let children = child_count(self);
        if children == 0 || own_resident(self) > budget {
            return spill_whole(self, options);
        }
        // Heaviest child first: each spills under what the budget leaves
        // once everything else that stays resident is counted.
        let mut order: Vec<(usize, u64)> = (0..children)
            .map(|index| (index, child_resident(self, index)))
            .collect();
        order.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        let mut remaining = resident;
        for (index, child_bytes) in order {
            if remaining <= budget {
                break;
            }
            let others = remaining - child_bytes;
            let child_budget = budget.saturating_sub(others);
            let name = self.name().to_owned();
            let child = child_mut(self, index).ok_or_else(|| Self::spill_mismatch(&name))?;
            child.spill_under(child_budget, options)?;
            remaining = others + child_resident(self, index);
        }
        Ok(())
    }

    /// State where this serie's buffers live, every nested child included;
    /// a run holds no buffer and is left alone.
    pub(crate) fn set_backing(&mut self, backing: Backing) {
        column_mut!(
            self,
            _run => {},
            column => column.set_backing(backing)
        );
    }

    /// Refuse a leaf that did not come back as itself from a spill, naming
    /// the column: the one conflict a re-narrowing can meet.
    pub(crate) fn spill_mismatch(name: &str) -> Error {
        Error::conflict("the spilled leaf", "another leaf", name)
    }
}

/// A byte count as the bound's width.
fn bytes(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Write every buffer of `serie` to one spill file and land the same column
/// over the mapping, in place.
fn spill_whole(serie: &mut Serie, options: &SpillOptions) -> Result<()> {
    let Some(field) = serie.field_ref() else {
        return Ok(());
    };
    let array = serie.require_arrow_array()?;
    let Some((mapped, mapping)) = spill_array(&array, options.folder())? else {
        return Ok(());
    };
    let mut landed = land(Arc::clone(field), mapped, &Proof::Proven)?;
    drop(mapping);
    landed.set_backing(Backing::Mapped);
    *serie = landed;
    Ok(())
}

/// How many child columns a nested leaf spills one by one: a record's and
/// a union's members, a sequence's items, a mapping's entries, an
/// encoding's keys and values or run ends and values; none for a flat leaf.
fn child_count(serie: &Serie) -> usize {
    match serie {
        Serie::Struct(held) => held.children().len(),
        Serie::Union(held) => held.children().len(),
        Serie::Serie(_)
        | Serie::LargeSerie(_)
        | Serie::SerieView(_)
        | Serie::LargeSerieView(_)
        | Serie::FixedSizeSerie(_)
        | Serie::Map(_)
        | Serie::SortedMap(_) => 1,
        Serie::Dictionary(_) | Serie::RunEndEncoded(_) => 2,
        _ => 0,
    }
}

/// Child `index` of a nested leaf, to write: the leaf struct copied once
/// where it is shared, its buffers untouched.
fn child_mut(serie: &mut Serie, index: usize) -> Option<&mut Serie> {
    match serie {
        Serie::Struct(held) => Arc::make_mut(held).children_mut().get_mut(index),
        Serie::Union(held) => Arc::make_mut(held).children_mut().get_mut(index),
        Serie::Serie(held) => (index == 0).then(|| Arc::make_mut(held).items_mut()),
        Serie::LargeSerie(held) => (index == 0).then(|| Arc::make_mut(held).items_mut()),
        Serie::SerieView(held) => (index == 0).then(|| Arc::make_mut(held).items_mut()),
        Serie::LargeSerieView(held) => (index == 0).then(|| Arc::make_mut(held).items_mut()),
        Serie::FixedSizeSerie(held) => (index == 0).then(|| Arc::make_mut(held).items_mut()),
        Serie::Map(held) | Serie::SortedMap(held) => {
            (index == 0).then(|| Arc::make_mut(held).entries_mut())
        }
        Serie::Dictionary(held) => match index {
            0 => Some(Arc::make_mut(held).keys_mut()),
            1 => Some(Arc::make_mut(held).values_mut()),
            _ => None,
        },
        Serie::RunEndEncoded(held) => match index {
            0 => Some(Arc::make_mut(held).run_ends_mut()),
            1 => Some(Arc::make_mut(held).values_mut()),
            _ => None,
        },
        _ => None,
    }
}

/// The resident bytes of child `index` of a nested leaf; zero for none.
fn child_resident(serie: &Serie, index: usize) -> u64 {
    child(serie, index).map_or(0, |child| bytes(child.resident_size()))
}

/// Child `index` of a nested leaf, to read; `None` past the children or on
/// a flat leaf.
fn child(serie: &Serie, index: usize) -> Option<&Serie> {
    match serie {
        Serie::Struct(held) => held.children().get(index),
        Serie::Union(held) => held.children().get(index),
        Serie::Serie(_)
        | Serie::LargeSerie(_)
        | Serie::SerieView(_)
        | Serie::LargeSerieView(_)
        | Serie::FixedSizeSerie(_)
        | Serie::Map(_)
        | Serie::SortedMap(_) => (index == 0).then(|| serie.items()).flatten(),
        Serie::Dictionary(held) => match index {
            0 => Some(held.keys()),
            1 => Some(held.values()),
            _ => None,
        },
        Serie::RunEndEncoded(held) => match index {
            0 => Some(held.run_ends()),
            1 => Some(held.values()),
            _ => None,
        },
        _ => None,
    }
}

/// The resident bytes of a leaf's own level: a flat leaf's every buffer, a
/// nested leaf's own buffers alone - zero where they are mapped.
fn own_resident(serie: &Serie) -> u64 {
    let (own, mapped) = match serie {
        Serie::Struct(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::Union(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::Serie(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::LargeSerie(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::SerieView(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::LargeSerieView(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::FixedSizeSerie(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::Map(held) | Serie::SortedMap(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::Dictionary(held) => (held.own_size(), held.backing().is_mapped()),
        Serie::RunEndEncoded(held) => (held.own_size(), held.backing().is_mapped()),
        flat => return bytes(flat.resident_size()),
    };
    if mapped { 0 } else { bytes(own) }
}
