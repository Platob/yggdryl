//! AutoFill: the cells a fill handle drags a selection over, as a series
//! its cells spell or as copies of them.
//!
//! A fill extends its source in one direction - down, up, right or left -
//! line by line along that direction, each line of the source read on its
//! own. [`FillMode::Copy`] repeats the source, each formula translated to
//! where it lands. [`FillMode::Series`] continues what the line spells when
//! every cell of it is one kind: numbers step linearly (one number copies,
//! as Excel's plain drag does; steps that are no arithmetic series follow
//! the least-squares trend), dates step by the day, the month or the year
//! their spelling shows, times by their step, text ending in a number counts
//! on (`Item 7`, `Item 8`), a day or a month name follows the calendar in
//! the case it is written, and a quarter (`Q1`, `Qtr1`, `Quarter 1`) cycles
//! through four; any other line repeats as copies. Each cell filled takes
//! the style of the source cell it continues.

use smol_str::{SmolStr, format_smolstr};

use crate::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result, Scalar};

use super::cell::{Cell, CellKind, CellRange, CellRef, DateSystem};
use super::edit::MAX_EDITED_CELLS;
use super::format::Digits;
use super::shift::materialized;
use super::styles::NumberFormat;
use super::workbook::Workbook;

/// How a fill continues its source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillMode {
    /// The series the source's cells spell, else copies of them.
    #[default]
    Series,
    /// Copies of the source, formulas translated.
    Copy,
}

impl FillMode {
    /// The mode as the service spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Series => "series",
            Self::Copy => "copy",
        }
    }
}

/// The direction a fill extends its source in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    Down,
    Up,
    Right,
    Left,
}

/// The direction `target` extends `source` in, `None` when it is the
/// source itself.
fn way(sheet: &str, source: CellRange, target: CellRange) -> Result<Option<Way>> {
    if source == target {
        return Ok(None);
    }
    let (s, t) = (source, target);
    let same_columns =
        s.start().column() == t.start().column() && s.end().column() == t.end().column();
    let same_rows = s.start().row() == t.start().row() && s.end().row() == t.end().row();
    let found = if same_columns
        && s.start().row() == t.start().row()
        && t.end().row() > s.end().row()
    {
        Some(Way::Down)
    } else if same_columns && s.end().row() == t.end().row() && t.start().row() < s.start().row() {
        Some(Way::Up)
    } else if same_rows
        && s.start().column() == t.start().column()
        && t.end().column() > s.end().column()
    {
        Some(Way::Right)
    } else if same_rows
        && s.end().column() == t.end().column()
        && t.start().column() < s.start().column()
    {
        Some(Way::Left)
    } else {
        None
    };
    found.map(Some).ok_or_else(|| Error::InvalidRecord {
        path: format_smolstr!("{sheet}!{target}"),
        reason: format_smolstr!(
            "expected a target extending the source {source} down, up, right or left, got \
             {target}"
        ),
    })
}

