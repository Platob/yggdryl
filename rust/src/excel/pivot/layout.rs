//! Checked tabular PivotTable geometry. Cell content is assembled separately.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::{PivotSpec, BLANK_CAPTION};
use crate::excel::{CellRange, CellRef, MAX_COLUMNS, MAX_EDITED_CELLS, MAX_ROWS};

/// The output rectangle and the zero-based location attributes written to OOXML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Geometry {
    pub range: CellRange,
    pub header_rows: u32,
    pub first_header_row: u32,
    pub first_data_row: u32,
    pub first_data_col: u32,
    pub width: u32,
    pub height: u32,
}

fn refused(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.pivot.location"),
        reason: reason.into(),
    }
}

/// Bound one layout after compute has counted rendered leaf, subtotal and grand
/// rows and displayed column groups, including parent subtotals. Both counts
/// are facts from the grouped item plan, never dense grids.
pub fn geometry(
    spec: &PivotSpec,
    anchor: CellRef,
    rendered_rows: u32,
    rendered_column_groups: u32,
) -> Result<Geometry> {
    anchor.require_in_grid()?;
    let rows = u32::try_from(spec.rows.len()).map_err(|_| refused("too many row fields"))?;
    let columns =
        u32::try_from(spec.columns.len()).map_err(|_| refused("too many column fields"))?;
    let values = u32::try_from(spec.values.len()).map_err(|_| refused("too many value fields"))?;
    if rows == 0
        || values == 0
        || rendered_rows == 0
        || (columns > 0 && rendered_column_groups == 0)
    {
        return Err(refused(
            "expected row fields, value fields, rendered rows and column items",
        ));
    }
    // Native Excel has one header row, including when only the implicit
    // Σ Values field occupies the column axis. It adds the values header
    // level only when an actual source column field precedes that axis.
    let header_rows = if columns == 0 {
        1
    } else {
        1u32.checked_add(columns)
            .and_then(|n| n.checked_add(u32::from(values > 1)))
            .ok_or_else(|| refused("header rows overflow"))?
    };
    let data_columns = if columns == 0 {
        values
    } else {
        rendered_column_groups.checked_mul(values)
            .ok_or_else(|| refused("data columns overflow"))?
    };
    let width = rows
        .checked_add(data_columns)
        .ok_or_else(|| refused("pivot width overflow"))?;
    let height = header_rows
        .checked_add(rendered_rows)
        .ok_or_else(|| refused("pivot height overflow"))?;
    let end_row = anchor
        .row()
        .checked_add(height - 1)
        .ok_or_else(|| refused("row overflow"))?;
    let end_column = anchor
        .column()
        .checked_add(width - 1)
        .ok_or_else(|| refused("column overflow"))?;
    let area = u64::from(width) * u64::from(height);
    if end_row >= MAX_ROWS || end_column >= MAX_COLUMNS || area > MAX_EDITED_CELLS {
        return Err(refused(format_smolstr!(
            "expected a pivot inside the Excel grid with at most {MAX_EDITED_CELLS} output cells, got {width} columns by {height} rows at {anchor}"
        )));
    }
    Ok(Geometry {
        range: CellRange::new(anchor, CellRef::new(end_row, end_column)),
        header_rows,
        first_header_row: u32::from(columns > 0 || values == 1),
        first_data_row: header_rows,
        first_data_col: rows,
        width,
        height,
    })
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Native layout controls for the single tabular geometry owner.

    pub use super::Geometry;
    pub use super::geometry;
}

// Append inside pivot/layout.rs after the checked geometry owner.

use super::ItemOrder;
use super::compute::{PivotComputed, PivotItem};
use std::cmp::Ordering;

fn display_refused(field: &str, reason: &'static str) -> Error {
    Error::InvalidRecord { path: field.into(), reason: reason.into() }
}

