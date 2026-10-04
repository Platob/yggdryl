//! A reference inside a formula: which cells it names, held relative to the
//! cell whose formula holds it, and the sheet it names them on.
//!
//! A coordinate written without `$` is stored as its distance from the host
//! cell ([`Coord::Relative`]), one written with `$` as the index itself
//! ([`Coord::Absolute`]), so one [`Reference`] renders the translated text at
//! every host a formula is copied, filled or shared to, and a shared
//! formula's dependents hold their master's shape unchanged. Rendering
//! spells a reference as Excel normalizes one: column letters upper case,
//! a sheet name quoted only where it has to be (`'Q1 ''24'!A1`), and a
//! coordinate that falls off the grid as `#REF!`.

use std::fmt;

use smol_str::SmolStr;

use crate::excel::cell::{CellRange, CellRef, MAX_COLUMNS, MAX_ROWS};

/// One coordinate of a reference: a row or a column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Coord {
    /// Written without `$`: the distance from the host's row or column.
    Relative(i32),
    /// Written with `$`: the zero-based row or column itself.
    Absolute(u32),
}

impl Coord {
    /// The coordinate naming zero-based `index` from a host at `host`,
    /// absolute when written with `$`.
    pub(crate) fn of(index: u32, host: u32, absolute: bool) -> Self {
        if absolute {
            Self::Absolute(index)
        } else {
            // Both lie within the grid, whose bounds fit an `i32`.
            Self::Relative(index as i32 - host as i32)
        }
    }

    /// The zero-based index at a host at `host`, `None` off a grid of
    /// `limit` rows or columns.
    pub(crate) fn resolve(self, host: u32, limit: u32) -> Option<u32> {
        let index = match self {
            Self::Absolute(index) => i64::from(index),
            Self::Relative(offset) => i64::from(host) + i64::from(offset),
        };
        u32::try_from(index).ok().filter(|index| *index < limit)
    }

    /// The inclusive host interval on which this coordinate falls inside
    /// `first..=last`, after translating the host by `delta`. Arithmetic is
    /// wider than the grid so negative relative offsets cannot wrap.
    fn hosts_in(self, first: u32, last: u32, limit: u32, delta: i32) -> Option<(u32, u32)> {
        match self {
            Self::Absolute(index) => (first <= index && index <= last).then_some((0, limit - 1)),
            Self::Relative(offset) => {
                let offset = i64::from(offset) + i64::from(delta);
                let first = (i64::from(first) - offset).max(0);
                let last = (i64::from(last) - offset).min(i64::from(limit) - 1);
                (first <= last).then_some((first as u32, last as u32))
            }
        }
    }

    /// Whether the coordinate was written with `$`.
    pub(crate) const fn is_absolute(self) -> bool {
        matches!(self, Self::Absolute(_))
    }
}

/// The cells a reference names, relative to its host.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    /// One cell: `A1`, `$B$2`.
    Cell { row: Coord, column: Coord },
    /// A rectangle between two corners: `A1:C3`.
    Area {
        first: (Coord, Coord),
        last: (Coord, Coord),
    },
    /// Whole rows: `1:3`.
    Rows { first: Coord, last: Coord },
    /// Whole columns: `A:C`.
    Columns { first: Coord, last: Coord },
    /// A name defined on the sheet the prefix names: `Sheet1!Print_Area`.
    Name(SmolStr),
    /// A reference that names no cell any more: `#REF!`.
    Invalid,
}

impl Target {
    /// Resolve geometry through the coordinate owner, without expanding a
    /// whole row or column. Reversed authored endpoints canonicalize here;
    /// the structural adjuster's Place retains their orientation separately.
    pub(crate) fn range(&self, host: CellRef) -> Option<CellRange> {
        let cell = |(row, column): (Coord, Coord)| {
            Some(CellRef::new(
                row.resolve(host.row(), MAX_ROWS)?,
                column.resolve(host.column(), MAX_COLUMNS)?,
            ))
        };
        let (first, last) = match self {
            Self::Cell { row, column } => {
                let at = cell((*row, *column))?;
                (at, at)
            }
            Self::Area { first, last } => (cell(*first)?, cell(*last)?),
            Self::Rows { first, last } => (
                CellRef::new(first.resolve(host.row(), MAX_ROWS)?, 0),
                CellRef::new(last.resolve(host.row(), MAX_ROWS)?, MAX_COLUMNS - 1),
            ),
            Self::Columns { first, last } => (
                CellRef::new(0, first.resolve(host.column(), MAX_COLUMNS)?),
                CellRef::new(MAX_ROWS - 1, last.resolve(host.column(), MAX_COLUMNS)?),
            ),
            Self::Name(_) | Self::Invalid => return None,
        };
        Some(CellRange::new(first, last))
    }