impl Workbook {
    /// Fill `target` of the sheet `sheet` from `source`, which it holds and
    /// extends in one direction: as the series the source spells, or as
    /// copies of it ([`FillMode`]). Answers nothing; the cells of `target`
    /// past `source` are replaced.
    ///
    /// ```
    /// use yggdryl::excel::{FillMode, Workbook};
    /// use yggdryl::Scalar;
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Sheet1")?;
    /// sheet.set_cell("A1".parse()?, 1.0)?;
    /// sheet.set_cell("A2".parse()?, 3.0)?;
    /// sheet.set_cell("B1".parse()?, "Item 7")?;
    /// sheet.set_cell("C1".parse()?, "Mon")?;
    /// // A target must hold the source and extend it one way.
    /// workbook.fill("Sheet1", "A1:C1".parse()?, "A2:C3".parse()?, FillMode::Series).unwrap_err();
    /// workbook.fill("Sheet1", "A1:A1".parse()?, "A1:B2".parse()?, FillMode::Series).unwrap_err();
    /// workbook.fill("Sheet1", "A1:A2".parse()?, "A1:A4".parse()?, FillMode::Series)?;
    /// workbook.fill("Sheet1", "B1:C1".parse()?, "B1:C3".parse()?, FillMode::Series)?;
    /// let sheet = workbook.sheet("Sheet1")?;
    /// assert_eq!(sheet.scalar("A4".parse()?), Scalar::from(7.0));
    /// assert_eq!(sheet.scalar("B3".parse()?), Scalar::from("Item 9"));
    /// assert_eq!(sheet.scalar("C3".parse()?), Scalar::from("Wed"));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a target that does not extend
    /// the source in one direction, or a value its cell cannot hold; what
    /// [`Self::sheet`] returns otherwise. A refused fill changes nothing.
    pub fn fill(
        &mut self,
        sheet: &str,
        source: CellRange,
        target: CellRange,
        mode: FillMode,
    ) -> Result<()> {
        let Some(way) = way(sheet, source, target)? else {
            return Ok(());
        };
        let system = self.date_system();
        let held = self.sheet(sheet)?;
        let vertical = matches!(way, Way::Down | Way::Up);
        let lines: Vec<u32> = if vertical {
            (source.start().column()..=source.end().column()).collect()
        } else {
            (source.start().row()..=source.end().row()).collect()
        };
        let (first, last) = if vertical {
            (source.start().row(), source.end().row())
        } else {
            (source.start().column(), source.end().column())
        };
        let span = last - first + 1;
        let beyond: Vec<u32> = match way {
            Way::Down => (last + 1..=target.end().row()).collect(),
            Way::Right => (last + 1..=target.end().column()).collect(),
            Way::Up => (target.start().row()..first).rev().collect(),
            Way::Left => (target.start().column()..first).rev().collect(),
        };
        // Every cell past the source is written: bounded before any is.
        let filled = lines.len() as u64 * beyond.len() as u64;
        if filled > MAX_EDITED_CELLS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{sheet}!{target}"),
                reason: format_smolstr!(
                    "expected a fill of at most {MAX_EDITED_CELLS} cells past its source, got \
                     {filled}"
                ),
            });
        }
        let forward = matches!(way, Way::Down | Way::Right);
        let mut writes: Vec<(CellRef, Option<Cell>, Option<u64>)> = Vec::new();
        for line in lines {
            let at = |index: u32| {
                if vertical {
                    CellRef::new(index, line)
                } else {
                    CellRef::new(line, index)
                }
            };
            let cells: Vec<Option<Cell>> = (first..=last)
                .map(|index| held.cell(at(index)).cloned())
                .collect();
            let series = match mode {
                FillMode::Series => Series::of(&cells, system, |index| {
                    held.retained_serial(at(first + index as u32))
                }),
                FillMode::Copy => None,
            };
            for (step, index) in beyond.iter().enumerate() {
                let destination = at(*index);
                // The position in the source's own order: past its end going
                // forward, before its start going back.
                let position = if forward {
                    i64::from(span) + step as i64
                } else {
                    -1 - step as i64
                };
                let pattern = position.rem_euclid(i64::from(span)) as usize;
                let template = cells[pattern].as_ref();
                let (cell, raw) = match (&series, template) {
                    (Some(series), Some(template)) => {
                        let (value, raw) = series
                            .at(position, system)
                            .map_err(|error| located(sheet, destination, error))?;
                        let cell = Cell::from_scalar(destination, value, system)
                            .map_err(|error| located(sheet, destination, error))?;
                        (Some(cell.with_style(template.style())), raw)
                    }
                    (_, Some(template)) => {
                        let mut cell = template.clone().at(destination);
                        if let Some(formula) = cell
                            .formula()
                            .and_then(|formula| materialized(formula, destination))
                        {
                            cell.set_formula(Some(formula));
                        }
                        let source = at(first + pattern as u32);
                        (Some(cell), held.retained_serial(source))
                    }
                    (_, None) => (None, None),
                };
                let bits = cell.as_ref().and_then(|cell| {
                    raw.and_then(|raw| {
                        super::sheet::CellExtra::exceptional_serial(cell, raw, system)
                    })
                });
                writes.push((destination, cell, bits));
            }
        }
        let sheet = self.sheet_mut(sheet)?;
        for (at, cell, bits) in writes {
            match cell {
                Some(cell) => {
                    sheet.insert_cell(cell)?;
                    if let Some(bits) = bits {
                        sheet.attach_serial_bits(at, bits);
                    }
                }
                None => {
                    sheet.remove_cell(at);
                }
            }
        }
        Ok(())
    }
}

/// A refusal naming the sheet and the cell.
fn located(sheet: &str, at: CellRef, error: Error) -> Error {
    match error {
        Error::InvalidRecord { reason, .. } => Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{at}"),
            reason,
        },
        other => other,
    }
}

