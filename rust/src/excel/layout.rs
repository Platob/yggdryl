//! What a worksheet states about its grid beside its cells: row heights and
//! formats, column widths and formats, merged ranges, the frozen pane, and
//! the defaults the rest take.
//!
//! These are the worksheet facts the model owns - read from the part once,
//! answered by [`Sheet`](super::Sheet), and written back from the model -
//! as against the elements a sheet carries verbatim (`carried`).
//! A row stating a format and holding no cell is a row of its own here, so
//! a height or a hidden flag survives on a row nothing is written in.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::cell::{CellRange, CellRef, MAX_COLUMNS};
use super::shift::{Axis, Band};
use super::style::StyleId;

/// The column width a sheet states no default for, in the file's
/// character units padding included: Excel's 64-pixel column at a maximum
/// digit width of 7.
pub const DEFAULT_COLUMN_WIDTH: f64 = 9.140_625;

/// The row height a sheet states no default for, in points: Calibri 11.
pub const DEFAULT_ROW_HEIGHT: f64 = 15.0;

/// A frozen pane: how many rows at the top and columns at the left stay in
/// view while the rest scrolls.
///
/// ```
/// use yggdryl::excel::{Frozen, Sheet};
///
/// let sheet = Sheet::new("Trades")?;
/// assert_eq!(sheet.frozen(), None);
/// let frozen = Frozen { rows: 1, columns: 2 };
/// assert_eq!(frozen.top_left().to_string(), "C2");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frozen {
    /// The rows frozen at the top.
    pub rows: u32,
    /// The columns frozen at the left.
    pub columns: u32,
}

impl Frozen {
    /// The first cell that scrolls: the pane's `topLeftCell`.
    #[must_use]
    pub const fn top_left(self) -> CellRef {
        CellRef::new(self.rows, self.columns)
    }

    /// The pane that holds the scrolling cells, as `activePane` names it.
    pub(crate) const fn active_pane(self) -> &'static str {
        match (self.rows > 0, self.columns > 0) {
            (true, true) => "bottomRight",
            (true, false) => "bottomLeft",
            _ => "topRight",
        }
    }
}

/// The formats of a span of rows: the span, and each row in it that
/// states one, by index - what an undo puts back over the span.
pub(crate) type RowFormats = (Range<u32>, Vec<(u32, RowFormat)>);

/// What one `<row>` states beyond its cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RowFormat {
    /// `ht`, in points.
    pub(crate) height: Option<f64>,
    pub(crate) hidden: bool,
    /// `s`: the style a cell the row holds none of shows.
    pub(crate) style: Option<StyleId>,
    /// `customFormat`: whether `s` applies.
    pub(crate) custom_format: bool,
    /// `customHeight`: whether `ht` was set rather than fitted.
    pub(crate) custom_height: bool,
    pub(crate) outline_level: u8,
    pub(crate) collapsed: bool,
    pub(crate) thick_top: bool,
    pub(crate) thick_bottom: bool,
    pub(crate) phonetic: bool,
    /// Every other attribute, as the file wrote it, a space before each:
    /// ` x14ac:dyDescent="0.25"`.
    pub(crate) extra: Option<Arc<str>>,
}

