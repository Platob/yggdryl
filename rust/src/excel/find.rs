//! Find and replace: the cells whose text matches a pattern, visited as
//! Excel's Find Next visits them, and replaced as Replace All replaces
//! them - one set of options, one matcher, so Find Next stops exactly on
//! the cells Replace All changes.
//!
//! A pattern is text with Excel's wildcards, read by the matcher the
//! criteria of `COUNTIF` and its kin read too (`formula/criteria.rs`): `*`
//! any run of characters, `?` one character, `~` taking the character
//! after it as it stands (`~*`, `~?`, `~~`), each cell read once per
//! match. It matches anywhere in a cell's text, or the whole
//! of it with [`FindOptions::entire_cell`], without case unless
//! [`FindOptions::match_case`]. A cell's text is what it shows
//! ([`Within::Values`], [`Workbook::display_text`]) or what would be typed
//! to make it ([`Within::Formulas`], [`Workbook::entry_text`]); a blank
//! cell never matches, and a merged range matches only at its top-left
//! cell.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::{CellRange, CellRef, MAX_CELL_TEXT};
use super::formula::criteria::Wildcard;
use super::sheet::{Sheet, SheetState};
use super::workbook::{SheetKind, Workbook};

/// Where a find looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FindScope {
    /// The one worksheet it starts on.
    #[default]
    Sheet,
    /// Every visible worksheet, from the one it starts on in tab order and
    /// round to it.
    Workbook,
}

impl FindScope {
    /// The scope as the service spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sheet => "sheet",
            Self::Workbook => "workbook",
        }
    }
}

/// Which text of a cell a find reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Within {
    /// The text the cell shows.
    #[default]
    Values,
    /// The text that typed makes the cell: a formula as it is typed.
    Formulas,
}

impl Within {
    /// The choice as the service spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Values => "values",
            Self::Formulas => "formulas",
        }
    }
}

/// What a find looks for, and where.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FindOptions {
    /// The pattern: text with `*`, `?` and `~` read as Excel reads them.
    pub text: SmolStr,
    /// The worksheet alone, or every visible one.
    pub scope: FindScope,
    /// The worksheet the find starts on.
    pub sheet: SmolStr,
    /// The text of each cell it reads.
    pub within: Within,
    /// Whether case tells two texts apart.
    pub match_case: bool,
    /// Whether the pattern must match the whole of a cell's text.
    pub entire_cell: bool,
}

/// The pattern `options` states, read by the one wildcard matcher.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] for an empty pattern.
fn pattern(options: &FindOptions) -> Result<Wildcard> {
    if options.text.is_empty() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.text"),
            reason: SmolStr::new_static("expected text to find, got the empty text"),
        });
    }
    Ok(Wildcard::new(
        &options.text,
        options.match_case,
        options.entire_cell,
    ))
}

impl Workbook {
    /// The next cell matching `options`, as Find Next finds it: from the
    /// cell after `from` on the worksheet `options.sheet` - by rows, left to
    /// right then down - through the following visible worksheets when the
    /// scope is the workbook, and round to `from` itself, so a lone match
    /// answers again; from `A1` of the worksheet, `A1` included, without
    /// `from`. `None` when no cell matches.
    ///
    /// ```
    /// use yggdryl::excel::{FindOptions, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Data")?;
    /// sheet.set_cell("B2".parse()?, "AAPL")?;
    /// sheet.set_cell("C9".parse()?, "aapl.us")?;
    /// let options = FindOptions { text: "aapl*".into(), sheet: "Data".into(), ..FindOptions::default() };
    /// let found = |from: &str| workbook.find(&options, Some(from.parse().unwrap())).unwrap().map(|(_, at)| at.to_string());
    /// assert_eq!(found("B2").as_deref(), Some("C9"));
    /// assert_eq!(found("C9").as_deref(), Some("B2"));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an empty pattern, [`Error::Absent`]
    /// for a sheet the workbook lacks, and what [`Self::sheet`] returns.
    pub fn find(
        &self,
        options: &FindOptions,
        from: Option<CellRef>,
    ) -> Result<Option<(SmolStr, CellRef)>> {
        let pattern = pattern(options)?;
        let order = self.find_order(options)?;
        let Some((&start, rest)) = order.split_first() else {
            return Ok(None);
        };
        let starting = self.sheet_at(start)?.expect("a sheet the order names");
        let after = |at: CellRef| from.is_none_or(|from| at > from);
        let upto = |at: CellRef| from.is_some_and(|from| at <= from);
        let name = |at: usize| SmolStr::new(self.sheet_names()[at]);
        if let Some(at) = self.first_match(starting, &pattern, options.within, after)? {
            return Ok(Some((name(start), at)));
        }
        for &index in rest {
            let sheet = self.sheet_at(index)?.expect("a sheet the order names");
            if let Some(at) = self.first_match(sheet, &pattern, options.within, |_| true)? {
                return Ok(Some((name(index), at)));
            }
        }
        Ok(self
            .first_match(starting, &pattern, options.within, upto)?
            .map(|at| (name(start), at)))
    }