    /// Whether a rectangular or whole-axis reference changes coordinates with
    /// its implicit host. Cell references use the separate scalar cut policy.
    pub(crate) fn is_relative_area(&self) -> bool {
        match self {
            Self::Area { first, last } => [first.0, first.1, last.0, last.1]
                .into_iter()
                .any(|coordinate| !coordinate.is_absolute()),
            Self::Rows { first, last } | Self::Columns { first, last } => {
                !first.is_absolute() || !last.is_absolute()
            }
            Self::Cell { .. } | Self::Name(_) | Self::Invalid => false,
        }
    }

    #[cfg(feature = "internals")]
    pub(crate) fn is_relative(&self) -> bool {
        match self {
            Self::Cell { row, column } => !row.is_absolute() || !column.is_absolute(),
            _ => self.is_relative_area(),
        }
    }

    /// Host rectangle on which every target coordinate is inside `range`.
    /// Whole-row/column references constrain only their represented axis;
    /// names and invalid references have no coordinate preimage.
    pub(crate) fn hosts_in(&self, range: CellRange, delta: (i32, i32)) -> Option<CellRange> {
        let mut first = CellRef::new(0, 0);
        let mut last = CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1);
        let mut row = |coordinate: Coord| -> Option<()> {
            let (start, end) =
                coordinate.hosts_in(range.start().row(), range.end().row(), MAX_ROWS, delta.0)?;
            first = CellRef::new(first.row().max(start), first.column());
            last = CellRef::new(last.row().min(end), last.column());
            (first.row() <= last.row()).then_some(())
        };
        match self {
            Self::Cell { row: at, .. } => row(*at)?,
            Self::Area { first, last } => {
                row(first.0)?;
                row(last.0)?;
            }
            Self::Rows { first, last } => {
                row(*first)?;
                row(*last)?;
            }
            Self::Columns { .. } => {}
            Self::Name(_) | Self::Invalid => return None,
        }
        let mut column = |coordinate: Coord| -> Option<()> {
            let (start, end) = coordinate.hosts_in(
                range.start().column(),
                range.end().column(),
                MAX_COLUMNS,
                delta.1,
            )?;
            first = CellRef::new(first.row(), first.column().max(start));
            last = CellRef::new(last.row(), last.column().min(end));
            (first.column() <= last.column()).then_some(())
        };
        match self {
            Self::Cell { column: at, .. } => column(*at)?,
            Self::Area { first, last } => {
                column(first.1)?;
                column(last.1)?;
            }
            Self::Columns { first, last } => {
                column(*first)?;
                column(*last)?;
            }
            Self::Rows { .. } => {}
            Self::Name(_) | Self::Invalid => return None,
        }
        Some(CellRange::new(first, last))
    }
}

/// The sheet a reference names its cells on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SheetSpec {
    /// No prefix: the sheet holding the formula.
    Own,
    /// One sheet by name; `quoted` when the file quoted it, which rendering
    /// keeps.
    Named { name: SmolStr, quoted: bool },
    /// Every sheet from `first` to `last` in tab order: `Jan:Mar!B2`.
    Span {
        first: SmolStr,
        last: SmolStr,
        quoted: bool,
    },
    /// A sheet of another workbook, spelled as the file spelled it before
    /// the `!`: `[1]Sheet1`, `'[1]Q 1'`.
    External(SmolStr),
    /// A sheet that is gone: `#REF!`.
    Invalid,
}

impl SheetSpec {
    /// Whether the prefix names the sheet `name`, compared as Excel compares
    /// sheet names.
    pub(crate) fn names(&self, name: &str) -> bool {
        match self {
            Self::Named { name: held, .. } => same_sheet(held, name),
            Self::Span { first, last, .. } => same_sheet(first, name) || same_sheet(last, name),
            Self::Own | Self::External(_) | Self::Invalid => false,
        }
    }
}

/// Whether two sheet names name one sheet: compared without case, as the
/// workbook compares the names of its tabs.
pub(crate) fn same_sheet(first: &str, second: &str) -> bool {
    first.eq_ignore_ascii_case(second)
}

/// A reference: the sheet, then the cells on it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Reference {
    pub(crate) sheet: SheetSpec,
    pub(crate) target: Target,
}

