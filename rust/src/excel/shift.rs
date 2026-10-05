//! The reference adjuster: where every reference a workbook states points
//! once rows or columns open or close, a block of cells moves, or a sheet
//! is renamed or removed.
//!
//! A reference keeps pointing at the same cells, whatever moved: a formula
//! whose host moves re-renders its relative tokens against the new host, a
//! range an insertion falls inside grows, one a removal cuts shrinks, and
//! one a removal takes whole is `#REF!`. Whole rows follow a row edit and
//! ignore a column edit, and the other way round; a span of sheets
//! (`Jan:Mar!B2`) names cells on no one sheet and is not moved by an edit
//! of one. One rule serves every place a reference is written: a cell's
//! formula, a defined name, the ranges and formulas a worksheet carries
//! (conditional formats, validations, hyperlinks, filters, sparklines),
//! and the parts beside a sheet - its tables, drawings, comments and their
//! VML, the charts and pivot caches of the package.
//!
//! A shape is rewritten once per host class - the hosts the edit treats
//! alike - so a shared formula over a million cells costs one rewrite and a
//! million pointer swaps, and each rewrite is checked against the opposite
//! edit: what that edit would not give back is what an undo restores.

use std::collections::HashMap;

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::{CellRange, CellRef, MAX_COLUMNS, MAX_ROWS};
use super::formula::Formula;
use super::formula::reference::{Coord, Reference, SheetSpec, Target, needs_quotes, same_sheet};
use super::formula::shape::Token;
use super::package::{Edits, Tag, edit_document, element_attributes};
use super::table::Table;

/// One axis of the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Axis {
    Rows,
    Columns,
}

impl Axis {
    /// How many rows or columns the grid has.
    pub(crate) const fn limit(self) -> u32 {
        match self {
            Self::Rows => MAX_ROWS,
            Self::Columns => MAX_COLUMNS,
        }
    }

    /// The index of `cell` along the axis.
    pub(crate) const fn of(self, cell: CellRef) -> u32 {
        match self {
            Self::Rows => cell.row(),
            Self::Columns => cell.column(),
        }
    }

    /// `cell` at `index` along the axis.
    pub(crate) const fn with(self, cell: CellRef, index: u32) -> CellRef {
        match self {
            Self::Rows => CellRef::new(index, cell.column()),
            Self::Columns => CellRef::new(cell.row(), index),
        }
    }

    /// The first and last index `range` spans along the axis.
    pub(crate) const fn span(self, range: CellRange) -> (u32, u32) {
        (self.of(range.start()), self.of(range.end()))
    }

    /// The whole rows or columns `start..end` along the axis, which name
    /// one at least.
    pub(crate) const fn whole(self, start: u32, end: u32) -> CellRange {
        match self {
            Self::Rows => CellRange::of_rows(start..end),
            Self::Columns => CellRange::of_columns(start..end),
        }
    }

    /// The axis as a refusal names what it counts.
    pub(crate) const fn noun(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Columns => "columns",
        }
    }
}

/// A band of rows or columns opened or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Band {
    /// `count` new ones before index `at`.
    Insert { at: u32, count: u32 },
    /// The ones from `start` up to `end`, `end` excluded.
    Remove { start: u32, end: u32 },
}

impl Band {
    /// The band that undoes this one.
    pub(crate) const fn inverse(self) -> Self {
        match self {
            Self::Insert { at, count } => Self::Remove {
                start: at,
                end: at + count,
            },
            Self::Remove { start, end } => Self::Insert {
                at: start,
                count: end - start,
            },
        }
    }

    /// Where index `index` stands once the band opened or closed on a grid
    /// of `limit`, `None` when it went: removed, or pushed off the grid.
    pub(crate) fn index(self, index: u32, limit: u32) -> Option<u32> {
        match self {
            Self::Insert { at, count } => {
                if index < at {
                    Some(index)
                } else {
                    index.checked_add(count).filter(|moved| *moved < limit)
                }
            }
            Self::Remove { start, end } => {
                if index < start {
                    Some(index)
                } else if index >= end {
                    Some(index - (end - start))
                } else {
                    None
                }
            }
        }
    }

    /// Where the span `first..=last` stands once the band opened or closed:
    /// an insertion inside it grows it and one at or before its first index
    /// moves it whole, a removal cutting it shrinks it; `None` when it went
    /// whole. A span of the whole axis stays whole.
    pub(crate) fn span(self, first: u32, last: u32, limit: u32) -> Option<(u32, u32)> {
        if first == 0 && last + 1 == limit {
            return Some((first, last));
        }
        match self {
            Self::Insert { at, count } => {
                if first >= at {
                    let moved = first.checked_add(count).filter(|moved| *moved < limit)?;
                    Some((moved, last.saturating_add(count).min(limit - 1)))
                } else if last >= at {
                    Some((first, last.saturating_add(count).min(limit - 1)))
                } else {
                    Some((first, last))
                }
            }
            Self::Remove { start, end } => {
                let count = end - start;
                if last < start {
                    Some((first, last))
                } else if first >= end {
                    Some((first - count, last - count))
                } else if first >= start && last < end {
                    None
                } else {
                    let moved_first = first.min(start);
                    let moved_last = if last >= end { last - count } else { start - 1 };
                    Some((moved_first, moved_last))
                }
            }
        }
    }

    /// Whether the band splits `first..=last`: an insertion strictly inside
    /// it, or a removal taking some of it and not all.
    pub(crate) fn splits(self, first: u32, last: u32) -> bool {
        match self {
            Self::Insert { at, .. } => first < at && at <= last,
            Self::Remove { start, end } => {
                let meets = first < end && last >= start;
                let whole = first >= start && last < end;
                meets && !whole
            }
        }
    }

    /// Whether the band takes or moves anything of `first..=last`: a
    /// removal meeting it.
    pub(crate) const fn meets(self, first: u32, last: u32) -> bool {
        match self {
            Self::Insert { at, .. } => first < at && at <= last,
            Self::Remove { start, end } => first < end && last >= start,
        }
    }

    /// Where a drawing anchor at `index`, `offset` past its start, stands:
    /// one inside a removed band moves to the band's start, its offset
    /// zero.
    pub(crate) fn anchor(self, index: u32, limit: u32) -> (u32, bool) {
        match self.index(index, limit) {
            Some(moved) => (moved, false),
            None => match self {
                Self::Insert { .. } => (limit - 1, true),
                Self::Remove { start, .. } => (start.min(limit - 1), true),
            },
        }
    }

    /// The class of index `index` for the memo: a plain region where the
    /// band only translates it, else the index itself.
    fn class(self, index: i64, limit: u32, key: &mut Key) {
        let plain = match self {
            Self::Insert { at, count } => {
                if index < 0 || index >= i64::from(limit) {
                    None
                } else if index < i64::from(at) {
                    Some(0)
                } else if index + i64::from(count) < i64::from(limit) {
                    Some(1)
                } else {
                    None
                }
            }
            Self::Remove { start, end } => {
                if index < 0 || index >= i64::from(limit) {
                    None
                } else if index < i64::from(start) {
                    Some(0)
                } else if index >= i64::from(end) {
                    Some(2)
                } else {
                    None
                }
            }
        };
        // The edges of the axis decide whether a span is whole.
        match plain.filter(|_| index != 0 && index + 1 != i64::from(limit)) {
            Some(region) => key.push(region),
            None => {
                key.push(3);
                key.push(index as u32);
            }
        }
    }
}

/// A bare name's resolved meaning when its formula leaves its worksheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NameMove {
    /// The source owns it, so a destination formula must name that source.
    Qualify,
    /// Only the destination owns it: a global or missing source binding
    /// must not silently become that destination's local binding.
    Refuse,
}

/// The affected local names for one cross-sheet cut. The workbook resolves
/// scopes once; formula tokens only search this sorted, case-insensitive map.
/// Four ordinary locals fit inline, and a cut without locals allocates none.
#[derive(Debug, Default)]
pub(crate) struct MoveNames(SmallVec<[(SmolStr, NameMove); 4]>);

impl MoveNames {
    /// Whether this cut has no local bindings whose meaning can change.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn new<'n>(names: impl Iterator<Item = (&'n str, NameMove)>) -> Self {
        let mut names: SmallVec<[(SmolStr, NameMove); 4]> = names
            .map(|(name, action)| (SmolStr::new(name), action))
            .collect();
        // Source-local binding wins when both worksheets define the name.
        names.sort_unstable_by(|(a, action_a), (b, action_b)| {
            Self::compare(a, b).then(action_a.cmp(action_b))
        });
        names.dedup_by(|(a, _), (b, _)| a.eq_ignore_ascii_case(b));
        Self(names)
    }

    fn compare(a: &str, b: &str) -> std::cmp::Ordering {
        a.bytes()
            .map(|byte| byte.to_ascii_lowercase())
            .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
    }

    fn action(&self, name: &str) -> Option<NameMove> {
        self.0
            .binary_search_by(|(held, _)| Self::compare(held, name))
            .ok()
            .map(|index| self.0[index].1)
    }
}

/// An edit every reference in a workbook follows.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Shift<'a> {
    /// Rows or columns opened or closed on the sheet `sheet`.
    Band {
        sheet: &'a str,
        axis: Axis,
        band: Band,
    },
    /// The cells of `block` on the sheet `from` moved to where `target`
    /// is their top-left cell on the sheet `to`: what a cut and paste does.
    Move {
        from: &'a str,
        block: CellRange,
        to: &'a str,
        target: CellRef,
        names: &'a MoveNames,
    },
    /// The sheet `from` renamed `to`.
    RenameSheet { from: &'a str, to: &'a str },
    /// The sheet `name` removed, `order` the tabs before the removal.
    RemoveSheet { name: &'a str, order: &'a [&'a str] },
}

impl<'a> Shift<'a> {
    /// Refuse a moved formula before any cells or metadata are published.
    /// Token intake has already happened; neither name lookup nor this
    /// preflight rebuilds a formula or scans the workbook's name registry.
    pub(crate) fn check_names(
        &self,
        formula: &Formula,
        host: &Host<'_>,
        location: impl FnOnce() -> SmolStr,
    ) -> Result<()> {
        let Self::Move {
            from, to, names, ..
        } = *self
        else {
            return Ok(());
        };
        if names.is_empty()
            || same_sheet(from, to)
            || !same_sheet(host.sheet, from)
            || !same_sheet(host.to_sheet, to)
        {
            return Ok(());
        }
        let tokens = &formula.shape().tokens;
        let binder = tokens.iter().find_map(|token| match token {
            Token::Function { name, .. }
                if name.eq_ignore_ascii_case("LET") || name.eq_ignore_ascii_case("LAMBDA") =>
            {
                Some(name)
            }
            _ => None,
        });
        for token in tokens.iter() {
            if let Token::Function { name, .. } = token
                && names.action(name).is_some()
            {
                // Function tokens do not yet distinguish a built-in
                // from a callable defined name. Preserve that binding
                // by refusing, rather than qualifying either blindly.
                return Err(Error::Unsupported {
                    operation: "moving a callable name across worksheet scopes",
                    filesystem: format_smolstr!("{}#{name} from {from} to {to}", location()),
                });
            }
            let Token::Reference(Reference {
                sheet: SheetSpec::Own,
                target: Target::Name(name),
            }) = token
            else {
                continue;
            };
            let Some(action) = names.action(name) else {
                continue;
            };
            let (operation, detail) = if let Some(binder) = binder {
                // The current shape carries lexical bindings verbatim.
                // Qualifying their declarations or bound uses would corrupt
                // the formula; do not guess which occurrence is free.
                (
                    "moving a name with lexical bindings",
                    format_smolstr!("{name} through {binder} to {to}"),
                )
            } else if action == NameMove::Refuse {
                (
                    "moving a locally shadowed name",
                    format_smolstr!("{name} shadowed on {to}"),
                )
            } else {
                continue;
            };
            return Err(Error::Unsupported {
                operation,
                filesystem: format_smolstr!("{}#{detail}", location()),
            });
        }
        Ok(())
    }

    /// The shift that undoes this one, where one exists: a band's opposite,
    /// a rename back.
    pub(crate) const fn inverse(&self) -> Option<Shift<'a>> {
        match *self {
            Self::Band { sheet, axis, band } => Some(Self::Band {
                sheet,
                axis,
                band: band.inverse(),
            }),
            Self::RenameSheet { from, to } => Some(Self::RenameSheet { from: to, to: from }),
            Self::Move { .. } | Self::RemoveSheet { .. } => None,
        }
    }

    /// Where the cell at `at` of the sheet `sheet` stands after the shift,
    /// and on which sheet; `None` when it went: removed, pushed off the grid,
    /// or overwritten by a moved block.
    pub(crate) fn place<'s>(&self, sheet: &'s str, at: CellRef) -> Option<(&'s str, CellRef)>
    where
        'a: 's,
    {
        match *self {
            Self::Band {
                sheet: edited,
                axis,
                band,
            } => {
                if !same_sheet(sheet, edited) {
                    return Some((sheet, at));
                }
                let index = band.index(axis.of(at), axis.limit())?;
                Some((sheet, axis.with(at, index)))
            }
            Self::Move {
                from,
                block,
                to,
                target,
                ..
            } => {
                if same_sheet(sheet, from) && block.contains(at) {
                    return Some((to, translated(at, block, target)?));
                }
                if same_sheet(sheet, to) && block.moved_to(target).contains(at) {
                    return None;
                }
                Some((sheet, at))
            }
            Self::RenameSheet { from, to } => {
                Some((if same_sheet(sheet, from) { to } else { sheet }, at))
            }
            Self::RemoveSheet { name, .. } => (!same_sheet(sheet, name)).then_some((sheet, at)),
        }
    }

    /// Where the cell standing at `at` of the sheet `sheet` after the shift
    /// stood before it, `None` for a cell the shift opened.
    pub(crate) fn origin(&self, sheet: &str, at: CellRef) -> Option<CellRef> {
        match *self {
            Self::Band {
                sheet: edited,
                axis,
                band,
            } => {
                if !same_sheet(sheet, edited) {
                    return Some(at);
                }
                let index = band.inverse().index(axis.of(at), axis.limit())?;
                Some(axis.with(at, index))
            }
            Self::Move {
                block, to, target, ..
            } => {
                let landed = block.moved_to(target);
                if same_sheet(sheet, to) && landed.contains(at) {
                    return translated(at, landed, block.start());
                }
                Some(at)
            }
            Self::RenameSheet { .. } | Self::RemoveSheet { .. } => Some(at),
        }
    }

    /// Whether the text `text` may name a cell the shift moves: it names
    /// the sheet the shift is about, and for a sheet's own ranges, is on
    /// it. A text that cannot is left as it is, never parsed.
    pub(crate) fn may_name(&self, text: &str) -> bool {
        let names = |sheet: &str| contains_name(text, sheet);
        match self {
            Self::Band { sheet, .. } => names(sheet),
            Self::Move { from, to, .. } => names(from) || names(to),
            Self::RenameSheet { from, .. } => names(from),
            Self::RemoveSheet { name, .. } => names(name),
        }
    }

    /// [`Self::may_name`] over the bytes of a part: XML, whose entities -
    /// `P&amp;L` for the sheet `P&L` - are read before the text is looked
    /// at.
    pub(crate) fn may_name_in(&self, bytes: &[u8]) -> bool {
        self.may_name(&unescaped(bytes))
    }

    /// Whether an attribute value or a text of the element `bytes` names a
    /// sheet the shift is about as a reference spells it: the name whole
    /// (`sheet="Data"`), before `!` (`Data!A1`, `'Q1 Data'!A1`) or at an
    /// end of a span (`Jan:Mar!A1`) - where a name's letters inside another
    /// word name nothing. What decides whether an element this crate does
    /// not model refuses the edit; one that does not read is taken to name
    /// the sheet wherever its bytes spell it.
    pub(crate) fn refers_in(&self, bytes: &[u8]) -> bool {
        let mut values = Values::default();
        if edit_document(bytes, &mut values).is_err() {
            return self.may_name_in(bytes);
        }
        let refers = |sheet: &str| values.0.iter().any(|value| refers_to(value, sheet));
        match self {
            Self::Band { sheet, .. } => refers(sheet),
            Self::Move { from, to, .. } => refers(from) || refers(to),
            Self::RenameSheet { from, .. } => refers(from),
            Self::RemoveSheet { name, .. } => refers(name),
        }
    }

    /// Whether the shift moves cells of the sheet `sheet` - which is what
    /// makes the ranges the sheet states about itself move.
    pub(crate) fn moves_cells_of(&self, sheet: &str) -> bool {
        match self {
            Self::Band { sheet: edited, .. } => same_sheet(sheet, edited),
            Self::Move { from, to, .. } => same_sheet(sheet, from) || same_sheet(sheet, to),
            Self::RenameSheet { .. } | Self::RemoveSheet { .. } => false,
        }
    }
}