/// The built-in lists a series of names follows.
const LISTS: [&[&str]; 4] = [
    &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    &[
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    &[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
];

/// How a name is written: all lower case, all upper, or capitalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Lower,
    Upper,
    Title,
}

impl Case {
    fn of(text: &str) -> Self {
        if text.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
            Self::Lower
        } else if text.chars().filter(|c| c.is_alphabetic()).count() > 1
            && text.chars().all(|c| !c.is_alphabetic() || c.is_uppercase())
        {
            Self::Upper
        } else {
            Self::Title
        }
    }

    fn apply(self, text: &str) -> String {
        match self {
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
            Self::Title => text.to_owned(),
        }
    }
}

/// What a line of a fill's source continues as.
#[derive(Clone, Debug, PartialEq)]
enum Series {
    /// `value(position) = start + step * position`.
    Linear { start: f64, step: f64 },
    /// Days from the Unix epoch, stepping by months or years.
    Dates { first: i64, months: i64 },
    /// A time of day or a duration: `start + step * position`, as serials.
    Serials {
        start: f64,
        step: f64,
        format: NumberFormat,
    },
    /// Text ending in a number: its prefix, the number, its step, the
    /// digits it is padded to.
    Counted {
        prefix: SmolStr,
        start: i64,
        step: i64,
        width: usize,
    },
    /// A day or month name: which list, the first index, the step, the case.
    Listed {
        list: usize,
        start: i64,
        step: i64,
        case: Case,
    },
    /// A quarter: its spelling before the number, the first quarter, the
    /// step.
    Quarter {
        prefix: SmolStr,
        start: i64,
        step: i64,
    },
}

/// What one source cell of a line is to a series.
enum Item {
    Number(f64),
    Date(i64, f64),
    Serial(f64, NumberFormat),
    Counted(SmolStr, i64, usize),
    Listed(usize, i64, Case),
    Quarter(SmolStr, i64),
}

impl Item {
    fn of(cell: &Cell, system: DateSystem, raw: Option<f64>) -> Option<Self> {
        if cell.formula().is_some() || cell.error().is_some() {
            return None;
        }
        let value = cell.value();
        match cell.format() {
            NumberFormat::Date => {
                let serial = raw.or_else(|| {
                    system
                        .serial_of(value)
                        .ok()
                        .flatten()
                        .map(|(serial, _)| serial)
                });
                if let Scalar::Date32(days) = value {
                    return Some(Self::Date(i64::from(days.count()), serial?));
                }
                if value.temporal_unit().is_some() {
                    return Some(Self::Serial(serial?, NumberFormat::Date));
                }
            }
            format @ (NumberFormat::Time | NumberFormat::Duration) => {
                let (serial, _) = system.serial_of(value).ok()??;
                return Some(Self::Serial(serial, format));
            }
            _ => {}
        }
        if cell.kind() == CellKind::Number {
            return super::cell::number_of(value).map(Self::Number);
        }
        let text = value.as_str()?;
        if let Some(found) = listed(text) {
            return Some(found);
        }
        if let Some(found) = quarter(text) {
            return Some(found);
        }
        let digits = text.len() - text.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || digits > 15 {
            return None;
        }
        let (prefix, number) = text.split_at(text.len() - digits);
        let width = if number.starts_with('0') && number.len() > 1 {
            number.len()
        } else {
            0
        };
        Some(Self::Counted(
            SmolStr::new(prefix),
            number.parse().ok()?,
            width,
        ))
    }
}

/// The day or month name `text` is, by list, index and case.
fn listed(text: &str) -> Option<Item> {
    LISTS.iter().enumerate().find_map(|(list, names)| {
        names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(text))
            .map(|index| Item::Listed(list, index as i64, Case::of(text)))
    })
}

/// The quarter `text` spells: `Q1`, `Qtr1`, `Quarter 1` to 4, any case.
fn quarter(text: &str) -> Option<Item> {
    let lower = text.to_ascii_lowercase();
    ["quarter ", "qtr", "q"].iter().find_map(|stem| {
        let number = lower.strip_prefix(stem)?;
        let quarter: i64 = number
            .parse()
            .ok()
            .filter(|quarter| (1..=4).contains(quarter))?;
        Some(Item::Quarter(SmolStr::new(&text[..stem.len()]), quarter))
    })
}