    /// Replace every match of `options` with `replacement`, as Replace All
    /// does, in every cell [`Self::find`] would stop on, answering how many
    /// cells changed. Each changed cell is typed again as
    /// [`Self::set_entry`] types text: a formula searched in its entry text
    /// is read again as a formula.
    ///
    /// ```
    /// use yggdryl::excel::{FindOptions, Within, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Data")?;
    /// sheet.set_cell("A1".parse()?, "north east")?;
    /// sheet.set_cell("A2".parse()?, "North")?;
    /// let options = FindOptions { text: "north".into(), sheet: "Data".into(), within: Within::Formulas, ..FindOptions::default() };
    /// assert_eq!(workbook.replace(&options, "South")?, 2);
    /// assert_eq!(workbook.sheet("Data")?.scalar("A1".parse()?), "South east".into());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns, before any cell changes: what [`Self::find`] returns,
    /// [`Error::InvalidRecord`] naming the cell for a formula the
    /// replacement leaves unreadable, a text it makes longer than
    /// [`MAX_CELL_TEXT`] characters, or a formula cell whose shown value a
    /// replace within values would overwrite; and the refusal
    /// [`Self::set_entry`] answers for a cell, the cells before it put back.
    pub fn replace(&mut self, options: &FindOptions, replacement: &str) -> Result<u64> {
        let planned = self.plan_replace(options, replacement)?;
        let mut steps = Vec::new();
        for (name, cells) in by_sheet(&planned) {
            steps.push(self.cells_step(&name, &cells)?);
        }
        self.guarded(steps, |workbook| workbook.enter_replaced(&planned))?;
        Ok(planned.len() as u64)
    }

    /// The cells a replace of `options` with `replacement` changes, each
    /// with its sheet and the text it is typed as - every refusal a text
    /// alone decides made before any cell changes.
    pub(crate) fn plan_replace(
        &self,
        options: &FindOptions,
        replacement: &str,
    ) -> Result<Vec<(SmolStr, CellRef, String)>> {
        let pattern = pattern(options)?;
        let mut planned: Vec<(SmolStr, CellRef, String)> = Vec::new();
        for (index, at) in self.find_all(options)? {
            let name = SmolStr::new(self.sheet_names()[index]);
            let text = self
                .cell_text(&name, at, options.within)?
                .unwrap_or_default();
            let Some(replaced) = pattern.replace(&text, replacement) else {
                continue;
            };
            let sheet = self.sheet(&name)?;
            let cell = sheet.cell(at).expect("a cell the find matched");
            let refused = |reason: SmolStr| Error::InvalidRecord {
                path: format_smolstr!("{name}!{at}"),
                reason,
            };
            if options.within == Within::Values && cell.formula().is_some() {
                return Err(refused(SmolStr::new_static(
                    "expected a constant to replace within values, got a formula's result; \
                     replace within formulas",
                )));
            }
            if let Some(formula) = replaced.strip_prefix('=') {
                super::formula::Formula::from_entry(formula, at).map_err(|error| match error {
                    Error::Parse {
                        position, reason, ..
                    } => refused(format_smolstr!(
                        "expected the replaced formula to read, got {reason} at byte {}",
                        position + 1
                    )),
                    other => other,
                })?;
            } else {
                let length = replaced.chars().count();
                if length > MAX_CELL_TEXT {
                    return Err(refused(format_smolstr!(
                        "expected at most {MAX_CELL_TEXT} characters in a cell, got {length}"
                    )));
                }
            }
            planned.push((name, at, replaced));
        }
        Ok(planned)
    }