/// Every attribute value and text of a document.
#[derive(Default)]
struct Values(Vec<String>);

impl Edits for Values {
    fn start(
        &mut self,
        _: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        self.0
            .extend(attributes.iter().map(|(_, value)| value.clone()));
        Ok(Tag::Keep)
    }

    fn text(&mut self, _: &[SmolStr], text: &str) -> Result<Option<String>> {
        self.0.push(text.to_owned());
        Ok(None)
    }
}

/// Whether `value` names the sheet `name` as a reference spells it,
/// compared without case: the name whole, or where a reference to it
/// starts - before `!`, quoted or not, or at an end of a span of sheets.
fn refers_to(value: &str, name: &str) -> bool {
    let value = value.to_lowercase();
    let name = name.to_lowercase();
    if value.trim() == name {
        return true;
    }
    let quoted = name.replace('\'', "''");
    let forms = [
        format!("{name}!"),
        format!("'{quoted}'!"),
        format!("{name}:"),
        format!("'{quoted}:"),
        format!(":{name}!"),
        format!(":{quoted}'!"),
    ];
    forms.iter().any(|form| {
        value.match_indices(form.as_str()).any(|(at, _)| {
            form.starts_with(':')
                || value[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|before| !(before.is_alphanumeric() || matches!(before, '_' | '.')))
        })
    })
}

/// Whether the bytes of a part may name the sheet `name` in a reference,
/// their entities read first ([`Shift::may_name_in`]).
pub(crate) fn names_sheet_in(bytes: &[u8], name: &str) -> bool {
    contains_name(&unescaped(bytes), name)
}

/// `bytes` as text with the five entities XML predefines and every
/// character reference read; any other `&` is kept as it stands. What decides
/// whether a part may name a sheet reads the name as a producer escaped it.
fn unescaped(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains('&') {
        return text;
    }
    let mut read = String::with_capacity(text.len());
    let mut rest: &str = &text;
    while let Some(at) = rest.find('&') {
        read.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| Some((entity_char(&rest[1..end])?, end + 1)));
        match entity {
            Some((character, length)) => {
                read.push(character);
                rest = &rest[length..];
            }
            None => {
                read.push('&');
                rest = &rest[1..];
            }
        }
    }
    read.push_str(rest);
    std::borrow::Cow::Owned(read)
}

/// The character the entity `name` (between `&` and `;`) stands for.
fn entity_char(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Whether `text` holds `name` as a sheet name is written in a reference:
/// as it is, or with its apostrophes doubled, compared without case.
fn contains_name(text: &str, name: &str) -> bool {
    let lower = text.to_lowercase();
    let wanted = name.to_lowercase();
    lower.contains(&wanted)
        || (wanted.contains('\'') && lower.contains(&wanted.replace('\'', "''")))
}

/// `at`, a cell of `block`, where it lands with `target` the block's new
/// top-left cell.
fn translated(at: CellRef, block: CellRange, target: CellRef) -> Option<CellRef> {
    let row = u64::from(target.row()) + u64::from(at.row() - block.start().row());
    let column = u64::from(target.column()) + u64::from(at.column() - block.start().column());
    let moved = CellRef::new(u32::try_from(row).ok()?, u32::try_from(column).ok()?);
    moved.is_in_grid().then_some(moved)
}

/// Where a formula stands: its sheet and cell before the shift, and after.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Host<'a> {
    pub(crate) sheet: &'a str,
    pub(crate) at: CellRef,
    pub(crate) to_sheet: &'a str,
    pub(crate) to: CellRef,
}

impl<'a> Host<'a> {
    /// A host the shift does not move: a defined name, a chart's series, a
    /// hyperlink's place.
    pub(crate) const fn fixed(sheet: &'a str, at: CellRef) -> Self {
        Self {
            sheet,
            at,
            to_sheet: sheet,
            to: at,
        }
    }

    /// The host seen from after the shift: where the opposite shift starts.
    const fn reversed(self) -> Self {
        Self {
            sheet: self.to_sheet,
            at: self.to,
            to_sheet: self.sheet,
            to: self.at,
        }
    }

    fn moved(&self) -> bool {
        self.at != self.to || !same_sheet(self.sheet, self.to_sheet)
    }
}

/// The cells a reference names, resolved at its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Cell(u32, u32),
    Area((u32, u32), (u32, u32)),
    Rows(u32, u32),
    Columns(u32, u32),
}

impl Place {
    /// The cells `target` names at `host`, `None` for a name, a `#REF!` or
    /// a coordinate off the grid there.
    fn of(target: &Target, host: CellRef) -> Option<Self> {
        let row = |coord: Coord| coord.resolve(host.row(), MAX_ROWS);
        let column = |coord: Coord| coord.resolve(host.column(), MAX_COLUMNS);
        Some(match target {
            Target::Cell {
                row: at_row,
                column: at_column,
            } => Self::Cell(row(*at_row)?, column(*at_column)?),
            Target::Area { first, last } => Self::Area(
                (row(first.0)?, column(first.1)?),
                (row(last.0)?, column(last.1)?),
            ),
            Target::Rows { first, last } => Self::Rows(row(*first)?, row(*last)?),
            Target::Columns { first, last } => Self::Columns(column(*first)?, column(*last)?),
            Target::Name(_) | Target::Invalid => return None,
        })
    }

    /// The place once `band` opened or closed along `axis`, `None` when it
    /// went.
    fn banded(self, axis: Axis, band: Band) -> Option<Self> {
        let limit = axis.limit();
        let pair = |a: u32, b: u32| -> Option<(u32, u32)> {
            let (first, last) = band.span(a.min(b), a.max(b), limit)?;
            Some(if a <= b { (first, last) } else { (last, first) })
        };
        Some(match (self, axis) {
            (Self::Cell(row, column), Axis::Rows) => Self::Cell(band.index(row, limit)?, column),
            (Self::Cell(row, column), Axis::Columns) => Self::Cell(row, band.index(column, limit)?),
            (Self::Area(first, last), Axis::Rows) => {
                let (a, b) = pair(first.0, last.0)?;
                Self::Area((a, first.1), (b, last.1))
            }
            (Self::Area(first, last), Axis::Columns) => {
                let (a, b) = pair(first.1, last.1)?;
                Self::Area((first.0, a), (last.0, b))
            }
            (Self::Rows(first, last), Axis::Rows) => {
                let (a, b) = pair(first, last)?;
                Self::Rows(a, b)
            }
            (Self::Columns(first, last), Axis::Columns) => {
                let (a, b) = pair(first, last)?;
                Self::Columns(a, b)
            }
            (other, _) => other,
        })
    }

    /// The cells as a range, `None` for whole rows or columns.
    fn range(self) -> Option<CellRange> {
        match self {
            Self::Cell(row, column) => {
                let at = CellRef::new(row, column);
                Some(CellRange::new(at, at))
            }
            Self::Area(first, last) => Some(CellRange::new(
                CellRef::new(first.0, first.1),
                CellRef::new(last.0, last.1),
            )),
            Self::Rows(..) | Self::Columns(..) => None,
        }
    }

    /// The occupied grid rectangle for a reference, including whole axes.
    fn extent(self) -> CellRange {
        match self {
            Self::Cell(row, column) => {
                let at = CellRef::new(row, column);
                CellRange::new(at, at)
            }
            Self::Area(first, last) => {
                CellRange::new(CellRef::new(first.0, first.1), CellRef::new(last.0, last.1))
            }
            Self::Rows(first, last) => CellRange::of_rows(first.min(last)..first.max(last) + 1),
            Self::Columns(first, last) => {
                CellRange::of_columns(first.min(last)..first.max(last) + 1)
            }
        }
    }

    /// Contract an area to its surviving bounding rectangle while retaining
    /// the reference's original corner order and whole-axis spelling.
    fn narrowed(self, range: CellRange) -> Self {
        let ordered = |first: u32, last: u32, low: u32, high: u32| {
            if first <= last {
                (low, high)
            } else {
                (high, low)
            }
        };
        match self {
            Self::Cell(..) => self,
            Self::Area(first, last) => {
                let (top, bottom) =
                    ordered(first.0, last.0, range.start().row(), range.end().row());
                let (left, right) = ordered(
                    first.1,
                    last.1,
                    range.start().column(),
                    range.end().column(),
                );
                Self::Area((top, left), (bottom, right))
            }
            Self::Rows(first, last) => {
                let (first, last) = ordered(first, last, range.start().row(), range.end().row());
                Self::Rows(first, last)
            }
            Self::Columns(first, last) => {
                let (first, last) =
                    ordered(first, last, range.start().column(), range.end().column());
                Self::Columns(first, last)
            }
        }
    }

    /// The place with every cell moved as `at` in `block` moves to
    /// `target`.
    fn translated(self, block: CellRange, target: CellRef) -> Option<Self> {
        let cell = |row: u32, column: u32| -> Option<(u32, u32)> {
            let moved = translated(CellRef::new(row, column), block, target)?;
            Some((moved.row(), moved.column()))
        };
        Some(match self {
            Self::Cell(row, column) => {
                let (row, column) = cell(row, column)?;
                Self::Cell(row, column)
            }
            Self::Area(first, last) => Self::Area(cell(first.0, first.1)?, cell(last.0, last.1)?),
            Self::Rows(first, last) => Self::Rows(cell(first, 0)?.0, cell(last, 0)?.0),
            Self::Columns(first, last) => Self::Columns(cell(0, first)?.1, cell(0, last)?.1),
        })
    }
}

/// `original`'s coordinates naming `place` from `host`: each written with
/// `$` stays absolute, each without is its distance from the host.
fn encode(original: &Target, place: Place, host: CellRef) -> Target {
    let row = |coord: Coord, index: u32| match coord {
        Coord::Absolute(_) => Coord::Absolute(index),
        Coord::Relative(_) => Coord::Relative(index as i32 - host.row() as i32),
    };
    let column = |coord: Coord, index: u32| match coord {
        Coord::Absolute(_) => Coord::Absolute(index),
        Coord::Relative(_) => Coord::Relative(index as i32 - host.column() as i32),
    };
    match (original, place) {
        (
            Target::Cell {
                row: at_row,
                column: at_column,
            },
            Place::Cell(r, c),
        ) => Target::Cell {
            row: row(*at_row, r),
            column: column(*at_column, c),
        },
        (Target::Area { first, last }, Place::Area(a, b)) => Target::Area {
            first: (row(first.0, a.0), column(first.1, a.1)),
            last: (row(last.0, b.0), column(last.1, b.1)),
        },
        (Target::Rows { first, last }, Place::Rows(a, b)) => Target::Rows {
            first: row(*first, a),
            last: row(*last, b),
        },
        (Target::Columns { first, last }, Place::Columns(a, b)) => Target::Columns {
            first: column(*first, a),
            last: column(*last, b),
        },
        (other, _) => other.clone(),
    }
}