/// Cache IDs stay in source insertion order. These ordinals and event streams
/// are the sole visible order used by cells and rowItems/colItems XML.
pub struct PivotDisplay {
    pub row_fields: Vec<Vec<usize>>,
    pub column_fields: Vec<Vec<usize>>,
    pub row_rank: Vec<Vec<usize>>,
    pub column_rank: Vec<Vec<usize>>,
    pub rows: Vec<usize>,
    pub columns: Vec<usize>,
    pub row_events: Vec<PivotEvent>,
    pub column_events: Vec<PivotEvent>,
}

/// One displayed axis event. A subtotal spans leaf ordinals, never source
/// rows or a dense grid; level is the parent field whose run just closed.
#[derive(Clone, Copy, Debug)]
pub enum PivotEvent {
    Leaf { id: usize, first_new: usize },
    Subtotal { level: usize, from: usize, to: usize },
    Grand,
}

fn item_kind(item: &PivotItem) -> u8 {
    match item {
        PivotItem::Number(_) => 0,
        PivotItem::Blank => 1,
        PivotItem::Date(_) => 2,
        PivotItem::Text(_) => 3,
        PivotItem::Boolean(_) => 4,
        PivotItem::Error(_) => 5,
    }
}

fn visible_items(
    items: &[PivotItem],
    order: ItemOrder,
    field: &str,
) -> Result<(Vec<usize>, Vec<usize>)> {
    let mut kinds = 0_u8;
    for item in items {
        match item {
            PivotItem::Blank => continue,
            PivotItem::Text(text) if !text.is_ascii() => {
                return Err(display_refused(field, "non-ASCII collation has no selected locale"));
            }
            PivotItem::Error(_) if item.authored_error_label().is_none() => {
                return Err(display_refused(field, "an authored classic error heading"));
            }
            _ => {}
        }
        kinds |= 1 << item_kind(item);
        // Native order covers Number+Date, Number+Text+Boolean and
        // Text+Error. Other mixed domains remain held.
        let numeric_text_bool = (1 << 0) | (1 << 3) | (1 << 4);
        let numeric_date = (1 << 0) | (1 << 2);
        let text_error = (1 << 3) | (1 << 5);
        if kinds.count_ones() > 1
            && kinds & !numeric_text_bool != 0
            && kinds & !numeric_date != 0
            && kinds & !text_error != 0
        {
            return Err(display_refused(field, "mixed item collation is unproved"));
        }
    }
    let mut visible: Vec<usize> = (0..items.len()).collect();
    visible.sort_by(|&left, &right| {
        let a = &items[left];
        let b = &items[right];
        let ord = match (a, b) {
            (PivotItem::Blank, PivotItem::Blank) => Ordering::Equal,
            // An authored item@n participates in Excel's requested order.
            // Its text compares with ordinary labels at the same axis level.
            (PivotItem::Blank, PivotItem::Text(text)) => BLANK_CAPTION
                .bytes()
                .cmp(text.bytes().map(|ch| ch.to_ascii_lowercase())),
            (PivotItem::Text(text), PivotItem::Blank) => text
                .bytes()
                .map(|ch| ch.to_ascii_lowercase())
                .cmp(BLANK_CAPTION.bytes()),
            (PivotItem::Text(a), PivotItem::Text(b)) => a
                .bytes()
                .map(|ch| ch.to_ascii_lowercase())
                .cmp(b.bytes().map(|ch| ch.to_ascii_lowercase())),
            (PivotItem::Number(a), PivotItem::Number(b))
            | (PivotItem::Date(a), PivotItem::Date(b)) => a.total_cmp(b),
            (PivotItem::Boolean(a), PivotItem::Boolean(b)) => a.cmp(b),
            (PivotItem::Error(a), PivotItem::Error(b)) => a.cmp(b),
            _ => item_kind(a).cmp(&item_kind(b)), // mixed kinds were preflighted
        };
        match order {
            ItemOrder::Ascending => ord,
            ItemOrder::Descending => ord.reverse(),
        }
    });
    let mut rank = vec![0; visible.len()];
    for (position, &cache) in visible.iter().enumerate() {
        rank[cache] = position;
    }
    Ok((visible, rank))
}