impl RowFormat {
    /// Whether the row states nothing.
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Read the attributes of a `<row>` other than `r` and `spans`.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the attribute whose value the schema does
    /// not spell.
    pub(crate) fn read(
        start: &quick_xml::events::BytesStart<'_>,
        extra: &mut String,
        last_extra: &mut Option<Arc<str>>,
    ) -> Result<Self> {
        let mut format = Self::default();
        extra.clear();
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| refusal("row", error.to_string()))?;
            let key = held.key.as_ref();
            let value = || -> Result<String> {
                Ok(held
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|error| refusal("row", error.to_string()))?
                    .into_owned())
            };
            match key {
                b"r" | b"spans" => {}
                b"s" | b"customFormat" => {
                    format.read_style_attribute(key, &value()?)?;
                }
                b"ht" => format.height = Some(number("row", "ht", &value()?)?),
                b"hidden" => format.hidden = flag("row", "hidden", &value()?)?,
                b"customHeight" => format.custom_height = flag("row", "customHeight", &value()?)?,
                b"outlineLevel" => format.outline_level = level("row", &value()?)?,
                b"collapsed" => format.collapsed = flag("row", "collapsed", &value()?)?,
                b"thickTop" => format.thick_top = flag("row", "thickTop", &value()?)?,
                b"thickBot" => format.thick_bottom = flag("row", "thickBot", &value()?)?,
                b"ph" => format.phonetic = flag("row", "ph", &value()?)?,
                _ => {
                    extra.push(' ');
                    extra.push_str(&String::from_utf8_lossy(key));
                    extra.push_str("=\"");
                    // The value as written, its quotes spelled for the
                    // double quotes it is written back inside.
                    extra.push_str(&String::from_utf8_lossy(&held.value).replace('"', "&quot;"));
                    extra.push('"');
                }
            }
        }
        if !extra.is_empty() {
            // Rows of one part state the same extra attributes over and
            // over - Excel's `x14ac:dyDescent` on every row - so a row
            // stating what the one before stated shares its text.
            let shared = match last_extra.as_ref() {
                Some(last) if **last == **extra => Arc::clone(last),
                _ => {
                    let held: Arc<str> = Arc::from(extra.as_str());
                    *last_extra = Some(Arc::clone(&held));
                    held
                }
            };
            format.extra = Some(shared);
        }
        Ok(format)
    }

    /// Read the two attributes that control the default style of absent cells.
    /// Package overlays lend normalized values; RowFormat::read shares this
    /// intake for a held sheet, so customFormat has one interpretation.
    pub(crate) fn read_style_attribute(&mut self, key: &[u8], value: &str) -> Result<()> {
        match key {
            b"s" => self.style = Some(StyleId::from_attribute(value)?),
            b"customFormat" => self.custom_format = flag("row", "customFormat", value)?,
            _ => {}
        }
        Ok(())
    }

    /// The style an absent cell inherits from this row.
    pub(crate) fn applied_style(&self) -> Option<StyleId> {
        self.style.filter(|_| self.custom_format)
    }

    /// Write the attributes, a space before each.
    pub(crate) fn write(&self, target: &mut String) {
        if let Some(style) = self.style {
            let _ = write!(target, " s=\"{}\"", style.as_u16());
        }
        if self.custom_format {
            target.push_str(" customFormat=\"1\"");
        }
        if let Some(height) = self.height {
            let _ = write!(target, " ht=\"{height}\"");
        }
        if self.hidden {
            target.push_str(" hidden=\"1\"");
        }
        if self.custom_height {
            target.push_str(" customHeight=\"1\"");
        }
        if self.outline_level > 0 {
            let _ = write!(target, " outlineLevel=\"{}\"", self.outline_level);
        }
        if self.collapsed {
            target.push_str(" collapsed=\"1\"");
        }
        if self.thick_top {
            target.push_str(" thickTop=\"1\"");
        }
        if self.thick_bottom {
            target.push_str(" thickBot=\"1\"");
        }
        if self.phonetic {
            target.push_str(" ph=\"1\"");
        }
        if let Some(extra) = &self.extra {
            target.push_str(extra);
        }
    }
}

/// What one `<col>` states about the columns it spans.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ColumnFormat {
    /// `width`, in the file's character units, padding included.
    pub(crate) width: Option<f64>,
    /// `style`: the style an empty cell of the column shows.
    pub(crate) style: Option<StyleId>,
    pub(crate) hidden: bool,
    pub(crate) best_fit: bool,
    /// `customWidth`: whether `width` was set rather than the default.
    pub(crate) custom_width: bool,
    pub(crate) phonetic: bool,
    pub(crate) outline_level: u8,
    pub(crate) collapsed: bool,
}

/// The formatted columns: spans sorted by their first column, none
/// overlapping another, each with what it states.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Columns(pub(crate) Vec<(Range<u32>, ColumnFormat)>);

impl Columns {
    /// What the column `column` states, when a span covers it.
    pub(crate) fn get(&self, column: u32) -> Option<&ColumnFormat> {
        let at = self.0.partition_point(|(span, _)| span.end <= column);
        self.0
            .get(at)
            .filter(|(span, _)| span.contains(&column))
            .map(|(_, format)| format)
    }

