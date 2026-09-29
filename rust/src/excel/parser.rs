//! The one pass over a worksheet part: `<sheetData>` folded into rows of
//! cells, each cell's facts as the file states them.
//!
//! Every reader of a sheet - the random-access [`Sheet`](super::Sheet), the
//! record stream, the schema tally, the row count - is this parser with a
//! different consumer, so the grammar of a `<row>` and a `<c>` is read in one
//! place: the optional `r` on both (a running cursor supplies the row after
//! the previous one and the column after the previous cell), the `t`, the
//! `s`, the `<v>`, the `<is>` with its `t` and `r/t` runs, the `<f>`. A row
//! or cell whose `r` goes backwards is refused, because Excel requires the
//! order and repairs a file without it.

use std::io::BufRead;

use quick_xml::Reader;
use quick_xml::events::Event;
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::{CellKind, CellRef, MAX_COLUMNS, MAX_ROWS};
use super::package::{attribute, codec_error, local_name, reference_text, text_piece};

/// One `<c>` as the file states it, its text still the file's: a shared
/// string's index, a serial's digits, a boolean's `0` or `1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawCell {
    pub(crate) column: u32,
    pub(crate) kind: CellKind,
    /// The `s` attribute, a `cellXfs` index; zero when absent.
    pub(crate) style: u32,
    /// The `<v>` text, or the `<is>` text with its runs joined and its
    /// `_xHHHH_` escapes decoded; inline up to `SmolStr`'s capacity, which
    /// every serial and most text fits, so a cell costs no allocation.
    pub(crate) content: SmolStr,
    /// Whether the cell stated any content at all: `<c/>` states none.
    pub(crate) has_content: bool,
    /// The `<f>` text, when the cell carries a formula.
    pub(crate) formula: Option<SmolStr>,
}

/// One `<row>`: its zero-based index and its cells in column order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawRow {
    pub(crate) index: u32,
    pub(crate) cells: Vec<RawCell>,
}

/// Where the parser stands inside the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Outside `<sheetData>`.
    Outside,
    /// Between rows.
    Rows,
    /// Inside a `<row>`, between cells.
    Row,
    /// Inside a `<c>`, between its children.
    Cell,
    /// Inside `<v>`.
    Value,
    /// Inside `<f>`.
    Formula,
    /// Inside `<is>`, between its runs.
    Inline,
    /// Inside a `<t>` of `<is>` whose text is the cell's.
    InlineText,
}

/// What one event did to the cursor.
enum Step {
    /// Nothing a consumer sees.
    Continue,
    /// A row closed.
    Row(RawRow),
    /// The part ended.
    End,
}

/// The rows of one worksheet part, one at a time.
///
/// Rows are yielded in document order, which the parser holds to be
/// ascending; a row the part declares with no cell is yielded with none.
pub(crate) struct SheetRows<R: BufRead> {
    reader: Reader<R>,
    buffer: Vec<u8>,
    /// The sheet's name, which every refusal names as Excel does: `Trades!B3`.
    sheet: SmolStr,
    place: Place,
    /// Elements deep inside a subtree nothing is read from, such as `rPh`.
    skipping: usize,
    /// The row after the last one yielded, which a `<row>` with no `r` takes.
    next_row: u32,
    /// The cursor's row, once a `<row>` has opened.
    row: u32,
    /// The column after the last cell, which a `<c>` with no `r` takes.
    next_column: u32,
    cells: Vec<RawCell>,
    current: RawCell,
    /// The character data of the element the cursor is in, gathered across
    /// its text, CDATA and reference events; one buffer for the whole part.
    content: String,
    /// Whether the part ended.
    done: bool,
    /// Whether `<sheetData>` was seen, so a part without one is refused.
    saw_data: bool,
}

impl<R: BufRead> SheetRows<R> {
    /// Parse the worksheet part `source` of the sheet `sheet`, which the
    /// refusals name.
    pub(crate) fn new(source: R, sheet: impl Into<SmolStr>) -> Self {
        Self {
            reader: super::styles::reader(source),
            buffer: Vec::with_capacity(256),
            sheet: sheet.into(),
            place: Place::Outside,
            skipping: 0,
            next_row: 0,
            row: 0,
            next_column: 0,
            cells: Vec::new(),
            current: RawCell::default(),
            content: String::new(),
            done: false,
            saw_data: false,
        }
    }

    /// The byte position the reader stands at.
    fn byte_position(&self) -> usize {
        usize::try_from(self.reader.buffer_position()).unwrap_or(usize::MAX)
    }