/// Which native reference rule applies to one implicit carried host.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReferenceMove {
    /// Ordinary cells and same-sheet carried rules follow source dependencies
    /// and lose references to overwritten destination cells.
    All,
    /// Cross-sheet retained CF/DV follow source dependencies but retain their
    /// original references into the overwritten destination rectangle.
    SourceOnly,
    /// Same-sheet carried references follow selected cells and whole selected
    /// areas, but do not contract a partially selected referenced area.
    CarriedSame,
    /// Cross-sheet incoming CF/DV copy relative Shapes and absolute coordinates;
    /// only resolved local-name scope changes follow the workbook cut.
    Copy,
}

/// `reference` held at `host` once `shift` moved what it names, `None` when
/// it names what it named in the same words.
fn adjust_reference(
    reference: &Reference,
    host: &Host<'_>,
    shift: &Shift<'_>,
    policy: ReferenceMove,
) -> Option<Reference> {
    if let (
        Reference {
            sheet: SheetSpec::Own,
            target: Target::Name(name),
        },
        Shift::Move {
            from, to, names, ..
        },
    ) = (reference, shift)
    {
        if !same_sheet(from, to)
            && same_sheet(host.sheet, from)
            && same_sheet(host.to_sheet, to)
            && names.action(name) == Some(NameMove::Qualify)
        {
            return Some(Reference {
                sheet: SheetSpec::Named {
                    name: SmolStr::new(host.sheet),
                    quoted: false,
                },
                target: reference.target.clone(),
            });
        }
        return None;
    }
    if policy == ReferenceMove::Copy {
        return None;
    }
    let target_sheet: Option<&str> = match &reference.sheet {
        SheetSpec::Own => Some(host.sheet),
        SheetSpec::Named { name, .. } => Some(name.as_str()),
        SheetSpec::Span { .. } | SheetSpec::External(_) | SheetSpec::Invalid => None,
    };
    let invalid = |sheet: SheetSpec| Reference {
        sheet,
        target: Target::Invalid,
    };
    let Some(place) = Place::of(&reference.target, host.at) else {
        // A coordinate off the grid at the host stays `#REF!` wherever the
        // host goes.
        return match &reference.target {
            Target::Name(_) | Target::Invalid => None,
            _ if host.moved() => Some(invalid(reference.sheet.clone())),
            _ => None,
        };
    };
    let (sheet, moved) = match target_sheet {
        Some(name) => match relocate_with(shift, name, place, policy) {
            Relocated::Same => (None, place),
            Relocated::Moved(sheet, place) => (sheet, place),
            Relocated::Gone => return Some(invalid(reference.sheet.clone())),
        },
        None => (None, place),
    };
    let spec = match &reference.sheet {
        SheetSpec::Own => {
            let on = sheet.unwrap_or(host.sheet);
            if same_sheet(on, host.to_sheet) {
                SheetSpec::Own
            } else {
                SheetSpec::Named {
                    name: SmolStr::new(on),
                    quoted: false,
                }
            }
        }
        SheetSpec::Named { name, quoted } => match sheet {
            Some(on) if !same_sheet(on, name) => SheetSpec::Named {
                name: SmolStr::new(on),
                quoted: *quoted && needs_quotes(on),
            },
            _ => reference.sheet.clone(),
        },
        other => other.clone(),
    };
    let adjusted = Reference {
        sheet: spec,
        target: encode(&reference.target, moved, host.to),
    };
    (adjusted != *reference).then_some(adjusted)
}

/// What a shift does to the cells a reference names.
enum Relocated<'a> {
    Same,
    /// Moved, onto the sheet named when that changed.
    Moved(Option<&'a str>, Place),
    Gone,
}

/// Where `place` of the sheet `sheet` stands after `shift`.
fn relocate<'a>(shift: &Shift<'a>, sheet: &str, place: Place) -> Relocated<'a> {
    relocate_with(shift, sheet, place, ReferenceMove::All)
}

fn relocate_with<'a>(
    shift: &Shift<'a>,
    sheet: &str,
    place: Place,
    policy: ReferenceMove,
) -> Relocated<'a> {
    match *shift {
        Shift::Band {
            sheet: edited,
            axis,
            band,
        } => {
            if !same_sheet(sheet, edited) {
                return Relocated::Same;
            }
            match place.banded(axis, band) {
                Some(moved) if moved == place => Relocated::Same,
                Some(moved) => Relocated::Moved(None, moved),
                None => Relocated::Gone,
            }
        }
        Shift::Move {
            from,
            block,
            to,
            target,
            ..
        } => {
            let range = place.extent();
            let inside =
                |outer: CellRange| outer.contains(range.start()) && outer.contains(range.end());
            if same_sheet(sheet, from) && inside(block) {
                return match place.translated(block, target) {
                    Some(moved) => Relocated::Moved((!same_sheet(sheet, to)).then_some(to), moved),
                    None => Relocated::Gone,
                };
            }
            if policy != ReferenceMove::CarriedSame
                && same_sheet(sheet, from)
                && range.intersects(block)
            {
                let (_, remaining) = range.partition(block);
                let Some((&first, rest)) = remaining.split_first() else {
                    return Relocated::Gone;
                };
                let bounds = rest.iter().fold(first, |bounds, part| {
                    CellRange::new(
                        CellRef::new(
                            bounds.start().row().min(part.start().row()),
                            bounds.start().column().min(part.start().column()),
                        ),
                        CellRef::new(
                            bounds.end().row().max(part.end().row()),
                            bounds.end().column().max(part.end().column()),
                        ),
                    )
                });
                let narrowed = place.narrowed(bounds);
                if narrowed != place {
                    return Relocated::Moved(None, narrowed);
                }
            }
            if matches!(policy, ReferenceMove::All | ReferenceMove::CarriedSame)
                && same_sheet(sheet, to)
                && inside(block.moved_to(target))
            {
                return Relocated::Gone;
            }
            Relocated::Same
        }
        Shift::RenameSheet { .. } | Shift::RemoveSheet { .. } => Relocated::Same,
    }
}

/// `formula` held at `host` after `shift`, `None` when it is spelled as it
/// was.
pub(crate) fn shifted(formula: &Formula, host: &Host<'_>, shift: &Shift<'_>) -> Option<Formula> {
    shifted_with(formula, host, shift, ReferenceMove::All)
}

fn shifted_with(
    formula: &Formula,
    host: &Host<'_>,
    shift: &Shift<'_>,
    policy: ReferenceMove,
) -> Option<Formula> {
    match *shift {
        Shift::RenameSheet { from, to } => return formula.renamed(from, to),
        Shift::RemoveSheet { name, order } => return formula.removed(name, order),
        Shift::Band { .. } | Shift::Move { .. } => {}
    }
    let shape = formula.shape();
    let mut tokens: Option<Vec<Token>> = None;
    for (index, token) in shape.tokens.iter().enumerate() {
        if let Token::Reference(reference) = token
            && let Some(adjusted) = adjust_reference(reference, host, shift, policy)
        {
            tokens.get_or_insert_with(|| shape.tokens.to_vec())[index] = Token::Reference(adjusted);
        }
    }
    tokens.map(|tokens| Formula::from_shape(shape.with_tokens(tokens)))
}

/// `formula` held at `host` with each reference falling off the grid there
/// made `#REF!` for good, as a paste leaves it; `None` when none falls off.
pub(crate) fn materialized(formula: &Formula, host: CellRef) -> Option<Formula> {
    let shape = formula.shape();
    let mut tokens: Option<Vec<Token>> = None;
    for (index, token) in shape.tokens.iter().enumerate() {
        let Token::Reference(reference) = token else {
            continue;
        };
        if matches!(reference.target, Target::Name(_) | Target::Invalid)
            || Place::of(&reference.target, host).is_some()
        {
            continue;
        }
        tokens.get_or_insert_with(|| shape.tokens.to_vec())[index] = Token::Reference(Reference {
            sheet: reference.sheet.clone(),
            target: Target::Invalid,
        });
    }
    tokens.map(|tokens| Formula::from_shape(shape.with_tokens(tokens)))
}

/// A carried formula family after sparse ownership and reference partitioning.
/// Formulas remain relative Shapes; the XML owner renders them at the bounding
/// top-left of `ranges`, even when that point is outside their sparse union.
pub(crate) struct FormulaRegion {
    pub(crate) ranges: Vec<CellRange>,
    pub(crate) formulas: SmallVec<[Formula; 2]>,
}

/// Native CF and validation formulas differ only in the same-sheet retained
/// host policy. Their representation (standard or x14 XML) does not decide it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CarriedFormulaKind {
    Conditional,
    Validation,
}

/// At most this many additional host fragments are produced for one original
/// rule. It bounds adversarial relative-reference arrangements, never cells.
const MAX_FORMULA_FRAGMENTS: usize = 1_024;

struct FormulaFragments<'a> {
    produced: usize,
    part: &'a str,
}

impl FormulaFragments<'_> {
    fn add(&mut self, count: usize) -> Result<()> {
        if count > MAX_FORMULA_FRAGMENTS - self.produced {
            return Err(Error::Unsupported {
                operation: "producing more than 1024 carried-formula host fragments",
                filesystem: SmolStr::new(self.part),
            });
        }
        self.produced += count;
        Ok(())
    }

    /// Divide only rectangles crossing a proven affine boundary. Each split
    /// retains one old slot, so the bound is checked before any queue grows.
    fn divide(&mut self, ranges: &mut Vec<CellRange>, boundary: CellRange) -> Result<()> {
        for index in 0..ranges.len() {
            let (inside, outside) = ranges[index].partition(boundary);
            let Some(inside) = inside else { continue };
            if outside.is_empty() {
                continue;
            }
            self.add(outside.len())?;
            ranges[index] = inside;
            ranges.extend(outside);
        }
        Ok(())
    }

    fn refine(
        &mut self,
        ranges: &mut Vec<CellRange>,
        formulas: &[Formula],
        sheet: &str,
        shift: &Shift<'_>,
        policy: ReferenceMove,
        delta: (i32, i32),
    ) -> Result<()> {
        let Shift::Move {
            from,
            block,
            to,
            target,
            ..
        } = *shift
        else {
            return Ok(());
        };
        let grid = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1),
        );
        for formula in formulas {
            for token in &formula.shape().tokens {
                let Token::Reference(reference) = token else {
                    continue;
                };
                // A moved off-grid reference is permanently invalid; a copy
                // is tested at its new host, a cut at its original host.
                let host_delta = if policy == ReferenceMove::Copy {
                    delta
                } else {
                    (0, 0)
                };
                if let Some(boundary) = reference.target.hosts_in(grid, host_delta) {
                    self.divide(ranges, boundary)?;
                }
                if policy == ReferenceMove::Copy
                    || !matches!(reference.target, Target::Cell { .. } | Target::Area { .. })
                {
                    continue;
                }
                let owner = match &reference.sheet {
                    SheetSpec::Own => sheet,
                    SheetSpec::Named { name, .. } => name,
                    SheetSpec::Span { .. } | SheetSpec::External(_) | SheetSpec::Invalid => {
                        continue;
                    }
                };
                if same_sheet(owner, from)
                    && let Some(boundary) = reference.target.hosts_in(block, (0, 0))
                {
                    self.divide(ranges, boundary)?;
                }
                if matches!(policy, ReferenceMove::All | ReferenceMove::CarriedSame)
                    && same_sheet(owner, to)
                    && let Some(boundary) =
                        reference.target.hosts_in(block.moved_to(target), (0, 0))
                {
                    self.divide(ranges, boundary)?;
                }
            }
        }
        Ok(())
    }
}

/// First geometric occurrence settles output order; equal Shapes share one
/// group, including incoming and retained pieces of the same original rule.
#[derive(Default)]
struct FormulaGroups(HashMap<SmallVec<[Formula; 2]>, (usize, Vec<CellRange>)>);

impl FormulaGroups {
    fn add(&mut self, formulas: SmallVec<[Formula; 2]>, range: CellRange) {
        let ordinal = self.0.len();
        self.0
            .entry(formulas)
            .or_insert_with(|| (ordinal, Vec::new()))
            .1
            .push(range);
    }

    fn finish(self) -> Vec<FormulaRegion> {
        let mut groups: Vec<_> = self.0.into_iter().collect();
        groups.sort_unstable_by_key(|(_, (ordinal, _))| *ordinal);
        groups
            .into_iter()
            .map(|(formulas, (_, ranges))| FormulaRegion { ranges, formulas })
            .collect()
    }
}