    /// Add the span of the `<col>` at `start`, which must follow every span
    /// read so far.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a span outside the grid, backwards, or
    /// overlapping the span before it.
    pub(crate) fn read(&mut self, start: &quick_xml::events::BytesStart<'_>) -> Result<()> {
        let next = self.0.last().map_or(0, |(span, _)| span.end);
        let span = Self::read_span(
            start.attributes().with_checks(false).map(|held| {
                let held = held.map_err(|error| refusal("col", error.to_string()))?;
                let value = held
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|error| refusal("col", error.to_string()))?;
                Ok((held.key.0, value))
            }),
            next,
        )?;
        self.0.push(span);
        Ok(())
    }

    /// Resolve one normalized column span after the preceding span's end.
    /// A streaming overlay can keep only the selected column defaults rather
    /// than collecting every column format in the worksheet.
    pub(crate) fn read_span<'a>(
        attributes: impl Iterator<Item = Result<(&'a [u8], std::borrow::Cow<'a, str>)>>,
        next: u32,
    ) -> Result<(Range<u32>, ColumnFormat)> {
        let mut format = ColumnFormat::default();
        let (mut min, mut max) = (None, None);
        for held in attributes {
            let (key, value) = held?;
            match key {
                b"min" => min = Some(index("min", &value)?),
                b"max" => max = Some(index("max", &value)?),
                b"width" => format.width = Some(number("col", "width", &value)?),
                b"style" => format.style = Some(StyleId::from_attribute(&value)?),
                b"hidden" => format.hidden = flag("col", "hidden", &value)?,
                b"bestFit" => format.best_fit = flag("col", "bestFit", &value)?,
                b"customWidth" => format.custom_width = flag("col", "customWidth", &value)?,
                b"phonetic" => format.phonetic = flag("col", "phonetic", &value)?,
                b"outlineLevel" => format.outline_level = level("col", &value)?,
                b"collapsed" => format.collapsed = flag("col", "collapsed", &value)?,
                _ => {}
            }
        }
        let (Some(min), Some(max)) = (min, max) else {
            return Err(refusal(
                "col",
                "expected a <col> stating its first and last column, got one without",
            ));
        };
        if min == 0 || max < min || max > MAX_COLUMNS {
            return Err(refusal(
                "col",
                format_smolstr!(
                    "expected columns from 1 to {MAX_COLUMNS}, first before last, got {min} to {max}"
                ),
            ));
        }
        let span = min - 1..max;
        if next > span.start {
            return Err(refusal(
                "col",
                format_smolstr!(
                    "expected column spans in ascending order, none overlapping, got {min} to {max}"
                ),
            ));
        }
        Ok((span, format))
    }

    /// Write `<cols>` under the element prefix `prefix`, nothing for no span.
    pub(crate) fn write(&self, target: &mut String, prefix: &str) {
        if self.0.is_empty() {
            return;
        }
        let _ = write!(target, "<{prefix}cols>");
        for (span, format) in &self.0 {
            let _ = write!(
                target,
                "<{prefix}col min=\"{}\" max=\"{}\"",
                span.start + 1,
                span.end
            );
            if let Some(width) = format.width {
                let _ = write!(target, " width=\"{width}\"");
            }
            if let Some(style) = format.style {
                let _ = write!(target, " style=\"{}\"", style.as_u16());
            }
            for (stated, name) in [
                (format.hidden, "hidden"),
                (format.best_fit, "bestFit"),
                (format.custom_width, "customWidth"),
                (format.phonetic, "phonetic"),
            ] {
                if stated {
                    let _ = write!(target, " {name}=\"1\"");
                }
            }
            if format.outline_level > 0 {
                let _ = write!(target, " outlineLevel=\"{}\"", format.outline_level);
            }
            if format.collapsed {
                target.push_str(" collapsed=\"1\"");
            }
            target.push_str("/>");
        }
        let _ = write!(target, "</{prefix}cols>");
    }
}

/// The worksheet facts the model owns beside the cells.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layout {
    pub(crate) columns: Columns,
    /// Rows stating a format, by zero-based row.
    pub(crate) rows: BTreeMap<u32, RowFormat>,
    /// Merged ranges, in the order the part listed them.
    pub(crate) merges: Vec<CellRange>,
    pub(crate) pane: Option<Frozen>,
    /// `sheetFormatPr@defaultColWidth`.
    pub(crate) default_column_width: Option<f64>,
    /// `sheetFormatPr@defaultRowHeight`.
    pub(crate) default_row_height: Option<f64>,
}