impl Series {
    /// The series a line of source cells spells, `None` when it spells
    /// none - a blank, a formula, cells of two kinds, one number alone -
    /// and the line repeats as copies.
    fn of(
        cells: &[Option<Cell>],
        system: DateSystem,
        mut retained: impl FnMut(usize) -> Option<f64>,
    ) -> Option<Self> {
        let items: Vec<Item> = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                cell.as_ref()
                    .and_then(|cell| Item::of(cell, system, retained(index)))
            })
            .collect::<Option<Vec<_>>>()?;
        let first = items.first()?;
        match first {
            Item::Number(_) => {
                let values: Vec<f64> = items
                    .iter()
                    .map(|item| match item {
                        Item::Number(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                // One number copies, as a plain drag does.
                if values.len() < 2 {
                    return None;
                }
                Some(linear(&values))
            }
            Item::Date(start, first_serial) => {
                let dates: Vec<(i64, f64)> = items
                    .iter()
                    .map(|item| match item {
                        Item::Date(day, serial) => Some((*day, *serial)),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                if dates.len() == 1 {
                    return Some(Self::Serials {
                        start: *first_serial,
                        step: 1.0,
                        format: NumberFormat::Date,
                    });
                }
                let (y0, m0, d0) = civil_from_days(dates[0].0);
                let (y1, m1, d1) = civil_from_days(dates[1].0);
                if d0 == d1 && (y0, m0) != (y1, m1) {
                    let series = Self::Dates {
                        first: *start,
                        months: i64::from(y1 - y0) * 12 + i64::from(m1) - i64::from(m0),
                    };
                    return dates
                        .iter()
                        .enumerate()
                        .all(|(position, (day, _))| series.day(position as i64) == Some(*day))
                        .then_some(series);
                }
                let step = dates[1].1 - dates[0].1;
                dates
                    .iter()
                    .enumerate()
                    .all(|(position, (_, serial))| {
                        round15(first_serial + step * position as f64) == *serial
                    })
                    .then_some(Self::Serials {
                        start: *first_serial,
                        step,
                        format: NumberFormat::Date,
                    })
            }
            Item::Serial(_, format) => {
                let format = *format;
                let values: Vec<f64> = items
                    .iter()
                    .map(|item| match item {
                        Item::Serial(value, held) if *held == format => Some(*value),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let step = if values.len() == 1 {
                    if format == NumberFormat::Time {
                        1.0 / 24.0
                    } else {
                        1.0
                    }
                } else {
                    values[1] - values[0]
                };
                Some(Self::Serials {
                    start: values[0],
                    step,
                    format,
                })
            }
            Item::Counted(prefix, start, width) => {
                let numbers: Vec<i64> = items
                    .iter()
                    .map(|item| match item {
                        Item::Counted(held, number, _) if held == prefix => Some(*number),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let step = numbers.get(1).map_or(1, |second| second - start);
                arithmetic(&numbers, *start, step).then(|| Self::Counted {
                    prefix: prefix.clone(),
                    start: *start,
                    step,
                    width: *width,
                })
            }
            Item::Listed(list, start, case) => {
                let indices: Vec<i64> = items
                    .iter()
                    .map(|item| match item {
                        Item::Listed(held, index, _) if held == list => Some(*index),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let length = LISTS[*list].len() as i64;
                let step = indices
                    .get(1)
                    .map_or(1, |second| (second - start).rem_euclid(length));
                indices
                    .iter()
                    .enumerate()
                    .all(|(position, index)| {
                        (start + step * position as i64).rem_euclid(length) == *index
                    })
                    .then_some(Self::Listed {
                        list: *list,
                        start: *start,
                        step,
                        case: *case,
                    })
            }
            Item::Quarter(prefix, start) => {
                let quarters: Vec<i64> = items
                    .iter()
                    .map(|item| match item {
                        Item::Quarter(held, quarter) if held.eq_ignore_ascii_case(prefix) => {
                            Some(*quarter)
                        }
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let step = quarters
                    .get(1)
                    .map_or(1, |second| (second - start).rem_euclid(4));
                quarters
                    .iter()
                    .enumerate()
                    .all(|(position, quarter)| {
                        (start - 1 + step * position as i64).rem_euclid(4) + 1 == *quarter
                    })
                    .then(|| Self::Quarter {
                        prefix: prefix.clone(),
                        start: *start,
                        step,
                    })
            }
        }
    }

    /// The day a date series holds at `position`.
    fn day(&self, position: i64) -> Option<i64> {
        let Self::Dates { first, months } = *self else {
            return None;
        };
        let (year, month, day) = civil_from_days(first);
        let total = i64::from(year) * 12 + i64::from(month) - 1 + months.checked_mul(position)?;
        let year = i32::try_from(total.div_euclid(12)).ok()?;
        let month = (total.rem_euclid(12) + 1) as u32;
        // A day past the month's end is its last day, as Excel steps months.
        let last = (28..=31)
            .rev()
            .find(|candidate| civil_from_days(days_from_civil(year, month, *candidate)).1 == month)
            .unwrap_or(28);
        Some(days_from_civil(year, month, day.min(last)))
    }

    /// The value the series holds at `position` of the line, `0` its first
    /// source cell.
    fn at(&self, position: i64, system: DateSystem) -> Result<(Scalar, Option<f64>)> {
        let refused = || Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static(
                "expected a series value the grid can hold, got one past it",
            ),
        };
        let mut raw = None;
        let value = match self {
            Self::Linear { start, step } => Scalar::from(round15(start + step * position as f64)),
            Self::Dates { .. } => {
                let day = self.day(position).ok_or_else(refused)?;
                Scalar::date32(i32::try_from(day).map_err(|_| refused())?)
            }
            Self::Serials {
                start,
                step,
                format,
            } => {
                let serial = round15(start + step * position as f64);
                let serial = match format {
                    NumberFormat::Date => serial.floor(),
                    NumberFormat::Time => serial.rem_euclid(1.0),
                    _ => serial,
                };
                raw = Some(serial);
                system.scalar_from_serial(serial, *format)?
            }
            Self::Counted {
                prefix,
                start,
                step,
                width,
            } => {
                let number = step
                    .checked_mul(position)
                    .and_then(|moved| start.checked_add(moved))
                    .ok_or_else(refused)?;
                // The number is the text's digits, which spell no sign:
                // counting down past zero counts up again, as Excel does.
                let number = number.unsigned_abs();
                Scalar::from(format!("{prefix}{number:0width$}"))
            }
            Self::Listed {
                list,
                start,
                step,
                case,
            } => {
                let names = LISTS[*list];
                let index = (start + step * position).rem_euclid(names.len() as i64) as usize;
                Scalar::from(case.apply(names[index]))
            }
            Self::Quarter {
                prefix,
                start,
                step,
            } => {
                let quarter = (start - 1 + step * position).rem_euclid(4) + 1;
                Scalar::from(format!("{prefix}{quarter}"))
            }
        };
        Ok((value, raw))
    }
}

/// Whether `numbers` step from `start` by `step`.
fn arithmetic(numbers: &[i64], start: i64, step: i64) -> bool {
    numbers.iter().enumerate().all(|(position, number)| {
        step.checked_mul(position as i64)
            .and_then(|moved| start.checked_add(moved))
            == Some(*number)
    })
}

/// The linear series `values` spell: their step where they step alike
/// within fifteen digits, else the least-squares line through them.
fn linear(values: &[f64]) -> Series {
    let step = values[1] - values[0];
    let alike = values
        .iter()
        .enumerate()
        .all(|(position, value)| round15(values[0] + step * position as f64) == round15(*value));
    if alike {
        return Series::Linear {
            start: values[0],
            step,
        };
    }
    let count = values.len() as f64;
    let mean_x = (count - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / count;
    let (mut covariance, mut variance) = (0.0, 0.0);
    for (position, value) in values.iter().enumerate() {
        let dx = position as f64 - mean_x;
        covariance += dx * (value - mean_y);
        variance += dx * dx;
    }
    let slope = covariance / variance;
    Series::Linear {
        start: mean_y - slope * mean_x,
        step: slope,
    }
}

/// `value` rounded to fifteen significant digits, as Excel keeps a number.
fn round15(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    // The display/entry decimal owner already rounds into stack storage.
    let magnitude = Digits::from_f64(value).as_f64().unwrap_or(value.abs());
    magnitude.copysign(value)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! The decimal boundary behind AutoFill, pinned without a public API.
    /// Evaluate the series decimal boundary without constructing a workbook.
    pub fn round15_for_test(value: f64) -> f64 {
        super::round15(value)
    }
}