impl Shift<'_> {
    /// Resolve one original CF/DV rule over sparse host rectangles. No cell is
    /// expanded; XML, priority, GUID and namespace ownership stay with the
    /// caller. `to_sheet` selects only the incoming source partition.
    pub(crate) fn formula_regions(
        &self,
        kind: CarriedFormulaKind,
        owner: (&str, Option<&str>),
        ranges: &[CellRange],
        formulas: &[Formula],
        part: &str,
    ) -> Result<Vec<FormulaRegion>> {
        let (sheet, to_sheet) = owner;
        if ranges.is_empty() {
            return Ok(Vec::new());
        }
        let Shift::Move {
            from,
            block,
            to,
            target,
            ..
        } = *self
        else {
            return Err(Error::Unsupported {
                operation: "partitioning carried formulas outside a cell cut",
                filesystem: SmolStr::new(part),
            });
        };
        if same_sheet(from, to) && target == block.start() {
            return Ok(vec![FormulaRegion {
                ranges: ranges.to_vec(),
                formulas: formulas.iter().cloned().collect(),
            }]);
        }
        let landing = block.moved_to(target);
        let delta = (
            target.row() as i32 - block.start().row() as i32,
            target.column() as i32 - block.start().column() as i32,
        );
        let cross_sheet = !same_sheet(from, to);
        let retained_policy = if cross_sheet {
            ReferenceMove::SourceOnly
        } else {
            ReferenceMove::CarriedSame
        };
        let relative_areas = formulas.iter().any(|formula| {
            formula.shape().tokens.iter().any(|token| {
                let Token::Reference(reference) = token else {
                    return false;
                };
                reference.target.is_relative_area()
                    && match &reference.sheet {
                        SheetSpec::Own => same_sheet(sheet, from),
                        SheetSpec::Named { name, .. } => same_sheet(name, from),
                        SheetSpec::Span { .. } | SheetSpec::External(_) | SheetSpec::Invalid => {
                            false
                        }
                    }
            })
        });
        let mut fragments = FormulaFragments { produced: 0, part };
        let mut groups = FormulaGroups::default();
        for &range in ranges {
            let (selected, remainder) = if same_sheet(sheet, from) {
                range.partition(block)
            } else {
                (None, smallvec::smallvec![range])
            };
            if relative_areas
                && to_sheet.is_none()
                && selected.is_some()
                && (!remainder.is_empty() || !cross_sheet)
            {
                // Excel's retained area updates depend on the rule family and
                // original component, not ordinary per-cell contraction. Until
                // that policy is modeled, refuse before publishing any frame.
                // Absolute/unrelated areas and cross-sheet copies remain exact.
                return Err(Error::Unsupported {
                    operation: "moving host-dependent formula areas from a split or same-sheet carried owner",
                    filesystem: format_smolstr!("{part}#sqref={range}"),
                });
            }
            let mut kept = Vec::new();
            if to_sheet.is_none() {
                for area in remainder {
                    if same_sheet(sheet, to) {
                        kept.extend(area.partition(landing).1);
                    } else {
                        kept.push(area);
                    }
                }
            }
            let incoming = selected.filter(|_| {
                to_sheet.is_some_and(|owner| same_sheet(owner, to))
                    || (to_sheet.is_none() && !cross_sheet)
            });
            // Original input areas are not charged. Only their newly produced
            // pieces and subsequent affine subdivisions consume the bound.
            fragments.add((kept.len() + usize::from(incoming.is_some())).saturating_sub(1))?;
            if !kept.is_empty() {
                if selected.is_none() || (kind == CarriedFormulaKind::Conditional && !cross_sheet) {
                    // An uncut original component follows dependencies once at
                    // its retained anchor in both CF and DV. Native same-sheet
                    // CF keeps that policy even when its component was cut;
                    // partial DV and cross-sheet CF require host classes.
                    let at = kept
                        .iter()
                        .map(|area| area.start())
                        .min()
                        .expect("a retained region is nonempty");
                    let host = Host::fixed(sheet, at);
                    let adjusted: SmallVec<[Formula; 2]> = formulas
                        .iter()
                        .map(|formula| {
                            shifted_with(formula, &host, self, retained_policy)
                                .unwrap_or_else(|| formula.clone())
                        })
                        .collect();
                    for area in kept {
                        groups.add(adjusted.clone(), area);
                    }
                } else {
                    fragments.refine(&mut kept, formulas, sheet, self, retained_policy, (0, 0))?;
                    for area in kept {
                        let host = Host::fixed(sheet, area.start());
                        let adjusted = formulas
                            .iter()
                            .map(|formula| {
                                shifted_with(formula, &host, self, retained_policy)
                                    .unwrap_or_else(|| formula.clone())
                            })
                            .collect();
                        groups.add(adjusted, area);
                    }
                }
            }
            if let Some(selected) = incoming {
                let policy = if cross_sheet {
                    ReferenceMove::Copy
                } else {
                    ReferenceMove::CarriedSame
                };
                let at = selected.start();
                let to_at = translated(at, block, target).ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new(part),
                    reason: "expected a carried formula host inside the worksheet".into(),
                })?;
                let host = Host {
                    sheet,
                    at,
                    to_sheet: to,
                    to: to_at,
                };
                for formula in formulas {
                    self.check_names(formula, &host, || SmolStr::new(part))?;
                }
                let mut pieces = vec![selected];
                fragments.refine(&mut pieces, formulas, sheet, self, policy, delta)?;
                for area in pieces {
                    let moved = translated(area.start(), block, target)
                        .expect("a selected host lies in the validated source block");
                    let host = Host {
                        sheet,
                        at: area.start(),
                        to_sheet: to,
                        to: moved,
                    };
                    let adjusted = formulas
                        .iter()
                        .map(|formula| {
                            let adjusted = shifted_with(formula, &host, self, policy)
                                .unwrap_or_else(|| formula.clone());
                            if policy == ReferenceMove::Copy {
                                materialized(&adjusted, moved).unwrap_or(adjusted)
                            } else {
                                adjusted
                            }
                        })
                        .collect();
                    groups.add(adjusted, area.moved_to(moved));
                }
            }
        }
        Ok(groups.finish())
    }
}

/// Native worksheet-filter ownership after a cut. A table's filter is
/// owned by its table and continues through the table reference adjuster.
pub(crate) enum FilterMove {
    Keep,
    Drop,
    Move(CellRange),
}

impl Shift<'_> {
    /// The observed partial cut removes every data row while keeping its
    /// header. Other partial rectangles need an explicit semantics decision.
    pub(crate) fn cuts_filter_body(&self, range: CellRange, sheet: &str) -> bool {
        let Self::Move {
            from, block, to, ..
        } = self
        else {
            return false;
        };
        !same_sheet(from, to)
            && same_sheet(sheet, from)
            && range.row_size() > 1
            && *block
                == CellRange::new(
                    CellRef::new(range.start().row() + 1, range.start().column()),
                    range.end(),
                )
    }

    /// Excel removes a fully cut cross-sheet filter, moves a same-sheet
    /// filter, and keeps a body-only source filter in its original geometry.
    pub(crate) fn filter_move(
        &self,
        range: CellRange,
        sheet: &str,
        part: &str,
    ) -> Result<FilterMove> {
        let Self::Move {
            from,
            block,
            to,
            target,
            ..
        } = *self
        else {
            return Ok(FilterMove::Keep);
        };
        if same_sheet(from, to) && target == block.start() {
            return Ok(FilterMove::Keep);
        }
        let landing = block.moved_to(target);
        let refused = || Error::Unsupported {
            operation: "moving an unmodeled worksheet filter overlap",
            filesystem: format_smolstr!("{part}#autoFilter"),
        };
        if same_sheet(sheet, from) && block.encloses(range) {
            if !same_sheet(from, to) {
                return Ok(FilterMove::Drop);
            }
            if range.intersects(landing) && target != block.start() {
                return Err(refused());
            }
            let start = translated(range.start(), block, target).ok_or_else(refused)?;
            return Ok(FilterMove::Move(range.moved_to(start)));
        }
        if same_sheet(sheet, from) && range.intersects(block) {
            if self.cuts_filter_body(range, sheet) {
                return Ok(FilterMove::Keep);
            }
            // Excel preserves a partially cut filter (including its criteria,
            // nested sort and FilterDatabase) when every outer edge survives.
            // A same-sheet landing inside that filter has separate semantics.
            let (_, remaining) = range.partition(block);
            let start = range.start();
            let end = range.end();
            let bounds_survive = remaining
                .iter()
                .any(|part| part.start().row() == start.row())
                && remaining
                    .iter()
                    .any(|part| part.start().column() == start.column())
                && remaining.iter().any(|part| part.end().row() == end.row())
                && remaining
                    .iter()
                    .any(|part| part.end().column() == end.column());
            return if bounds_survive && (!same_sheet(from, to) || !range.intersects(landing)) {
                Ok(FilterMove::Keep)
            } else {
                Err(refused())
            };
        }
        if same_sheet(sheet, to) && range.intersects(landing) {
            return if landing.encloses(range) {
                Ok(FilterMove::Drop)
            } else {
                Err(refused())
            };
        }
        Ok(FilterMove::Keep)
    }

    /// A fully selected standalone sort remains on the source of a
    /// cross-sheet cut. Unobserved affected placements refuse before mutation.
    pub(crate) fn check_sort_cut(&self, range: CellRange, sheet: &str, part: &str) -> Result<()> {
        let Self::Move {
            from,
            block,
            to,
            target,
            ..
        } = *self
        else {
            return Ok(());
        };
        if same_sheet(from, to) && target == block.start() {
            return Ok(());
        }
        if !same_sheet(from, to) && same_sheet(sheet, from) && block.encloses(range) {
            return Ok(());
        }
        if (same_sheet(sheet, from) && range.intersects(block))
            || (same_sheet(sheet, to) && range.intersects(block.moved_to(target)))
        {
            return Err(Error::Unsupported {
                operation: "moving an unmodeled standalone worksheet sort overlap",
                filesystem: format_smolstr!("{part}#sortState"),
            });
        }
        Ok(())
    }

    /// The filter's hidden range contracts to its header after the observed
    /// whole-body cut. Reuse the resolved formula token and coordinate encoder;
    /// ordinary defined names retain the general reference-following rule.
    pub(crate) fn filter_database(
        &self,
        formula: &Formula,
        scope: &str,
        part: &str,
    ) -> Result<Option<Formula>> {
        let Self::Move { from, .. } = self else {
            return Ok(None);
        };
        if !same_sheet(scope, from) {
            return Ok(None);
        }
        let shape = formula.shape();
        let mut reference = None;
        for (index, token) in shape.tokens.iter().enumerate() {
            match token {
                Token::Reference(value) if reference.is_none() => reference = Some((index, value)),
                Token::Text(text) if text.trim().is_empty() => {}
                _ => {
                    return Err(Error::Unsupported {
                        operation: "moving a nonrectangular filter database",
                        filesystem: format_smolstr!("{part}#_xlnm._FilterDatabase"),
                    });
                }
            }
        }
        let Some((index, reference)) = reference else {
            return Ok(None);
        };
        let sheet = match &reference.sheet {
            SheetSpec::Own => scope,
            SheetSpec::Named { name, .. } => name,
            _ => return Ok(None),
        };
        let origin = CellRef::new(0, 0);
        let Some(range) = Place::of(&reference.target, origin).and_then(Place::range) else {
            return Ok(None);
        };
        if !self.cuts_filter_body(range, sheet) {
            return Ok(None);
        }
        let first = (range.start().row(), range.start().column());
        let last = (range.start().row(), range.end().column());
        let adjusted = Reference {
            sheet: reference.sheet.clone(),
            target: encode(&reference.target, Place::Area(first, last), origin),
        };
        let mut tokens = shape.tokens.to_vec();
        tokens[index] = Token::Reference(adjusted);
        Ok(Some(Formula::from_shape(shape.with_tokens(tokens))))
    }
}

/// The memo key of one host class: the host's own region and, for each
/// coordinate the class depends on, its region or its index.
type Key = SmallVec<[u32; 8]>;

/// What one rewrite answered: the original held so no other shape takes
/// its address while the memo lives, the rewrite, and whether the opposite
/// shift gives the original back.
struct Answer {
    _original: Formula,
    rewritten: Option<Formula>,
    reversible: bool,
}

/// Every formula of a workbook rewritten for one shift, each shape once per
/// host class.
pub(crate) struct Rewriter<'a> {
    shift: Shift<'a>,
    inverse: Option<Shift<'a>>,
    memo: HashMap<(usize, Key), Answer>,
}

impl<'a> Rewriter<'a> {
    pub(crate) fn new(shift: Shift<'a>) -> Self {
        Self {
            inverse: shift.inverse(),
            shift,
            memo: HashMap::new(),
        }
    }

    /// `formula` held at `host` after the shift - `None` when it is spelled
    /// as it was - and whether the opposite shift gives `formula` back.
    pub(crate) fn rewrite(
        &mut self,
        formula: &Formula,
        host: &Host<'_>,
    ) -> (Option<Formula>, bool) {
        let key = match self.shift {
            Shift::Band { sheet, axis, band } => class_key(formula, host, sheet, axis, band),
            Shift::RenameSheet { .. } | Shift::RemoveSheet { .. } => Key::new(),
            Shift::Move { .. } => {
                // A move is a user's cut: few cells, each its own class. No
                // opposite shift gives a rewrite back, so what changed is
                // what an undo restores; what did not needs nothing.
                let rewritten = shifted(formula, host, &self.shift);
                let reversible = rewritten.is_none();
                return (rewritten, reversible);
            }
        };
        let shift = self.shift;
        let inverse = self.inverse;
        let answer = self
            .memo
            .entry((formula.address(), key))
            .or_insert_with(|| {
                let rewritten = shifted(formula, host, &shift);
                let reversible = match (&rewritten, &inverse) {
                    (None, _) => true,
                    (Some(rewritten), Some(inverse)) => {
                        shifted(rewritten, &host.reversed(), inverse).as_ref() == Some(formula)
                    }
                    (Some(_), None) => false,
                };
                Answer {
                    _original: formula.clone(),
                    rewritten,
                    reversible,
                }
            });
        (answer.rewritten.clone(), answer.reversible)
    }
}

/// The host class of `formula` at `host` under a band opened or closed
/// along `axis` of the sheet `edited`.
fn class_key(formula: &Formula, host: &Host<'_>, edited: &str, axis: Axis, band: Band) -> Key {
    let mut key = Key::new();
    let on_edited = same_sheet(host.sheet, edited);
    key.push(u32::from(on_edited));
    if on_edited {
        band.class(i64::from(axis.of(host.at)), axis.limit(), &mut key);
    }
    let host_index = i64::from(axis.of(host.at));
    for token in &*formula.shape().tokens {
        let Token::Reference(reference) = token else {
            continue;
        };
        let names_edited = match &reference.sheet {
            SheetSpec::Own => on_edited,
            SheetSpec::Named { name, .. } => same_sheet(name, edited),
            _ => false,
        };
        if !names_edited {
            continue;
        }
        let mut coord = |coord: Coord| {
            if let Coord::Relative(offset) = coord {
                band.class(host_index + i64::from(offset), axis.limit(), &mut key);
            }
        };
        match (&reference.target, axis) {
            (Target::Cell { row, .. }, Axis::Rows) => coord(*row),
            (Target::Cell { column, .. }, Axis::Columns) => coord(*column),
            (Target::Area { first, last }, Axis::Rows) => {
                coord(first.0);
                coord(last.0);
            }
            (Target::Area { first, last }, Axis::Columns) => {
                coord(first.1);
                coord(last.1);
            }
            (Target::Rows { first, last }, Axis::Rows)
            | (Target::Columns { first, last }, Axis::Columns) => {
                coord(*first);
                coord(*last);
            }
            _ => {}
        }
    }
    key
}