impl Reference {
    /// The reference spelled at `host` in A1 notation.
    pub(crate) fn a1(&self, host: CellRef) -> impl fmt::Display + '_ {
        Spelled {
            reference: self,
            host,
            r1c1: false,
        }
    }

    /// The reference spelled in R1C1 notation, which reads the same at
    /// every host.
    pub(crate) fn r1c1(&self) -> impl fmt::Display + '_ {
        Spelled {
            reference: self,
            host: CellRef::new(0, 0),
            r1c1: true,
        }
    }
}

/// A reference spelled at one host.
struct Spelled<'a> {
    reference: &'a Reference,
    host: CellRef,
    r1c1: bool,
}

impl fmt::Display for Spelled<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_sheet(formatter, &self.reference.sheet)?;
        if self.r1c1 {
            write_target_r1c1(formatter, &self.reference.target)
        } else {
            write_target(formatter, &self.reference.target, self.host)
        }
    }
}

/// Write the prefix of `sheet`, its `!` included; nothing for the own sheet.
fn write_sheet(formatter: &mut fmt::Formatter<'_>, sheet: &SheetSpec) -> fmt::Result {
    match sheet {
        SheetSpec::Own => Ok(()),
        SheetSpec::Named { name, quoted } => {
            if *quoted || needs_quotes(name) {
                write_quoted(formatter, &[name])?;
            } else {
                formatter.write_str(name)?;
            }
            formatter.write_str("!")
        }
        SheetSpec::Span {
            first,
            last,
            quoted,
        } => {
            if *quoted || needs_quotes(first) || needs_quotes(last) {
                write_quoted(formatter, &[first, ":", last])?;
            } else {
                write!(formatter, "{first}:{last}")?;
            }
            formatter.write_str("!")
        }
        SheetSpec::External(prefix) => write!(formatter, "{prefix}!"),
        SheetSpec::Invalid => formatter.write_str("#REF!"),
    }
}

/// Write `pieces` inside apostrophes, each apostrophe in them doubled.
fn write_quoted(formatter: &mut fmt::Formatter<'_>, pieces: &[&str]) -> fmt::Result {
    formatter.write_str("'")?;
    for piece in pieces {
        for (at, part) in piece.split('\'').enumerate() {
            if at > 0 {
                formatter.write_str("''")?;
            }
            formatter.write_str(part)?;
        }
    }
    formatter.write_str("'")
}

/// Whether Excel quotes the sheet name `name` in a reference: anything but
/// letters, digits, `_` and `.`, a name opening with a digit or a `.`, one
/// that reads as a reference itself - `A1`, `R1C1`, `R`, `C` - and the two
/// booleans.
pub(crate) fn needs_quotes(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return true;
    };
    if first.is_ascii_digit() || first == '.' {
        return true;
    }
    if name
        .chars()
        .any(|character| !(character.is_alphanumeric() || character == '_' || character == '.'))
    {
        return true;
    }
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        return true;
    }
    reads_as_a1(name) || reads_as_r1c1(name)
}

/// Whether `name` spells an A1 cell of the grid: `A1`, `xfd1048576`.
fn reads_as_a1(name: &str) -> bool {
    let letters = name.bytes().take_while(u8::is_ascii_alphabetic).count();
    let digits = &name[letters..];
    letters > 0
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && CellRef::column_index(&name[..letters]).is_some()
        && digits
            .parse::<u32>()
            .is_ok_and(|row| (1..=MAX_ROWS).contains(&row))
}

/// Whether `name` spells an R1C1 reference: `R`, `C`, `R1`, `C2`, `R1C1`,
/// `RC`.
fn reads_as_r1c1(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let rest = upper.strip_prefix('R').map_or(upper.as_str(), |rest| {
        rest.trim_start_matches(|character: char| character.is_ascii_digit())
    });
    let rest = rest.strip_prefix('C').map_or(rest, |rest| {
        rest.trim_start_matches(|character: char| character.is_ascii_digit())
    });
    rest.is_empty() && (upper.starts_with('R') || upper.starts_with('C'))
}