    /// Type each text `planned` holds into its cell, stopping at the first
    /// refusal: [`Self::replace`] without its snapshot.
    pub(crate) fn enter_replaced(&mut self, planned: &[(SmolStr, CellRef, String)]) -> Result<()> {
        for (name, at, text) in planned {
            self.set_entry(name, *at, text)?;
        }
        Ok(())
    }

    /// Every cell a find of `options` stops on, sheet by sheet in the
    /// order it looks, by sheet position and reference.
    pub(crate) fn find_all(&self, options: &FindOptions) -> Result<Vec<(usize, CellRef)>> {
        let pattern = pattern(options)?;
        let mut found = Vec::new();
        for index in self.find_order(options)? {
            let sheet = self.sheet_at(index)?.expect("a sheet the order names");
            let name = SmolStr::new(sheet.name());
            for cell in sheet.cells() {
                let at = cell.reference();
                if !anchors(sheet, at) {
                    continue;
                }
                if self
                    .cell_text(&name, at, options.within)?
                    .is_some_and(|text| !text.is_empty() && pattern.is_match(&text))
                {
                    found.push((index, at));
                }
            }
        }
        Ok(found)
    }

    /// The worksheets a find of `options` looks through, by position: the
    /// one it starts on, then - for the workbook - each visible worksheet
    /// after it in tab order, round to it.
    fn find_order(&self, options: &FindOptions) -> Result<Vec<usize>> {
        let start = self
            .position(&options.sheet)
            .ok_or_else(|| Error::absent("worksheet", options.sheet.as_str()))?;
        self.sheet_at(start)?;
        let mut order = vec![start];
        if options.scope == FindScope::Workbook {
            let count = self.len();
            for offset in 1..count {
                let index = (start + offset) % count;
                let name = self.sheet_names()[index];
                if self.sheet_kind(name) != Some(SheetKind::Worksheet) {
                    continue;
                }
                if self
                    .sheet_at(index)?
                    .is_some_and(|sheet| sheet.state() == SheetState::Visible)
                {
                    order.push(index);
                }
            }
        }
        Ok(order)
    }

    /// The first cell of `sheet` `take` keeps whose text matches, by rows.
    fn first_match(
        &self,
        sheet: &Sheet,
        pattern: &Wildcard,
        within: Within,
        take: impl Fn(CellRef) -> bool,
    ) -> Result<Option<CellRef>> {
        let name = SmolStr::new(sheet.name());
        for cell in sheet.cells() {
            let at = cell.reference();
            if !take(at) || !anchors(sheet, at) {
                continue;
            }
            if self
                .cell_text(&name, at, within)?
                .is_some_and(|text| !text.is_empty() && pattern.is_match(&text))
            {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }

    /// The text a find reads of the cell at `at`: what it shows, or what
    /// typing makes it.
    fn cell_text(&self, sheet: &str, at: CellRef, within: Within) -> Result<Option<String>> {
        match within {
            Within::Values => Ok(self
                .display_text(sheet, at)?
                .map(|shown| shown.text.to_string())),
            Within::Formulas => self.entry_text(sheet, at),
        }
    }
}

/// The cells `planned` changes, by sheet in the order they come.
pub(crate) fn by_sheet(planned: &[(SmolStr, CellRef, String)]) -> Vec<(SmolStr, Vec<CellRange>)> {
    let mut sheets: Vec<(SmolStr, Vec<CellRange>)> = Vec::new();
    for (name, at, _) in planned {
        let range = CellRange::new(*at, *at);
        match sheets.iter_mut().find(|(held, _)| held == name) {
            Some((_, ranges)) => ranges.push(range),
            None => sheets.push((name.clone(), vec![range])),
        }
    }
    sheets
}

/// Whether the cell at `at` of `sheet` shows: every cell but those a merge
/// covers past its top-left one.
fn anchors(sheet: &Sheet, at: CellRef) -> bool {
    !sheet
        .merges()
        .any(|merge: CellRange| merge.contains(at) && merge.start() != at)
}