impl CellRange {
    /// The OOXML range spelling uses full corners even at a grid edge.
    pub(crate) fn write_a1(self, text: &mut String) {
        self.start().write_a1(text);
        if self.start() != self.end() {
            text.push(':');
            self.end().write_a1(text);
        }
    }

    /// Split a carried rectangle into its intersection with `cut` and the
    /// disjoint remainder. At most four strips, independent of cell count;
    /// predecessor/successor arithmetic occurs only inside a strict bound.
    fn partition(self, cut: Self) -> (Option<Self>, SmallVec<[Self; 4]>) {
        if !self.intersects(cut) {
            return (None, smallvec::smallvec![self]);
        }
        let first = CellRef::new(
            self.start().row().max(cut.start().row()),
            self.start().column().max(cut.start().column()),
        );
        let last = CellRef::new(
            self.end().row().min(cut.end().row()),
            self.end().column().min(cut.end().column()),
        );
        let mut remainder = SmallVec::new();
        if self.start().row() < first.row() {
            remainder.push(Self::new(
                self.start(),
                CellRef::new(first.row() - 1, self.end().column()),
            ));
        }
        if last.row() < self.end().row() {
            remainder.push(Self::new(
                CellRef::new(last.row() + 1, self.start().column()),
                self.end(),
            ));
        }
        if self.start().column() < first.column() {
            remainder.push(Self::new(
                CellRef::new(first.row(), self.start().column()),
                CellRef::new(last.row(), first.column() - 1),
            ));
        }
        if last.column() < self.end().column() {
            remainder.push(Self::new(
                CellRef::new(first.row(), last.column() + 1),
                CellRef::new(last.row(), self.end().column()),
            ));
        }
        (Some(Self::new(first, last)), remainder)
    }
}

/// A divisible carried range list after a cut. Scalar references, table
/// extents, filter ranges and sparkline hosts retain their own rules.
/// Source cells win a same-sheet overlap: subtract source and landing from
/// the retained rectangles, then append the translated source intersection.
fn partition_list(
    text: &str,
    sheet: &str,
    to_sheet: Option<&str>,
    shift: &Shift<'_>,
    path: &str,
) -> Result<Option<String>> {
    let Shift::Move {
        from,
        block,
        to,
        target,
        ..
    } = *shift
    else {
        return match to_sheet {
            Some(to) => moved_list(text, sheet, to, shift, path),
            None => adjust_list(text, sheet, shift, path),
        };
    };
    let landing = block.moved_to(target);
    let mut output = String::new();
    let mut write = |range: CellRange| {
        // A completely removed registration needs no output allocation.
        if output.capacity() == 0 {
            output.reserve(text.len().max(16));
        }
        if !output.is_empty() {
            output.push(' ');
        }
        range.write_a1(&mut output);
    };
    for piece in text.split_whitespace() {
        let range: CellRange = piece.parse().map_err(|_| Error::InvalidRecord {
            path: SmolStr::new(path),
            reason: format_smolstr!("expected a list of ranges such as A1:B2 C3, got {text:?}"),
        })?;
        let (selected, kept) = if same_sheet(sheet, from) {
            range.partition(block)
        } else {
            (None, smallvec::smallvec![range])
        };
        if to_sheet.is_none() {
            for range in kept {
                if same_sheet(sheet, to) {
                    for range in range.partition(landing).1 {
                        write(range);
                    }
                } else {
                    write(range);
                }
            }
        }
        if (to_sheet.is_some_and(|owner| same_sheet(owner, to))
            || (to_sheet.is_none() && same_sheet(from, to)))
            && let Some(selected) = selected
        {
            let moved = translated(selected.start(), block, target).ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new(path),
                    reason: format_smolstr!("expected the carried range {selected} to land inside the worksheet, got {target}"),
                })?;
            write(selected.moved_to(moved));
        }
    }
    Ok((!output.is_empty()).then_some(output))
}

/// `range`, on the sheet `sheet`, once `shift` moved its cells, `None` when
/// they went: a band removing all of it, a move overwriting it. A range the
/// shift does not reach is answered as it is.
pub(crate) fn adjust_range(range: CellRange, sheet: &str, shift: &Shift<'_>) -> Option<CellRange> {
    let place = Place::Area(
        (range.start().row(), range.start().column()),
        (range.end().row(), range.end().column()),
    );
    match relocate(shift, sheet, place) {
        Relocated::Same => Some(range),
        // A range moved onto another sheet leaves this one.
        Relocated::Moved(None, moved) => moved.range(),
        Relocated::Moved(Some(_), _) | Relocated::Gone => None,
    }
}

/// A space-separated list of ranges (`sqref`) once `shift` moved the cells
/// of the sheet `sheet`, the ranges that went left out; `None` when none is
/// left.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] naming `path` for a list naming no
/// range.
pub(crate) fn adjust_list(
    text: &str,
    sheet: &str,
    shift: &Shift<'_>,
    path: &str,
) -> Result<Option<String>> {
    let mut kept = Vec::new();
    for piece in text.split_whitespace() {
        let range: CellRange = piece.parse().map_err(|_| Error::InvalidRecord {
            path: SmolStr::new(path),
            reason: format_smolstr!("expected a list of ranges such as A1:B2 C3, got {text:?}"),
        })?;
        check_partial_carried(range, sheet, shift, path)?;
        if let Some(moved) = adjust_range(range, sheet, shift) {
            kept.push(range_text(moved));
        }
    }
    Ok((!kept.is_empty()).then(|| kept.join(" ")))
}

/// A carried range partly crossing a cut boundary has no single owner.
/// Refuse before either worksheet can silently keep or lose it.
fn check_partial_carried(
    range: CellRange,
    sheet: &str,
    shift: &Shift<'_>,
    path: &str,
) -> Result<()> {
    if let Shift::Move {
        from,
        block,
        to,
        target,
        ..
    } = shift
        && !same_sheet(from, to)
    {
        let overlap = if same_sheet(sheet, from) {
            Some(*block)
        } else if same_sheet(sheet, to) {
            Some(block.moved_to(*target))
        } else {
            None
        };
        if let Some(area) = overlap
            && range.intersects(area)
            && !area.encloses(range)
        {
            return Err(Error::Unsupported {
                operation: "cross-sheet cut of a partial carried range",
                filesystem: format_smolstr!("{path}#{range}"),
            });
        }
    }
    Ok(())
}

/// The partition of a carried range list landing on `to`. The source pass
/// uses `adjust_list`; both decisions come from the same `relocate` owner.
pub(crate) fn moved_list(
    text: &str,
    sheet: &str,
    to: &str,
    shift: &Shift<'_>,
    path: &str,
) -> Result<Option<String>> {
    let mut moved = Vec::new();
    for piece in text.split_whitespace() {
        let range: CellRange = piece.parse().map_err(|_| Error::InvalidRecord {
            path: SmolStr::new(path),
            reason: format_smolstr!("expected a list of ranges such as A1:B2 C3, got {text:?}"),
        })?;
        check_partial_carried(range, sheet, shift, path)?;
        let place = Place::Area(
            (range.start().row(), range.start().column()),
            (range.end().row(), range.end().column()),
        );
        if let Relocated::Moved(Some(owner), landed) = relocate(shift, sheet, place)
            && same_sheet(owner, to)
            && let Some(range) = landed.range()
        {
            moved.push(range_text(range));
        }
    }
    Ok((!moved.is_empty()).then(|| moved.join(" ")))
}

/// A range as a part's `ref` or `sqref` spells it: two corners, or one
/// cell.
pub(crate) fn range_text(range: CellRange) -> String {
    let mut text = String::with_capacity(16);
    range.write_a1(&mut text);
    text
}

/// The formula text `text` held at `host` after `shift`, `None` when it is
/// spelled as it was.
pub(crate) fn adjust_formula_text(
    text: &str,
    host: &Host<'_>,
    shift: &Shift<'_>,
) -> Result<Option<String>> {
    if !shift.may_name(text) && !shift.moves_cells_of(host.sheet) {
        return Ok(None);
    }
    let formula = Formula::from_file(text, host.at);
    shift.check_names(&formula, host, || {
        format_smolstr!("{}!{}", host.sheet, host.at)
    })?;
    Ok(shifted(&formula, host, shift).map(|formula| formula.at(host.to).to_string()))
}

/// The first cell `sqref` names, `None` for a list naming none.
fn first_cell(sqref: &str) -> Option<CellRef> {
    sqref
        .split_whitespace()
        .next()
        .and_then(|piece| piece.parse::<CellRange>().ok())
        .map(CellRange::start)
}

/// The edits of a document stating ranges of the sheet `sheet` - a carried
/// child of its worksheet part, one of its tables, its comments, its pivot
/// tables: its ranges moved, its formulas rewritten, what the shift took
/// whole left out. The x14 children of a worksheet state their range after
/// their formulas, so the ranges each states are read first (`hosts`).
pub(crate) struct SheetEdits<'s, 'a> {
    pub(crate) sheet: &'s str,
    pub(crate) shift: &'s Shift<'a>,
    /// The part refusals name.
    pub(crate) part: &'s str,
    /// A carried registration being transferred to another worksheet. Its
    /// source name still resolves the formulas it held before this shift.
    to_sheet: Option<&'s str>,
    /// The destination pass found an owned range, even if no bytes changed.
    transferred: bool,
    /// The `sqref`s the x14 children state, in document order.
    hosts: Vec<Option<String>>,
    /// Which x14 child the pass is in, and the host of the formulas it
    /// holds: the cell they are spelled from, the cell they are read as and
    /// where that cell stands after.
    x14: usize,
    host: Option<(CellRef, CellRef, CellRef)>,
    /// The autofilter's first column, before and after, for `colId`.
    filter: Option<(u32, u32)>,
    /// A table part before the shift, its range after and its new sheet
    /// when it leaves this one. Its filter excludes totals; each column's
    /// formula templates have their own hosts, even outside an empty table.
    table: Option<(Table, CellRange, Option<&'a str>)>,
    /// A table column's zero-based position, independent of its id.
    table_column: u32,
    /// The manual breaks a pass left out.
    manual_dropped: usize,
}

impl<'s, 'a> SheetEdits<'s, 'a> {
    /// The edits of one carried child.
    ///
    /// # Errors
    ///
    /// Returns the refusal of bytes that are not well-formed.
    pub(crate) fn apply(
        bytes: &[u8],
        sheet: &'s str,
        shift: &'s Shift<'a>,
        part: &'s str,
        to_sheet: Option<&'s str>,
    ) -> Result<Option<Vec<u8>>> {
        let mut edits = Self {
            sheet,
            shift,
            part,
            to_sheet,
            transferred: false,
            hosts: Vec::new(),
            x14: 0,
            host: None,
            filter: None,
            table: None,
            table_column: 0,
            manual_dropped: 0,
        };
        let mut gather = GatherX14 { hosts: Vec::new() };
        edit_document(bytes, &mut gather)?;
        edits.hosts = gather.hosts;
        let rewritten = edit_document(bytes, &mut edits)?;
        if to_sheet.is_some() && !edits.transferred {
            // Unselected opaque children stay with their existing owner.
            Ok(Some(Vec::new()))
        } else {
            Ok(rewritten)
        }
    }

    fn ranges(&mut self, text: &str) -> Result<Option<String>> {
        let ranges = match self.to_sheet {
            Some(to) => moved_list(text, self.sheet, to, self.shift, self.part),
            None => adjust_list(text, self.sheet, self.shift, self.part),
        }?;
        self.transferred |= self.to_sheet.is_some() && ranges.is_some();
        Ok(ranges)
    }

    fn partitioned_ranges(&mut self, text: &str) -> Result<Option<String>> {
        let ranges = partition_list(text, self.sheet, self.to_sheet, self.shift, self.part)?;
        self.transferred |= self.to_sheet.is_some() && ranges.is_some();
        Ok(ranges)
    }

    /// A range hyperlink remains on its original whole anchor when only an
    /// interior is cut, while the selected intersection gets its own copy on
    /// another worksheet. The source relationship is retained by the frame's
    /// surviving r:id; the destination receives its own relationship entry.
    fn hyperlink_ref(&mut self, text: &str) -> Result<Option<String>> {
        if let Shift::Move {
            from,
            block,
            to,
            target,
            ..
        } = *self.shift
            && !same_sheet(from, to)
            && same_sheet(self.sheet, from)
        {
            let range: CellRange = text.parse().map_err(|_| self.refused("hyperlink", text))?;
            let (selected, _) = range.partition(block);
            if let Some(selected) = selected.filter(|_| !block.encloses(range)) {
                if self.to_sheet.is_none() {
                    return Ok(Some(text.to_owned()));
                }
                let moved = translated(selected.start(), block, target).ok_or_else(||
                        Error::InvalidRecord {
                            path: format_smolstr!("{}#hyperlink", self.part),
                            reason: format_smolstr!(
                                "expected hyperlink intersection {selected} to land inside the worksheet, got {target}"
                            ),
                        })?;
                self.transferred = true;
                return Ok(Some(range_text(selected.moved_to(moved))));
            }
        }
        self.ranges(text)
    }

    /// Unlike worksheet-carried ranges, a table's ranges may leave their
    /// original sheet together with the table's package ownership.
    fn table_range(&self, range: CellRange) -> Option<(CellRange, Option<&'a str>)> {
        let place = Place::Area(
            (range.start().row(), range.start().column()),
            (range.end().row(), range.end().column()),
        );
        match relocate(self.shift, self.sheet, place) {
            Relocated::Same => Some((range, None)),
            Relocated::Moved(sheet, moved) => moved.range().map(|range| (range, sheet)),
            Relocated::Gone => None,
        }
    }