/// Write `target` in A1 notation at `host`; a coordinate off the grid makes
/// the whole target `#REF!`.
fn write_target(formatter: &mut fmt::Formatter<'_>, target: &Target, host: CellRef) -> fmt::Result {
    let row = |coord: Coord| coord.resolve(host.row(), MAX_ROWS);
    let column = |coord: Coord| coord.resolve(host.column(), MAX_COLUMNS);
    match target {
        Target::Cell {
            row: at_row,
            column: at_column,
        } => match (row(*at_row), column(*at_column)) {
            (Some(index), Some(letters)) => {
                write_column(formatter, *at_column, letters)?;
                write_row(formatter, *at_row, index)
            }
            _ => formatter.write_str("#REF!"),
        },
        Target::Area { first, last } => {
            match (row(first.0), column(first.1), row(last.0), column(last.1)) {
                (Some(first_row), Some(first_column), Some(last_row), Some(last_column)) => {
                    write_column(formatter, first.1, first_column)?;
                    write_row(formatter, first.0, first_row)?;
                    formatter.write_str(":")?;
                    write_column(formatter, last.1, last_column)?;
                    write_row(formatter, last.0, last_row)
                }
                _ => formatter.write_str("#REF!"),
            }
        }
        Target::Rows { first, last } => match (row(*first), row(*last)) {
            (Some(first_row), Some(last_row)) => {
                write_row(formatter, *first, first_row)?;
                formatter.write_str(":")?;
                write_row(formatter, *last, last_row)
            }
            _ => formatter.write_str("#REF!"),
        },
        Target::Columns { first, last } => match (column(*first), column(*last)) {
            (Some(first_column), Some(last_column)) => {
                write_column(formatter, *first, first_column)?;
                formatter.write_str(":")?;
                write_column(formatter, *last, last_column)
            }
            _ => formatter.write_str("#REF!"),
        },
        Target::Name(name) => formatter.write_str(name),
        Target::Invalid => formatter.write_str("#REF!"),
    }
}

/// Write a column's letters, `$` first when absolute.
fn write_column(formatter: &mut fmt::Formatter<'_>, coord: Coord, column: u32) -> fmt::Result {
    if coord.is_absolute() {
        formatter.write_str("$")?;
    }
    let mut letters = [0_u8; 3];
    let mut at = letters.len();
    let mut rest = i64::from(column);
    loop {
        at -= 1;
        letters[at] = b'A' + (rest % 26) as u8;
        rest = rest / 26 - 1;
        if rest < 0 {
            break;
        }
    }
    formatter.write_str(std::str::from_utf8(&letters[at..]).unwrap_or("#REF!"))
}

/// Write a row's number, `$` first when absolute.
fn write_row(formatter: &mut fmt::Formatter<'_>, coord: Coord, row: u32) -> fmt::Result {
    if coord.is_absolute() {
        formatter.write_str("$")?;
    }
    write!(formatter, "{}", row + 1)
}

/// Write `target` in R1C1 notation: `R[-1]C`, `R2C3`, `R1:R3`, `C[1]:C[2]`.
fn write_target_r1c1(formatter: &mut fmt::Formatter<'_>, target: &Target) -> fmt::Result {
    let coord = |formatter: &mut fmt::Formatter<'_>, axis: char, coord: Coord| match coord {
        Coord::Absolute(index) => write!(formatter, "{axis}{}", index + 1),
        Coord::Relative(0) => write!(formatter, "{axis}"),
        Coord::Relative(offset) => write!(formatter, "{axis}[{offset}]"),
    };
    match target {
        Target::Cell { row, column } => {
            coord(formatter, 'R', *row)?;
            coord(formatter, 'C', *column)
        }
        Target::Area { first, last } => {
            coord(formatter, 'R', first.0)?;
            coord(formatter, 'C', first.1)?;
            formatter.write_str(":")?;
            coord(formatter, 'R', last.0)?;
            coord(formatter, 'C', last.1)
        }
        Target::Rows { first, last } => {
            coord(formatter, 'R', *first)?;
            formatter.write_str(":")?;
            coord(formatter, 'R', *last)
        }
        Target::Columns { first, last } => {
            coord(formatter, 'C', *first)?;
            formatter.write_str(":")?;
            coord(formatter, 'C', *last)
        }
        Target::Name(name) => formatter.write_str(name),
        Target::Invalid => formatter.write_str("#REF!"),
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Typed geometry used by the mirrored reference tests.
    use super::{CellRange, CellRef, Target};
    use crate::excel::formula::{Formula, parser::Node};

    fn target(formula: &Formula) -> Option<&Target> {
        let expression = formula.expression().ok()?;
        match &expression.nodes[expression.root] {
            Node::Reference(reference) => Some(&reference.target),
            _ => None,
        }
    }

    /// Resolve a single typed reference root; names and held roots have no area.
    #[must_use]
    pub fn range(formula: &Formula, host: CellRef) -> Option<CellRange> {
        target(formula)?.range(host)
    }

    /// Whether a single typed reference uses a host-relative coordinate.
    #[must_use]
    pub fn relative(formula: &Formula) -> Option<bool> {
        target(formula).map(Target::is_relative)
    }
}