impl Layout {
    /// Read the `ref` of a `<mergeCell>`.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a merge naming no range of the grid.
    pub(crate) fn read_merge(&mut self, start: &quick_xml::events::BytesStart<'_>) -> Result<()> {
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| refusal("mergeCell", error.to_string()))?;
            if held.key.as_ref() != b"ref" {
                continue;
            }
            let value = held
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| refusal("mergeCell", error.to_string()))?;
            let range: CellRange = value.trim().parse().map_err(|_| {
                refusal(
                    "mergeCell",
                    format_smolstr!("expected a range such as A1:B2, got {value:?}"),
                )
            })?;
            self.merges.push(range);
            return Ok(());
        }
        Err(refusal(
            "mergeCell",
            "expected a <mergeCell> stating its ref, got one without",
        ))
    }

    /// Read what `sheetFormatPr` states the defaults are.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a default that is no number.
    pub(crate) fn read_defaults(
        &mut self,
        start: &quick_xml::events::BytesStart<'_>,
    ) -> Result<()> {
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| refusal("sheetFormatPr", error.to_string()))?;
            let value = held
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| refusal("sheetFormatPr", error.to_string()))?;
            match held.key.as_ref() {
                b"defaultColWidth" => {
                    self.default_column_width =
                        Some(number("sheetFormatPr", "defaultColWidth", &value)?);
                }
                b"defaultRowHeight" => {
                    self.default_row_height =
                        Some(number("sheetFormatPr", "defaultRowHeight", &value)?);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Write `<mergeCells>` under the element prefix `prefix`, nothing for no
    /// merge.
    pub(crate) fn write_merges(&self, target: &mut String, prefix: &str) {
        if self.merges.is_empty() {
            return;
        }
        let _ = write!(
            target,
            "<{prefix}mergeCells count=\"{}\">",
            self.merges.len()
        );
        for range in &self.merges {
            let _ = write!(
                target,
                "<{prefix}mergeCell ref=\"{}\"/>",
                merge_text(*range)
            );
        }
        let _ = write!(target, "</{prefix}mergeCells>");
    }

    /// Move what stands on the rows or columns of `axis` as `band` opens or
    /// closes them, as Excel moves it: a format moves with its row or
    /// column and goes with a removed one, a span of columns an insertion
    /// falls inside grows; a merge moves, grows where an insertion falls
    /// inside it and shrinks where a removal cuts it - one left a single
    /// cell, or taken whole, is none; the frozen split moves with the rows
    /// or columns frozen above or left of it. What would leave the grid
    /// goes.
    pub(crate) fn shift(&mut self, axis: Axis, band: Band) {
        let limit = axis.limit();
        match axis {
            Axis::Rows => {
                let inherited = match band {
                    Band::Insert { at, .. } => at
                        .checked_sub(1)
                        .and_then(|previous| self.rows.get(&previous))
                        .map(|previous| RowFormat {
                            height: previous.height,
                            custom_height: previous.custom_height,
                            outline_level: previous.outline_level,
                            ..RowFormat::default()
                        })
                        .filter(|format| !format.is_empty()),
                    Band::Remove { .. } => None,
                };
                let moved = std::mem::take(&mut self.rows);
                self.rows = moved
                    .into_iter()
                    .filter_map(|(row, format)| Some((band.index(row, limit)?, format)))
                    .collect();
                if let (Band::Insert { at, count }, Some(format)) = (band, inherited) {
                    for row in at..at.saturating_add(count).min(limit) {
                        self.rows.insert(row, format.clone());
                    }
                }
            }
            Axis::Columns => {
                let inherited = match band {
                    Band::Insert { at, .. } => at
                        .checked_sub(1)
                        .and_then(|previous| self.columns.get(previous))
                        .cloned(),
                    Band::Remove { .. } => None,
                };
                let spans = std::mem::take(&mut self.columns.0);
                for (span, format) in spans {
                    if let Some((first, last)) = band.span(span.start, span.end - 1, limit) {
                        self.columns.0.push((first..last + 1, format));
                    }
                }
                if let (Band::Insert { at, count }, Some(previous)) = (band, inherited) {
                    // Excel carries the preceding dimension and group level,
                    // while inserted rows/columns start visible and expanded.
                    self.update_columns(at..at.saturating_add(count).min(limit), |format| {
                        format.width = previous.width;
                        format.custom_width = previous.custom_width;
                        format.outline_level = previous.outline_level;
                        format.hidden = false;
                        format.collapsed = false;
                    });
                }
            }
        }
        self.merges.retain_mut(|merge| {
            let (first, last) = axis.span(*merge);
            let Some((first, last)) = band.span(first, last, limit) else {
                return false;
            };
            *merge = CellRange::new(
                axis.with(merge.start(), first),
                axis.with(merge.end(), last),
            );
            is_merge(*merge)
        });
        if let Some(pane) = self.pane.as_mut() {
            let frozen = frozen_along(*pane, axis);
            let moved = match band {
                Band::Insert { at, count } if at < frozen => {
                    frozen.saturating_add(count).min(limit - 1)
                }
                Band::Insert { .. } => frozen,
                Band::Remove { start, end } => frozen - frozen.min(end).saturating_sub(start),
            };
            match axis {
                Axis::Rows => pane.rows = moved,
                Axis::Columns => pane.columns = moved,
            }
        }
        self.pane = self.pane.filter(|pane| pane.rows > 0 || pane.columns > 0);
    }

    /// Put `update`'s answer in place of what each column of `columns`
    /// states - a span split where the columns start or end inside it, a
    /// column stating nothing taking a span of its own - dropping a span
    /// left stating nothing and joining neighbours stating the same.
    /// Answers whether anything changed.
    pub(crate) fn update_columns(
        &mut self,
        columns: Range<u32>,
        update: impl Fn(&mut ColumnFormat),
    ) -> bool {
        if columns.is_empty() {
            return false;
        }
        let held = std::mem::take(&mut self.columns.0);
        let mut cuts: Vec<u32> = held
            .iter()
            .flat_map(|(span, _)| [span.start, span.end])
            .chain([columns.start, columns.end])
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut spans: Vec<(Range<u32>, ColumnFormat)> = Vec::new();
        for pair in cuts.windows(2) {
            let piece = pair[0]..pair[1];
            let stated = held
                .iter()
                .find(|(span, _)| span.start <= piece.start && piece.end <= span.end)
                .map(|(_, format)| format.clone());
            let inside = columns.start <= piece.start && piece.end <= columns.end;
            let format = match (stated, inside) {
                (stated, true) => {
                    let mut format = stated.unwrap_or_default();
                    update(&mut format);
                    format
                }
                (Some(format), false) => format,
                (None, false) => continue,
            };
            if format == ColumnFormat::default() {
                continue;
            }
            match spans.last_mut() {
                Some((last, previous)) if last.end == piece.start && *previous == format => {
                    last.end = piece.end;
                }
                _ => spans.push((piece, format)),
            }
        }
        let changed = spans != held;
        self.columns.0 = spans;
        changed
    }

    /// Update formats without erasing an existing physical row. Clearing
    /// absent formatting creates no row; answers whether anything changed.
    pub(crate) fn update_rows(
        &mut self,
        rows: Range<u32>,
        update: impl Fn(&mut RowFormat),
    ) -> bool {
        let mut changed = false;
        for row in rows {
            match self.rows.entry(row) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let format = entry.get_mut();
                    let before = format.clone();
                    update(format);
                    changed |= *format != before;
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let mut format = RowFormat::default();
                    update(&mut format);
                    if !format.is_empty() {
                        entry.insert(format);
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

/// How many rows or columns `pane` freezes along `axis`.
const fn frozen_along(pane: Frozen, axis: Axis) -> u32 {
    match axis {
        Axis::Rows => pane.rows,
        Axis::Columns => pane.columns,
    }
}

/// Whether `range` spans more than one cell, which a merge must.
fn is_merge(range: CellRange) -> bool {
    range.start() != range.end()
}

/// A merged range as `mergeCell@ref` spells it: always two corners.
fn merge_text(range: CellRange) -> SmolStr {
    let mut text = String::with_capacity(16);
    range.start().write_a1(&mut text);
    text.push(':');
    range.end().write_a1(&mut text);
    SmolStr::new(text)
}

/// A refusal of the `element` of a worksheet part.
fn refusal(element: &str, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{element}"),
        reason: reason.into(),
    }
}

/// A boolean attribute `name` of `element`: `1`, `true`, `0` or `false`.
fn flag(element: &str, name: &str, value: &str) -> Result<bool> {
    match value.trim() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(refusal(
            element,
            format_smolstr!("expected 1, true, 0 or false for {name}, got {other:?}"),
        )),
    }
}

/// A number attribute `name` of `element`, at least zero.
fn number(element: &str, name: &str, value: &str) -> Result<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(|| {
            refusal(
                element,
                format_smolstr!("expected a number of at least 0 for {name}, got {value:?}"),
            )
        })
}

/// A one-based column index attribute `name` of a `<col>`.
fn index(name: &str, value: &str) -> Result<u32> {
    value.trim().parse::<u32>().map_err(|_| {
        refusal(
            "col",
            format_smolstr!("expected a column number for {name}, got {value:?}"),
        )
    })
}

/// The outline level `element` states: 0 to 7.
fn level(element: &str, value: &str) -> Result<u8> {
    value
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|level| *level <= 7)
        .ok_or_else(|| {
            refusal(
                element,
                format_smolstr!("expected an outline level from 0 to 7, got {value:?}"),
            )
        })
}