    /// The host of a formula written for the ranges `before`, which the
    /// shift makes `after`: their first cell after, and where that cell
    /// stood before - the formula is each cell's own, spelled from the
    /// first, so it is read from the cell that becomes the first.
    fn host_of(&self, before: &str, after: Option<&str>) -> Option<(CellRef, CellRef, CellRef)> {
        let old = first_cell(before)?;
        let new = after.and_then(first_cell).unwrap_or(old);
        let origin = self
            .shift
            .origin(self.to_sheet.unwrap_or(self.sheet), new)
            .unwrap_or(old);
        Some((old, origin, new))
    }

    fn formula(&self, text: &str) -> Result<Option<String>> {
        let origin = CellRef::new(0, 0);
        let (written, at, to) = self.host.unwrap_or((origin, origin, origin));
        if !self.shift.may_name(text) && !self.shift.moves_cells_of(self.sheet) {
            return Ok(None);
        }
        // Spelled from the first cell, read as the cell that becomes the
        // first sees it.
        let formula = Formula::from_file(text, written);
        let host = Host {
            sheet: self.sheet,
            at,
            to_sheet: self
                .to_sheet
                .or_else(|| self.table.as_ref().and_then(|(_, _, sheet)| *sheet))
                .unwrap_or(self.sheet),
            to,
        };
        self.shift
            .check_names(&formula, &host, || SmolStr::new(self.part))?;
        let rewritten = shifted(&formula, &host, self.shift);
        if rewritten.is_none() && written == to {
            return Ok(None);
        }
        Ok(Some(rewritten.unwrap_or(formula).at(to).to_string()).filter(|spelled| spelled != text))
    }
}

/// The ranges each x14 conditional format, validation and sparkline
/// states, in document order.
struct GatherX14 {
    hosts: Vec<Option<String>>,
}

impl Edits for GatherX14 {
    fn start(
        &mut self,
        path: &[SmolStr],
        _: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        if is_x14_host(path) {
            self.hosts.push(None);
        }
        Ok(Tag::Keep)
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        if path.last().is_some_and(|name| name == "sqref")
            && path.len() >= 2
            && is_x14_host(&path[..path.len() - 1])
            && let Some(last) = self.hosts.last_mut()
        {
            *last = Some(text.to_owned());
        }
        Ok(None)
    }
}

/// Whether `path` ends at an x14 element whose `xm:sqref` hosts the
/// formulas inside it.
fn is_x14_host(path: &[SmolStr]) -> bool {
    path.len() >= 2
        && path.first().is_some_and(|root| root == "extLst")
        && path.last().is_some_and(|name| {
            matches!(
                name.as_str(),
                "conditionalFormatting" | "dataValidation" | "sparkline"
            )
        })
}

impl Edits for SheetEdits<'_, '_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let get = |name: &str| {
            attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let own = self.shift.moves_cells_of(self.sheet);
        let name = path.last().map_or("", SmolStr::as_str);
        let parent = path.len().checked_sub(2).map_or("", |at| path[at].as_str());
        if is_x14_host(path) {
            let index = self.x14;
            self.x14 += 1;
            let before = self.hosts.get(index).cloned().flatten();
            let Some(before) = before else {
                self.host = None;
                return Ok(Tag::Keep);
            };
            let after = if own {
                self.ranges(&before)?
            } else {
                Some(before.clone())
            };
            if after.is_none() {
                return Ok(Tag::Drop);
            }
            self.host = self.host_of(&before, after.as_deref());
            // A sparkline's data is no formula held at its cell.
            if name == "sparkline" {
                self.host = None;
            }
            return Ok(Tag::Keep);
        }
        let mut set: Vec<(SmolStr, Option<String>)> = Vec::new();
        match (parent, name) {
            ("", "table") => {
                let table = Table::from_attributes(attributes, self.part, false)?;
                let (moved, destination) = if own {
                    self.table_range(table.range)
                        .ok_or_else(|| Error::InvalidRecord {
                            path: SmolStr::new(self.part),
                            reason: format_smolstr!(
                                "expected the edit to keep a cell of the table at {}, got one \
                                 taking all of it",
                                table.range
                            ),
                        })?
                } else {
                    (table.range, None)
                };
                if moved != table.range {
                    set.push(("ref".into(), Some(range_text(moved))));
                }
                self.table = Some((table, moved, destination));
                self.table_column = 0;
            }
            ("tableColumn", "calculatedColumnFormula" | "totalsRowFormula") => {
                if let Some((table, after, destination)) = &self.table {
                    let totals = name == "totalsRowFormula";
                    let before =
                        table.formula_host(table.range, self.table_column, totals, self.part)?;
                    let to = table.formula_host(*after, self.table_column, totals, self.part)?;
                    let at = match self.shift {
                        Shift::Move { from, block, .. }
                            if same_sheet(self.sheet, from) && block.encloses(table.range) =>
                        {
                            // Dormant templates can be hosted just outside
                            // a header-only table, outside the cut's landing.
                            before
                        }
                        _ => self
                            .shift
                            .origin(destination.unwrap_or(self.sheet), to)
                            .unwrap_or(before),
                    };
                    self.host = Some((before, at, to));
                }
            }
            ("pivotTableDefinition", "location") => {
                if let Some(range) = get("ref").filter(|_| own) {
                    let parsed: CellRange = range.parse().map_err(|_| self.refused(name, range))?;
                    match adjust_range(parsed, self.sheet, self.shift) {
                        None => {
                            return Err(Error::InvalidRecord {
                                path: SmolStr::new(self.part),
                                reason: format_smolstr!(
                                    "expected the edit to keep a cell of the {name} at {range}, got \
                                     one taking all of it"
                                ),
                            });
                        }
                        Some(moved) => {
                            if moved != parsed {
                                set.push(("ref".into(), Some(range_text(moved))));
                            }
                        }
                    }
                }
            }
            ("commentList", "comment") | (_, "threadedComment") => {
                if let Some(cell) = get("ref").filter(|_| own) {
                    let at: CellRef = cell.parse().map_err(|_| self.refused(name, cell))?;
                    match self.shift.place(self.sheet, at) {
                        None => return Ok(Tag::Drop),
                        Some((_, moved)) if moved != at => {
                            set.push(("ref".into(), Some(moved.to_string())));
                        }
                        Some(_) => {}
                    }
                }
            }
            ("hyperlinks", "hyperlink") => {
                if let Some(range) = get("ref").filter(|_| own) {
                    match self.hyperlink_ref(range)? {
                        None => return Ok(Tag::Drop),
                        Some(moved) if moved != range => set.push(("ref".into(), Some(moved))),
                        Some(_) => {}
                    }
                }
                if let Some(location) = get("location") {
                    let origin = CellRef::new(0, 0);
                    let host = Host {
                        sheet: self.sheet,
                        at: origin,
                        to_sheet: self.to_sheet.unwrap_or(self.sheet),
                        to: origin,
                    };
                    if let Some(moved) = adjust_formula_text(location, &host, self.shift)? {
                        set.push(("location".into(), Some(moved)));
                    }
                }
            }
            (_, "cfvo") => {
                // A threshold stated as a formula - or as a number a cell
                // reference spells - reads as the rule's formulas do.
                if let Some(value) = get("val")
                    && let Some(moved) = self.formula(value)?
                {
                    set.push(("val".into(), Some(moved)));
                }
            }
            (_, "conditionalFormatting") | ("dataValidations", "dataValidation") => {
                if let Some(sqref) = get("sqref") {
                    let after = if own {
                        self.ranges(sqref)?
                    } else {
                        Some(sqref.to_owned())
                    };
                    let Some(after) = after else {
                        return Ok(Tag::Drop);
                    };
                    self.host = self.host_of(sqref, Some(&after));
                    if after != sqref {
                        set.push(("sqref".into(), Some(after)));
                    }
                }
            }
            ("protectedRanges", "protectedRange") | ("ignoredErrors", "ignoredError") => {
                if let Some(sqref) = get("sqref").filter(|_| own) {
                    match self.partitioned_ranges(sqref)? {
                        None => return Ok(Tag::Drop),
                        Some(moved) if moved != sqref => set.push(("sqref".into(), Some(moved))),
                        Some(_) => {}
                    }
                }
            }
            (_, "autoFilter" | "sortState" | "sortCondition") => {
                if let Some(range) = get("ref").filter(|_| own) {
                    let parsed: CellRange = range.parse().map_err(|_| self.refused(name, range))?;
                    // A table's filter spans the table but its totals rows,
                    // which a row opening above the totals grows with it.
                    let moved = match &self.table {
                        Some((table, after, _))
                            if parent == "table"
                                && name == "autoFilter"
                                && without_totals(table.range, table.totals_rows)
                                    == Some(parsed) =>
                        {
                            without_totals(*after, table.totals_rows)
                        }
                        Some((_, _, destination)) => {
                            self.table_range(parsed).and_then(|(range, sheet)| {
                                same_sheet(
                                    sheet.unwrap_or(self.sheet),
                                    destination.unwrap_or(self.sheet),
                                )
                                .then_some(range)
                            })
                        }
                        _ => adjust_range(parsed, self.sheet, self.shift),
                    };
                    match moved {
                        None => return Ok(Tag::Drop),
                        Some(moved) => {
                            if name == "autoFilter" {
                                self.filter =
                                    Some((parsed.start().column(), moved.start().column()));
                            }
                            if moved != parsed {
                                set.push(("ref".into(), Some(range_text(moved))));
                            }
                        }
                    }
                }
            }
            ("autoFilter", "filterColumn") => {
                // A column counts from the filter's first column: only
                // columns opening or closing move it, and a filter moved
                // whole by a cut keeps its columns.
                if let (
                    Some(column),
                    Some((before, after)),
                    Shift::Band {
                        axis: Axis::Columns,
                        band,
                        ..
                    },
                ) = (get("colId"), self.filter.filter(|_| own), *self.shift)
                {
                    let offset: u32 = column
                        .trim()
                        .parse()
                        .map_err(|_| self.refused(name, column))?;
                    let index = before
                        .checked_add(offset)
                        .ok_or_else(|| self.refused(name, column))?;
                    match band.index(index, MAX_COLUMNS) {
                        Some(moved) => {
                            let moved_offset = moved.saturating_sub(after);
                            if moved_offset != offset {
                                set.push(("colId".into(), Some(moved_offset.to_string())));
                            }
                        }
                        None => return Ok(Tag::Drop),
                    }
                }
            }
            ("rowBreaks" | "colBreaks", "brk") => {
                if let (Some(id), true) = (get("id"), own) {
                    let index: u32 = id.trim().parse().map_err(|_| self.refused(name, id))?;
                    let axis = if parent == "rowBreaks" {
                        Axis::Rows
                    } else {
                        Axis::Columns
                    };
                    if let Shift::Band {
                        axis: edited, band, ..
                    } = *self.shift
                        && edited == axis
                    {
                        match band.index(index, axis.limit()) {
                            None => {
                                if matches!(get("man"), Some("1" | "true")) {
                                    self.manual_dropped += 1;
                                }
                                return Ok(Tag::Drop);
                            }
                            Some(moved) if moved != index => {
                                set.push(("id".into(), Some(moved.to_string())));
                            }
                            Some(_) => {}
                        }
                    }
                }
            }
            ("cellWatches", "cellWatch") => {
                if let Some(cell) = get("r").filter(|_| own) {
                    let at: CellRef = cell.parse().map_err(|_| self.refused(name, cell))?;
                    let owner = self.to_sheet.unwrap_or(self.sheet);
                    match self
                        .shift
                        .place(self.sheet, at)
                        .filter(|(sheet, _)| same_sheet(sheet, owner))
                    {
                        None => return Ok(Tag::Drop),
                        Some((_, moved)) => {
                            self.transferred |= self.to_sheet.is_some();
                            if moved != at {
                                set.push(("r".into(), Some(moved.to_string())));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(if set.is_empty() {
            Tag::Keep
        } else {
            Tag::Set(set)
        })
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        let name = path.last().map_or("", SmolStr::as_str);
        let in_x14 = path.first().is_some_and(|root| root == "extLst");
        match name {
            "formula" | "formula1" | "formula2" if !in_x14 => self.formula(text),
            "calculatedColumnFormula" | "totalsRowFormula" if self.table.is_some() => {
                self.formula(text)
            }
            "f" if in_x14 => {
                if path.iter().any(|name| name == "sparkline") {
                    let host = Host::fixed(self.sheet, CellRef::new(0, 0));
                    return adjust_formula_text(text, &host, self.shift);
                }
                self.formula(text)
            }
            "sqref" if in_x14 && self.shift.moves_cells_of(self.sheet) => {
                Ok(self.ranges(text)?.filter(|moved| moved != text))
            }
            _ => Ok(None),
        }
    }

    fn end(
        &mut self,
        path: &[SmolStr],
        children: usize,
        dropped: usize,
        attributes: &[(SmolStr, String)],
    ) -> Tag {
        let name = path.last().map_or("", SmolStr::as_str);
        if name == "tableColumn" {
            self.table_column = self.table_column.saturating_add(1);
            // The editor calls end before text for a formula leaf.
            self.host = None;
        }
        if matches!(name, "autoFilter") {
            self.filter = None;
        }
        if dropped == 0 {
            return Tag::Keep;
        }
        // A list the schema holds to one member at least goes with its last.
        if dropped == children
            && matches!(
                name,
                "hyperlinks"
                    | "dataValidations"
                    | "protectedRanges"
                    | "ignoredErrors"
                    | "cellWatches"
                    | "conditionalFormattings"
                    | "sparklines"
                    | "sparklineGroup"
                    | "sparklineGroups"
                    | "ext"
                    | "extLst"
            )
        {
            return Tag::Drop;
        }
        let mut set = Vec::new();
        let counted = |key: &str, less: usize| {
            attributes
                .iter()
                .find(|(held, _)| held == key)
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .map(|count| {
                    (
                        SmolStr::from(key),
                        Some(count.saturating_sub(less).to_string()),
                    )
                })
        };
        set.extend(counted("count", dropped));
        if matches!(name, "rowBreaks" | "colBreaks") {
            set.extend(counted("manualBreakCount", self.manual_dropped));
            self.manual_dropped = 0;
        }
        if set.is_empty() {
            Tag::Keep
        } else {
            Tag::Set(set)
        }
    }
}

/// `range` without its last `totals` rows, `None` when they are all of it.
fn without_totals(range: CellRange, totals: u32) -> Option<CellRange> {
    (totals < range.row_size()).then(|| {
        CellRange::new(
            range.start(),
            CellRef::new(range.end().row() - totals, range.end().column()),
        )
    })
}

impl SheetEdits<'_, '_> {
    fn refused(&self, element: &str, value: &str) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}#{element}", self.part),
            reason: format_smolstr!("expected a reference in <{element}>, got {value:?}"),
        }
    }
}

/// The edits of a drawing part: each shape's cell link (`textlink`) read as
/// a formula held at `A1` of the sheet the drawing is on, and - where the
/// shift is a `band` opening or closing that sheet's rows or columns - each
/// two-cell anchor's corners moving with their cells: an anchor inside a
/// removed band to the band's start, its offset zero, a one-cell anchor and
/// one stating `editAs="oneCell"` keeping its size, one stating
/// `editAs="absolute"` where it is. An absolute anchor names no cell.
pub(crate) struct DrawingEdits<'s, 'a> {
    shift: &'s Shift<'a>,
    /// The sheet the drawing is on, `""` where no one asked: a link naming
    /// no sheet is then left as it is.
    sheet: &'s str,
    band: Option<(Axis, Band)>,
    /// The anchor being read: how it moves, and how far its first corner
    /// moved.
    anchor: Option<AnchorKind>,
    moved_by: i64,
    /// Whether the corner being read moved into a removed band's start, so
    /// its offset along the axis is zero.
    zeroed: bool,
}

/// How a drawing anchor follows its cells.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AnchorKind {
    TwoCell,
    OneCell,
    Absolute,
}

impl<'s, 'a> DrawingEdits<'s, 'a> {
    /// The edits of a drawing on the sheet `sheet` under `shift`, its
    /// anchors moving when the shift is a band of that sheet.
    pub(crate) fn new(shift: &'s Shift<'a>, sheet: &'s str) -> Self {
        let band = match *shift {
            Shift::Band {
                sheet: edited,
                axis,
                band,
            } if same_sheet(sheet, edited) => Some((axis, band)),
            _ => None,
        };
        Self {
            shift,
            sheet,
            band,
            anchor: None,
            moved_by: 0,
            zeroed: false,
        }
    }
}

impl Edits for DrawingEdits<'_, '_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        if path.last().is_some_and(|name| name == "sp") {
            let link = attributes
                .iter()
                .find(|(key, value)| key == "textlink" && !value.trim().is_empty());
            if let Some((_, link)) = link {
                let host = Host::fixed(self.sheet, CellRef::new(0, 0));
                if let Some(moved) = adjust_formula_text(link, &host, self.shift)? {
                    return Ok(Tag::Set(vec![("textlink".into(), Some(moved))]));
                }
            }
            return Ok(Tag::Keep);
        }
        match path.last().map(SmolStr::as_str) {
            Some("twoCellAnchor") => {
                let edit_as = attributes
                    .iter()
                    .find(|(key, _)| key == "editAs")
                    .map(|(_, value)| value.as_str());
                self.anchor = Some(match edit_as {
                    Some("oneCell") => AnchorKind::OneCell,
                    Some("absolute") => AnchorKind::Absolute,
                    _ => AnchorKind::TwoCell,
                });
                self.moved_by = 0;
            }
            Some("oneCellAnchor") => {
                self.anchor = Some(AnchorKind::OneCell);
                self.moved_by = 0;
            }
            Some("absoluteAnchor") => self.anchor = None,
            _ => {}
        }
        Ok(Tag::Keep)
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        let (Some(anchor), Some((axis, band))) = (self.anchor, self.band) else {
            return Ok(None);
        };
        let [.., corner, name] = path else {
            return Ok(None);
        };
        let (index_name, offset_name) = match axis {
            Axis::Rows => ("row", "rowOff"),
            Axis::Columns => ("col", "colOff"),
        };
        if !matches!(corner.as_str(), "from" | "to") || anchor == AnchorKind::Absolute {
            return Ok(None);
        }
        if name == offset_name {
            return Ok((self.zeroed && text.trim() != "0").then(|| "0".to_owned()));
        }
        if name != index_name {
            return Ok(None);
        }
        let Ok(index) = text.trim().parse::<u32>() else {
            return Ok(None);
        };
        let limit = axis.limit();
        let moved = if corner == "to" && anchor == AnchorKind::OneCell {
            self.zeroed = false;
            let moved = i64::from(index) + self.moved_by;
            moved.clamp(0, i64::from(limit - 1)) as u32
        } else {
            let (moved, zeroed) = band.anchor(index, limit);
            self.zeroed = zeroed;
            if corner == "from" {
                self.moved_by = i64::from(moved) - i64::from(index);
            }
            moved
        };
        Ok((moved != index).then(|| moved.to_string()))
    }
}