/// Emit one event per leaf and one per closed parent run. A singleton axis
/// has no parent level, so the subtotals flag is a no-op for that axis.
fn events(sorted: &[usize], tuples: &[Vec<usize>], subtotals: bool, grand: bool) -> Vec<PivotEvent> {
    let depth = tuples.first().map_or(0, Vec::len);
    let parents = depth.saturating_sub(1);
    let mut starts = vec![0; parents];
    let mut result = Vec::with_capacity(sorted.len() + usize::from(grand));
    let mut prior: Option<usize> = None;
    for (position, &id) in sorted.iter().enumerate() {
        let first_new = prior.map_or(0, |old| {
            tuples[old].iter().zip(&tuples[id]).take_while(|(a, b)| a == b).count()
        });
        if prior.is_some() {
            for level in (first_new..parents).rev() {
                if subtotals {
                    result.push(PivotEvent::Subtotal { level, from: starts[level], to: position });
                }
            }
        }
        for start in starts.iter_mut().skip(first_new) {
            *start = position;
        }
        result.push(PivotEvent::Leaf { id, first_new });
        prior = Some(id);
    }
    if !sorted.is_empty() && subtotals {
        for level in (0..parents).rev() {
            result.push(PivotEvent::Subtotal { level, from: starts[level], to: sorted.len() });
        }
    }
    if grand {
        result.push(PivotEvent::Grand);
    }
    result
}

impl PivotDisplay {
    /// Resolve source cache IDs to visible lexicographic tuples once.
    pub fn new(spec: &PivotSpec, computed: &PivotComputed) -> Result<Self> {
        let mut row_fields = Vec::with_capacity(spec.rows.len());
        let mut row_rank = Vec::with_capacity(spec.rows.len());
        for (index, (axis, items)) in spec.rows.iter().zip(&computed.row_items).enumerate() {
            let (visible, rank) = visible_items(items.items(), axis.order, &format!("$.rows[{index}]"))?;
            row_fields.push(visible);
            row_rank.push(rank);
        }
        let mut column_fields = Vec::with_capacity(spec.columns.len());
        let mut column_rank = Vec::with_capacity(spec.columns.len());
        for (index, (axis, items)) in spec.columns.iter().zip(&computed.column_items).enumerate() {
            let (visible, rank) = visible_items(items.items(), axis.order, &format!("$.columns[{index}]"))?;
            column_fields.push(visible);
            column_rank.push(rank);
        }
        let mut rows: Vec<usize> = (0..computed.row_tuples.len()).collect();
        rows.sort_by(|&left, &right| {
            computed.row_tuples[left].iter().zip(&computed.row_tuples[right])
                .enumerate().map(|(field, (&a, &b))| row_rank[field][a].cmp(&row_rank[field][b]))
                .find(|ord| !ord.is_eq()).unwrap_or(Ordering::Equal)
        });
        let mut columns: Vec<usize> = (0..computed.column_tuples.len()).collect();
        columns.sort_by(|&left, &right| {
            computed.column_tuples[left].iter().zip(&computed.column_tuples[right])
                .enumerate().map(|(field, (&a, &b))| column_rank[field][a].cmp(&column_rank[field][b]))
                .find(|ord| !ord.is_eq()).unwrap_or(Ordering::Equal)
        });
        let row_events = events(&rows, &computed.row_tuples, spec.subtotals, spec.column_grand_totals);
        let column_events = events(
            &columns, &computed.column_tuples, spec.subtotals && !spec.columns.is_empty(),
            spec.row_grand_totals && !spec.columns.is_empty(),
        );
        Ok(Self {
            row_fields, column_fields, row_rank, column_rank, rows, columns,
            row_events, column_events,
        })
    }
}