    /// Refuse the part at the cursor, naming it and the cell.
    fn refuse(&self, reason: SmolStr) -> Error {
        self.refuse_at(
            CellRef::new(self.row, self.next_column.saturating_sub(1)),
            reason,
        )
    }

    /// Refuse the part at the cursor, naming the cell `reference`.
    fn refuse_at(&self, reference: CellRef, reason: SmolStr) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{reference}", self.sheet),
            reason: format_smolstr!("at byte {}: {reason}", self.byte_position()),
        }
    }

    /// Read the next row, `None` at the end of the part.
    fn read_row(&mut self) -> Result<Option<RawRow>> {
        if self.done {
            return Ok(None);
        }
        loop {
            // The event borrows the buffer it was read into, so the buffer
            // leaves `self` for the duration of one event and comes back.
            let mut buffer = std::mem::take(&mut self.buffer);
            buffer.clear();
            let position = self.byte_position();
            let step = match self.reader.read_event_into(&mut buffer) {
                Ok(event) => self.step(&event, position),
                Err(error) => Err(codec_error(position, error.to_string())),
            };
            self.buffer = buffer;
            match step? {
                Step::Continue => {}
                Step::Row(row) => return Ok(Some(row)),
                Step::End => {
                    self.done = true;
                    if !self.saw_data {
                        return Err(Error::InvalidRecord {
                            path: self.sheet.clone(),
                            reason: SmolStr::new_static(
                                "expected a worksheet part holding sheetData, got a part without one",
                            ),
                        });
                    }
                    return Ok(None);
                }
            }
        }
    }

    /// Fold one event into the cursor's state.
    fn step(&mut self, event: &Event<'_>, position: usize) -> Result<Step> {
        match event {
            Event::Start(start) => {
                if self.skipping > 0 {
                    self.skipping += 1;
                    return Ok(Step::Continue);
                }
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                Ok(self
                    .open(start, name, position, false)?
                    .map_or(Step::Continue, Step::Row))
            }
            Event::Empty(start) => {
                if self.skipping > 0 {
                    return Ok(Step::Continue);
                }
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                Ok(self
                    .open(start, name, position, true)?
                    .map_or(Step::Continue, Step::Row))
            }
            Event::End(end) => {
                if self.skipping > 0 {
                    self.skipping -= 1;
                    return Ok(Step::Continue);
                }
                let qualified = end.name();
                Ok(self
                    .close(local_name(qualified.as_ref()))
                    .map_or(Step::Continue, Step::Row))
            }
            Event::Text(held) => {
                if self.reads_text() {
                    let piece = text_piece(held.xml10_content(), position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::CData(held) => {
                if self.reads_text() {
                    let piece = text_piece(held.xml10_content(), position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::GeneralRef(_) => {
                if self.reads_text() {
                    let piece = reference_text(event, position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::Eof => Ok(Step::End),
            _ => Ok(Step::Continue),
        }
    }

    /// Whether character data at the cursor is a cell's content.
    const fn reads_text(&self) -> bool {
        self.skipping == 0
            && matches!(
                self.place,
                Place::Value | Place::Formula | Place::InlineText
            )
    }

    /// Open an element; a self-closed `<row/>` or `<c/>` is opened and closed.
    fn open(
        &mut self,
        start: &quick_xml::events::BytesStart<'_>,
        name: &[u8],
        position: usize,
        empty: bool,
    ) -> Result<Option<RawRow>> {
        match (self.place, name) {
            (Place::Outside, b"sheetData") => {
                self.saw_data = true;
                if empty {
                    return Ok(None);
                }
                self.place = Place::Rows;
            }
            (Place::Rows, b"row") => {
                let row = match attribute(start, b"r", position)? {
                    Some(text) => {
                        let stated: u32 = text
                            .trim()
                            .parse()
                            .ok()
                            .filter(|row| (1..=MAX_ROWS).contains(row))
                            .ok_or_else(|| {
                                self.refuse(format_smolstr!(
                                    "expected a row number from 1 to {MAX_ROWS}, got {text:?}"
                                ))
                            })?
                            - 1;
                        if stated < self.next_row {
                            return Err(self.refuse(format_smolstr!(
                                "expected rows in ascending order, got row {} after row {}",
                                stated + 1,
                                self.next_row
                            )));
                        }
                        stated
                    }
                    None => self.next_row,
                };
                if row >= MAX_ROWS {
                    return Err(self.refuse(format_smolstr!(
                        "expected at most {MAX_ROWS} rows, got row {}",
                        row + 1
                    )));
                }
                self.row = row;
                self.next_row = row + 1;
                self.next_column = 0;
                self.cells.clear();
                if empty {
                    return Ok(Some(RawRow {
                        index: row,
                        cells: Vec::new(),
                    }));
                }
                self.place = Place::Row;
            }
            (Place::Row, b"c") => {
                let column = match attribute(start, b"r", position)? {
                    Some(text) => {
                        let reference: CellRef = text.trim().parse().map_err(|error: Error| {
                            self.refuse(super::cell::wire_reason(&error))
                        })?;
                        if reference.row() != self.row {
                            return Err(self.refuse_at(
                                reference,
                                format_smolstr!(
                                    "expected a cell of row {}, got {reference}",
                                    self.row + 1
                                ),
                            ));
                        }
                        if reference.column() < self.next_column {
                            return Err(self.refuse_at(
                                reference,
                                format_smolstr!(
                                    "expected cells in ascending column order, got {reference} after column {}",
                                    CellRef::column_name(self.next_column - 1)
                                ),
                            ));
                        }
                        reference.column()
                    }
                    None => self.next_column,
                };
                if column >= MAX_COLUMNS {
                    return Err(self.refuse(format_smolstr!(
                        "expected at most {MAX_COLUMNS} columns, got column {}",
                        column + 1
                    )));
                }
                // The cursor is on this cell from here on, so a refusal of
                // its attributes names it.
                self.next_column = column + 1;
                let kind = match attribute(start, b"t", position)? {
                    Some(text) => CellKind::from_attribute(text.trim())
                        .map_err(|error| self.refuse(super::cell::wire_reason(&error)))?,
                    None => CellKind::Number,
                };
                let style = attribute(start, b"s", position)?
                    .and_then(|text| text.trim().parse().ok())
                    .unwrap_or(0);
                self.current = RawCell {
                    column,
                    kind,
                    style,
                    content: SmolStr::default(),
                    has_content: false,
                    formula: None,
                };
                self.content.clear();
                if empty {
                    self.cells.push(std::mem::take(&mut self.current));
                    return Ok(None);
                }
                self.place = Place::Cell;
            }
            (Place::Cell, b"v") => {
                self.current.has_content = true;
                if !empty {
                    self.place = Place::Value;
                }
            }
            (Place::Cell, b"f") => {
                self.current.formula = Some(SmolStr::default());
                if !empty {
                    self.place = Place::Formula;
                }
            }
            (Place::Cell, b"is") => {
                self.current.has_content = true;
                if !empty {
                    self.place = Place::Inline;
                }
            }
            (Place::Inline, b"t") => {
                if !empty {
                    self.place = Place::InlineText;
                }
            }
            (Place::Inline, b"rPh" | b"phoneticPr") => {
                if !empty {
                    self.skipping = 1;
                }
            }
            // A rich run holds its `t`, read at the `Inline` place.
            (Place::Inline, b"r") => {}
            // `extLst`, `rPr` and anything else stated inside the data is
            // not a fact this reader keeps; a self-closed one has no subtree.
            (Place::Cell | Place::Row | Place::Rows | Place::Inline, _) if !empty => {
                self.skipping = 1;
            }
            _ => {}
        }
        Ok(None)
    }

    /// Close an element, yielding the row a `</row>` completes.
    fn close(&mut self, name: &[u8]) -> Option<RawRow> {
        match (self.place, name) {
            (Place::Value, b"v") => self.place = Place::Cell,
            (Place::Formula, b"f") => {
                if let Some(formula) = &mut self.current.formula {
                    *formula = SmolStr::new(&self.content);
                }
                self.content.clear();
                self.place = Place::Cell;
            }
            (Place::InlineText, b"t") => self.place = Place::Inline,
            (Place::Inline, b"is") => self.place = Place::Cell,
            (Place::Cell, b"c") => {
                let mut cell = std::mem::take(&mut self.current);
                // Text written into the part itself - inline, or a formula's
                // cached string - carries the `_xHHHH_` escapes; a shared
                // string's content is its index.
                cell.content =
                    if matches!(cell.kind, CellKind::InlineString | CellKind::FormulaString) {
                        SmolStr::new(super::shared_strings::decode(&self.content))
                    } else {
                        SmolStr::new(&self.content)
                    };
                self.content.clear();
                self.cells.push(cell);
                self.place = Place::Row;
            }
            (Place::Row, b"row") => {
                self.place = Place::Rows;
                return Some(RawRow {
                    index: self.row,
                    cells: std::mem::take(&mut self.cells),
                });
            }
            (Place::Rows, b"sheetData") => {
                self.place = Place::Outside;
            }
            _ => {}
        }
        None
    }
}

impl<R: BufRead> Iterator for SheetRows<R> {
    type Item = Result<RawRow>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_row() {
            Ok(row) => row.map(Ok),
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}