/// The edits of a legacy VML drawing for one row or column band. A
/// note's box follows its owning cell by the same index delta at both
/// corners. Genuine controls refuse; foreign namespace lookalikes stay exact.
pub(crate) struct VmlEdits<'a> {
    axis: Axis,
    band: Band,
    shape: usize,
    shape_depth: Option<usize>,
    client_depth: Option<usize>,
    cells: Vec<VmlNote>,
    field: VmlField,
    part: &'a str,
}

#[derive(Default)]
struct VmlNote {
    note: bool,
    client_data: bool,
    owners: [Option<u32>; 2],
    anchor: Option<[i64; 8]>,
    /// Presence is separate from parsed content: `<Anchor/>` has no text
    /// callback, and two empty tags must still be refused as ambiguous.
    seen: [bool; 3],
}

impl VmlNote {
    fn owner(&self, axis: Axis) -> Option<u32> {
        self.owners[match axis {
            Axis::Rows => 0,
            Axis::Columns => 1,
        }]
    }

    fn anchor_error(part: &str) -> Error {
        Error::InvalidRecord { path: part.into(), reason:
            "expected eight VML anchor integers with in-grid coordinates and nonnegative offsets".into() }
    }

    fn start_field(&mut self, field: VmlField, part: &str) -> Result<()> {
        let (index, name) = match field {
            VmlField::Owner(Axis::Rows) => (0, "Row"),
            VmlField::Owner(Axis::Columns) => (1, "Column"),
            VmlField::Anchor => (2, "Anchor"),
            VmlField::None => return Ok(()),
        };
        if self.seen[index] {
            return Err(Error::InvalidRecord {
                path: part.into(),
                reason: format_smolstr!("expected one {name} for a VML note"),
            });
        }
        self.seen[index] = true;
        Ok(())
    }

    fn capture(&mut self, field: VmlField, text: &str, part: &str) -> Result<()> {
        match field {
            VmlField::Owner(axis) => {
                let owner = text
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value < axis.limit())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: part.into(),
                        reason: "expected a VML note owner within the grid".into(),
                    })?;
                self.owners[match axis {
                    Axis::Rows => 0,
                    Axis::Columns => 1,
                }] = Some(owner);
            }
            VmlField::Anchor => {
                let mut numbers = [0_i64; 8];
                let mut values = text.split(',');
                for number in &mut numbers {
                    *number = values
                        .next()
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .filter(|value| *value >= 0)
                        .ok_or_else(|| Self::anchor_error(part))?;
                }
                if values.next().is_some()
                    || [0, 4]
                        .into_iter()
                        .any(|at| numbers[at] >= i64::from(MAX_COLUMNS))
                    || [2, 6]
                        .into_iter()
                        .any(|at| numbers[at] >= i64::from(MAX_ROWS))
                {
                    return Err(Self::anchor_error(part));
                }
                self.anchor = Some(numbers);
            }
            VmlField::None => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
enum VmlField {
    #[default]
    None,
    Owner(Axis),
    Anchor,
}

impl VmlField {
    fn of(path: &[SmolStr], namespace: quick_xml::name::ResolveResult<'_>) -> Self {
        if !matches!(namespace, quick_xml::name::ResolveResult::Bound(value)
            if value.as_ref() == super::workbook::VML_EXCEL_NAMESPACE.as_bytes())
            || path.len() < 3
            || path[path.len() - 2] != "ClientData"
            || path[path.len() - 3] != "shape"
        {
            return Self::None;
        }
        match path.last().map(SmolStr::as_str) {
            Some("Row") => Self::Owner(Axis::Rows),
            Some("Column") => Self::Owner(Axis::Columns),
            Some("Anchor") => Self::Anchor,
            _ => Self::None,
        }
    }
}

impl<'a> VmlEdits<'a> {
    /// Rewrite a drawing or refuse unrepresentable geometry before publication.
    ///
    /// # Errors
    ///
    /// Returns a located error for malformed XML, controls, ambiguous note
    /// metadata, an absent or malformed anchor, or an anchor leaving the grid.
    /// Absolute-style geometry has no portable font and display metrics from
    /// which to derive a missing cell anchor.
    pub(crate) fn apply(
        bytes: &[u8],
        axis: Axis,
        band: Band,
        part: &'a str,
    ) -> Result<Option<Vec<u8>>> {
        let refused = |error| match error {
            Error::Codec { .. } => Error::unsupported(
                "moving the notes of a VML drawing that is not well-formed XML",
                part,
            ),
            other => other,
        };
        let mut gather = GatherNotes {
            shape_depth: None,
            client_depth: None,
            cells: Vec::new(),
            field: VmlField::None,
            part,
        };
        edit_document(bytes, &mut gather).map_err(refused)?;
        // Worksheet controls already block structural edits. A control carried
        // only in legacyDrawing has the same unsupported geometry boundary.
        if gather
            .cells
            .iter()
            .any(|note| note.client_data && !note.note)
        {
            return Err(Error::unsupported(
                "moving a VML control with unmodeled geometry",
                part,
            ));
        }
        for note in gather.cells.iter().filter(|note| note.note) {
            if note.seen[..2]
                .iter()
                .zip(&note.owners)
                .any(|(seen, owner)| *seen && owner.is_none())
            {
                return Err(Error::InvalidRecord {
                    path: part.into(),
                    reason: "expected a VML note owner within the grid".into(),
                });
            }
            let owner = note.owner(axis).ok_or_else(|| Error::InvalidRecord {
                path: part.into(),
                reason: "expected a VML note owner within the grid".into(),
            })?;
            if note.seen[2] && note.anchor.is_none() {
                return Err(VmlNote::anchor_error(part));
            }
            if band.index(owner, axis.limit()).is_some() && note.anchor.is_none() {
                return Err(Error::unsupported(
                    "moving a VML note without a cell anchor",
                    part,
                ));
            }
        }
        let mut edits = Self {
            axis,
            band,
            shape: 0,
            shape_depth: None,
            client_depth: None,
            cells: gather.cells,
            field: VmlField::None,
            part,
        };
        edit_document(bytes, &mut edits).map_err(refused)
    }
}

/// The existing XML walk resolves and validates note geometry once before
/// rewriting. The second pass uses the captured integers, never re-parses them.
struct GatherNotes<'a> {
    shape_depth: Option<usize>,
    client_depth: Option<usize>,
    cells: Vec<VmlNote>,
    field: VmlField,
    part: &'a str,
}

impl Edits for GatherNotes<'_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        if path.last().is_some_and(|name| name == "shape")
            && matches!(&namespace, quick_xml::name::ResolveResult::Bound(value)
                if value.as_ref() == super::workbook::VML_NAMESPACE.as_bytes())
        {
            if self.shape_depth.is_some() {
                return Err(Error::unsupported("moving a nested VML shape", self.part));
            }
            self.shape_depth = Some(path.len());
            self.cells.push(VmlNote::default());
        }
        if self
            .shape_depth
            .is_some_and(|depth| path.len() == depth + 1)
            && path[path.len() - 2] == "shape"
            && path.last().is_some_and(|name| name == "ClientData")
            && matches!(&namespace, quick_xml::name::ResolveResult::Bound(value)
                if value.as_ref() == super::workbook::VML_EXCEL_NAMESPACE.as_bytes())
            && let Some(last) = self.cells.last_mut()
        {
            if last.client_data {
                return Err(Error::InvalidRecord {
                    path: self.part.into(),
                    reason: "expected one ClientData for a VML shape".into(),
                });
            }
            last.client_data = true;
            last.note = attributes
                .iter()
                .any(|(key, value)| key == "ObjectType" && value == "Note");
            self.client_depth = Some(path.len());
        }
        self.field = if self
            .client_depth
            .is_some_and(|depth| path.len() == depth + 1)
        {
            VmlField::of(path, namespace)
        } else {
            VmlField::None
        };
        if let Some(last) = self.cells.last_mut().filter(|note| note.note) {
            last.start_field(self.field, self.part)?;
        }
        Ok(Tag::Keep)
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        if self
            .client_depth
            .is_some_and(|depth| path.len() == depth + 1)
            && let Some(last) = self.cells.last_mut().filter(|note| note.note)
        {
            last.capture(self.field, text, self.part)?;
        }
        Ok(None)
    }

    fn end(&mut self, path: &[SmolStr], _: usize, _: usize, _: &[(SmolStr, String)]) -> Tag {
        if self.client_depth == Some(path.len()) {
            self.client_depth = None;
            self.field = VmlField::None;
        }
        if self.shape_depth == Some(path.len()) {
            self.shape_depth = None;
            self.field = VmlField::None;
        }
        Tag::Keep
    }
}

impl Edits for VmlEdits<'_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        _: &[(SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        if self
            .shape_depth
            .is_some_and(|depth| path.len() == depth + 1)
            && path.last().is_some_and(|name| name == "ClientData")
            && matches!(&namespace, quick_xml::name::ResolveResult::Bound(value)
                if value.as_ref() == super::workbook::VML_EXCEL_NAMESPACE.as_bytes())
        {
            self.client_depth = Some(path.len());
        }
        self.field = if self
            .client_depth
            .is_some_and(|depth| path.len() == depth + 1)
        {
            VmlField::of(path, namespace.clone())
        } else {
            VmlField::None
        };
        if path.last().is_some_and(|name| name == "shape")
            && matches!(namespace, quick_xml::name::ResolveResult::Bound(value)
                if value.as_ref() == super::workbook::VML_NAMESPACE.as_bytes())
        {
            let index = self.shape;
            self.shape += 1;
            self.shape_depth = Some(path.len());
            let note = &self.cells[index];
            if note.note
                && note
                    .owner(self.axis)
                    .is_some_and(|cell| self.band.index(cell, self.axis.limit()).is_none())
            {
                self.shape_depth = None;
                return Ok(Tag::Drop);
            }
        }
        Ok(Tag::Keep)
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        if !self
            .client_depth
            .is_some_and(|depth| path.len() == depth + 1)
            || matches!(self.field, VmlField::None)
        {
            return Ok(None);
        }
        let Some(note) = self
            .shape
            .checked_sub(1)
            .and_then(|index| self.cells.get(index))
            .filter(|note| note.note)
        else {
            return Ok(None);
        };
        let owner = note.owner(self.axis).expect("note intake proved its owner");
        let moved = self
            .band
            .index(owner, self.axis.limit())
            .expect("a removed note was dropped");
        match self.field {
            VmlField::Owner(axis) => {
                Ok((axis == self.axis && owner != moved).then(|| moved.to_string()))
            }
            VmlField::Anchor => {
                let mut numbers = note
                    .anchor
                    .expect("intake proved a surviving note's anchor");
                let delta = i64::from(moved) - i64::from(owner);
                if delta == 0 {
                    return Ok(None);
                }
                let corners = match self.axis {
                    Axis::Rows => [2, 6],
                    Axis::Columns => [0, 4],
                };
                for index in corners {
                    let Some(value) = numbers[index]
                        .checked_add(delta)
                        .filter(|value| (0..i64::from(self.axis.limit())).contains(value))
                    else {
                        return Err(Error::unsupported(
                            "moving a VML note anchor outside the worksheet grid",
                            self.part,
                        ));
                    };
                    numbers[index] = value;
                }
                let lead: String = text.chars().take_while(|c| c.is_whitespace()).collect();
                Ok(Some(format!(
                    "{lead}{}, {}, {}, {}, {}, {}, {}, {}",
                    numbers[0],
                    numbers[1],
                    numbers[2],
                    numbers[3],
                    numbers[4],
                    numbers[5],
                    numbers[6],
                    numbers[7]
                )))
            }
            VmlField::None => Ok(None),
        }
    }

    fn end(&mut self, path: &[SmolStr], _: usize, _: usize, _: &[(SmolStr, String)]) -> Tag {
        if self.client_depth == Some(path.len()) {
            self.client_depth = None;
            self.field = VmlField::None;
        }
        if self.shape_depth == Some(path.len()) {
            self.shape_depth = None;
            self.field = VmlField::None;
        }
        Tag::Keep
    }
}

/// The edits of a chart: every series formula (`c:f`) rewritten as it would
/// be held at `A1` of no sheet - a chart names each sheet it reads.
pub(crate) struct ChartEdits<'s, 'a> {
    pub(crate) shift: &'s Shift<'a>,
}

impl Edits for ChartEdits<'_, '_> {
    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        if path.last().is_some_and(|name| name == "f") {
            let host = Host::fixed("", CellRef::new(0, 0));
            return adjust_formula_text(text, &host, self.shift);
        }
        Ok(None)
    }
}

/// The edits of a pivot cache: the sheet and range its worksheet source
/// names.
pub(crate) struct CacheEdits<'s, 'a> {
    pub(crate) shift: &'s Shift<'a>,
    pub(crate) part: &'s str,
}

impl Edits for CacheEdits<'_, '_> {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        if path.last().is_none_or(|name| name != "worksheetSource") {
            return Ok(Tag::Keep);
        }
        let get = |name: &str| {
            attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let Some(sheet) = get("sheet") else {
            return Ok(Tag::Keep);
        };
        let mut set = Vec::new();
        match *self.shift {
            Shift::RenameSheet { from, to } if same_sheet(sheet, from) => {
                set.push(("sheet".into(), Some(to.to_owned())));
            }
            Shift::Band { .. } | Shift::Move { .. } => {
                if let Some(range) = get("ref") {
                    let parsed: CellRange = range.parse().map_err(|_| Error::InvalidRecord {
                        path: SmolStr::new(self.part),
                        reason: format_smolstr!(
                            "expected a range in <worksheetSource>, got {range:?}"
                        ),
                    })?;
                    match adjust_range(parsed, sheet, self.shift) {
                        None => {
                            return Err(Error::InvalidRecord {
                                path: SmolStr::new(self.part),
                                reason: format_smolstr!(
                                    "expected the edit to keep a cell of the pivot cache's source \
                                     {sheet}!{range}, got one taking all of it"
                                ),
                            });
                        }
                        Some(moved) if moved != parsed => {
                            set.push(("ref".into(), Some(range_text(moved))));
                        }
                        Some(_) => {}
                    }
                }
            }
            _ => {}
        }
        Ok(if set.is_empty() {
            Tag::Keep
        } else {
            Tag::Set(set)
        })
    }
}

/// The references a part beside the sheets states by sheet name, as text:
/// each formula element's (`c:f`, `cx:f`), each shape's cell link
/// (`textlink`), and the sheet a pivot cache's source is on - what decides
/// whether an edit reaches the part, without the part.
#[derive(Default)]
pub(crate) struct References {
    text: String,
}

impl References {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The references gathered, one per line.
    pub(crate) fn into_text(self) -> String {
        self.text
    }

    fn push(&mut self, reference: &str) {
        self.text.push_str(reference);
        self.text.push('\n');
    }
}

impl Edits for References {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let wanted = match path.last().map(SmolStr::as_str) {
            Some("sp") => "textlink",
            Some("worksheetSource") => "sheet",
            _ => return Ok(Tag::Keep),
        };
        if let Some((_, value)) = attributes.iter().find(|(key, _)| key == wanted) {
            self.push(value);
        }
        Ok(Tag::Keep)
    }

    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        if path.last().is_some_and(|name| {
            matches!(
                name.as_str(),
                "f" | "calculatedColumnFormula" | "totalsRowFormula"
            )
        }) {
            self.push(text);
        }
        Ok(None)
    }
}

/// What a slicer or timeline cache names: the tabs - by `sheetId` - of the
/// pivot tables it filters, and the ids of the tables.
#[derive(Default)]
pub(crate) struct CacheSources {
    pub(crate) tabs: Vec<u32>,
    pub(crate) tables: Vec<u32>,
}

impl CacheSources {
    /// Read the cache part `bytes`, which `part` names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the part for one that is not
    /// well-formed or states an id that is no number.
    pub(crate) fn read(bytes: &[u8], part: &str) -> Result<Self> {
        let mut sources = Self::default();
        edit_document(bytes, &mut sources).map_err(|error| Error::InvalidRecord {
            path: SmolStr::new(part),
            reason: format_smolstr!("expected a slicer or timeline cache, got {error}"),
        })?;
        Ok(sources)
    }
}

impl Edits for CacheSources {
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        _: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let (wanted, into) = match path.last().map(SmolStr::as_str) {
            Some("pivotTable") => ("tabId", &mut self.tabs),
            Some("tableSlicerCache") => ("tableId", &mut self.tables),
            _ => return Ok(Tag::Keep),
        };
        if let Some((_, value)) = attributes.iter().find(|(key, _)| key == wanted) {
            into.push(value.trim().parse().map_err(|_| Error::InvalidRecord {
                path: SmolStr::new(wanted),
                reason: format_smolstr!("expected a number, got {value:?}"),
            })?);
        }
        Ok(Tag::Keep)
    }
}

impl Table {
    /// Where the column's formula is spelled: its first data or totals row.
    /// A dormant template uses the row where data or totals would appear,
    /// even when that row is just beyond the table's current range.
    fn formula_host(
        &self,
        range: CellRange,
        column: u32,
        totals: bool,
        part: &str,
    ) -> Result<CellRef> {
        let rows = u64::from(self.header_rows) + u64::from(self.totals_rows);
        if column >= range.column_size() || rows > u64::from(range.row_size()) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(part),
                reason: format_smolstr!(
                    "expected a {} formula inside column {} of table {} at {range}, got {} \
                     header and {} totals rows",
                    if totals { "totals" } else { "calculated" },
                    u64::from(column) + 1,
                    self.name,
                    self.header_rows,
                    self.totals_rows
                ),
            });
        }
        let row = if totals {
            range.end().row() + 1 - self.totals_rows
        } else {
            range.start().row() + self.header_rows
        };
        let host = CellRef::new(row, range.start().column() + column);
        host.require_in_grid().map_err(|_| Error::InvalidRecord {
            path: SmolStr::new(part),
            reason: format_smolstr!(
                "expected a formula host inside the worksheet grid for table {}, got {host}",
                self.name
            ),
        })
    }

    /// Why `band` along `axis` cannot pass through the table, `None` when it
    /// can: columns opening inside it or taking any of it, rows taking its
    /// header, its totals or every data row - and a table whose header and
    /// totals rows the part states as more rows than it has, whose rows no
    /// edit can tell apart, or an insertion moving its end off the grid.
    pub(crate) fn refusal(&self, axis: Axis, band: Band) -> Option<SmolStr> {
        let (first, last) = axis.span(self.range);
        let height = u64::from(self.range.row_size());
        if u64::from(self.header_rows) + u64::from(self.totals_rows) > height {
            return Some(format_smolstr!(
                "expected the table {} to state at most {height} header and totals rows, got {} \
                 and {}",
                self.name,
                self.header_rows,
                self.totals_rows
            ));
        }
        match (axis, band) {
            (Axis::Columns, Band::Insert { .. }) if band.splits(first, last) => {
                Some(format_smolstr!(
                    "expected columns opening outside the table {}, got some inside it",
                    self.name
                ))
            }
            (Axis::Columns, Band::Remove { .. }) if band.meets(first, last) => {
                Some(format_smolstr!(
                    "expected columns outside the table {}, got some of its columns",
                    self.name
                ))
            }
            (_, Band::Insert { at, count })
                if at <= last
                    && last
                        .checked_add(count)
                        .is_none_or(|end| end >= axis.limit()) =>
            {
                Some(format_smolstr!(
                    "expected the table {} to stay within {} {}, got its last index moving by {count}",
                    self.name,
                    axis.limit(),
                    axis.noun()
                ))
            }
            (Axis::Rows, Band::Remove { start, end }) => {
                let takes = |from: u32, to: u32| from <= to && start <= to && end > from;
                let header = self.header_rows > 0 && takes(first, first + self.header_rows - 1);
                let totals =
                    self.totals_rows > 0 && takes(last + 1 - self.totals_rows.min(last + 1), last);
                let data_first = first + self.header_rows;
                let data_last = last.saturating_sub(self.totals_rows);
                let every_row = data_first <= data_last && start <= data_first && end > data_last;
                (header || totals || every_row).then(|| {
                    format_smolstr!(
                        "expected rows leaving the header, the totals and a data row of the table \
                         {}, got {}",
                        self.name,
                        if header {
                            "its header row"
                        } else if totals {
                            "its totals row"
                        } else {
                            "every data row"
                        }
                    )
                })
            }
            _ => None,
        }
    }
}

/// The range a pivot table part states its report fills.
///
/// # Errors
///
/// Returns the refusal of a part that is not well-formed or states no
/// location.
pub(crate) fn pivot_location(bytes: &[u8], part: &str) -> Result<CellRange> {
    element_attributes(bytes, &["pivotTableDefinition", "location"], part)?
        .into_iter()
        .find(|(key, _)| key == "ref")
        .and_then(|(_, range)| range.parse::<CellRange>().ok())
        .ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new(part),
            reason: SmolStr::new_static("expected a pivot table stating its location, got none"),
        })
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Sparse carried-range partition geometry pinned without opening a package.

    use super::{CarriedFormulaKind, CellRange, CellRef, Formula, MoveNames, Shift};

    /// Native carried-host policies without XML serialization or cell expansion.
    pub fn carried_formula_regions(
        conditional: bool,
        from: (&str, CellRange),
        to: (&str, CellRef),
        owner: (&str, Option<&str>),
        ranges: &[CellRange],
        formulas: &[Formula],
    ) -> crate::Result<Vec<(Vec<CellRange>, Vec<Formula>)>> {
        let names = MoveNames::default();
        let shift = Shift::Move {
            from: from.0,
            block: from.1,
            to: to.0,
            target: to.1,
            names: &names,
        };
        let kind = if conditional {
            CarriedFormulaKind::Conditional
        } else {
            CarriedFormulaKind::Validation
        };
        shift
            .formula_regions(kind, owner, ranges, formulas, "worksheet#rule")
            .map(|regions| {
                regions
                    .into_iter()
                    .map(|region| (region.ranges, region.formulas.into_vec()))
                    .collect()
            })
    }

    /// The selected rectangle and at most four disjoint remainder strips.
    pub fn range_partition(
        range: CellRange,
        cut: CellRange,
    ) -> (Option<CellRange>, [Option<CellRange>; 4]) {
        let (selected, remainder) = range.partition(cut);
        (
            selected,
            std::array::from_fn(|index| remainder.get(index).copied()),
        )
    }
}
