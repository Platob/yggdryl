//! Pretty, debug-oriented display for scalars.
//!
//! An **atomic** scalar renders as its value (`42`, `1.5`, `"hi"`, `0x0102`,
//! `null`); a **serie** renders as a one-column table headed by its field (name and
//! type) with the first [`max_rows`](DisplayOptions::max_rows) elements; a **struct**
//! serie / record renders as a multi-column table (one column per field), and a
//! **map** as a two-column `key | value` table. Any value that lands in a *cell*
//! (a struct field, a list element, a map key/value) is shown compactly inline —
//! `{x: 1, name: "a"}`, `[1, 2, …]`, `{7: 42, …}` — so the whole thing tries to fit
//! the [`max_width`](DisplayOptions::max_width). Nesting past [`MAX_DEPTH`] collapses
//! to a bare `{…}` / `[…]`, which both keeps a pathological value readable and is the
//! backstop that stops a container type with no dedicated arm (a bare Arrow union or
//! map) from recursing forever. The [`Display`](std::fmt::Display) impls use the
//! defaults ([`DisplayOptions::default`]); `display_with` takes an explicit
//! [`DisplayOptions`].

use crate::AnyScalar;
use arrow_array::Array;
use arrow_schema::DataType;

/// Rendering knobs for [`display_with`](crate::Scalar) — how many rows a table shows
/// and the width it tries to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayOptions {
    /// The maximum number of rows a serie / struct-serie table prints before it
    /// truncates with a `… (N more)` footer. Default `10`.
    pub max_rows: usize,
    /// The width, in characters, the table tries to fit — columns that overflow are
    /// dropped into a trailing `…` column and long cells are elided. Default `100`.
    pub max_width: usize,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            max_rows: 10,
            max_width: 100,
        }
    }
}

/// The widest a single cell prints before it is elided with `…`.
const MAX_CELL: usize = 40;

/// How deep a nested cell (a struct/list/map inside a struct/list/map) recurses before
/// it collapses to a bare `{…}` / `[…]`. It keeps a pathological value readable, and —
/// because every arm that recurses (including the `_` fallback's bounce back through
/// [`format_any`]) counts against it — it is the hard backstop that stops a container
/// type with no dedicated arm (a bare Arrow union or map) from recursing forever.
const MAX_DEPTH: usize = 5;

/// The most elements a nested `list` or `map` cell prints inline before it elides the
/// tail with `…`.
const MAX_INLINE: usize = 6;

/// Elide `text` to `max` characters (counting `char`s, good enough for debug), adding
/// a trailing `…` when cut.
fn elide(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1).max(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// The display width of `text` in characters.
fn width(text: &str) -> usize {
    text.chars().count()
}

/// Format one element of an Arrow array at `index` compactly — the fallback for the
/// values a type-erased [`AnyScalar`] holds as Arrow (utf8, binary, null and the
/// nested types), and for the cells of a struct column. `depth` bounds nesting (see
/// [`MAX_DEPTH`]).
fn format_arrow(array: &dyn Array, index: usize, depth: usize) -> String {
    use arrow_array::{
        BinaryArray, BooleanArray, LargeListArray, LargeStringArray, ListArray, MapArray,
        StringArray, StructArray,
    };
    if index >= array.len() || array.is_null(index) {
        return "null".to_string();
    }
    // Past the depth budget a nested container collapses to a bare marker — both to
    // keep a pathological value readable and to guarantee termination.
    if depth > MAX_DEPTH {
        return match array.data_type() {
            DataType::Struct(_) | DataType::Map(..) | DataType::Union(..) => "{…}".to_string(),
            DataType::List(_) | DataType::LargeList(_) => "[…]".to_string(),
            _ => "…".to_string(),
        };
    }
    match array.data_type() {
        DataType::Utf8 => array
            .as_any()
            .downcast_ref::<StringArray>()
            .map(|a| format!("{:?}", a.value(index)))
            .unwrap_or_else(|| "?".to_string()),
        DataType::LargeUtf8 => array
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .map(|a| format!("{:?}", a.value(index)))
            .unwrap_or_else(|| "?".to_string()),
        DataType::Binary => array
            .as_any()
            .downcast_ref::<BinaryArray>()
            .map(|a| hex(a.value(index)))
            .unwrap_or_else(|| "?".to_string()),
        DataType::Boolean => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .map(|a| a.value(index).to_string())
            .unwrap_or_else(|| "?".to_string()),
        DataType::Struct(_) => array
            .as_any()
            .downcast_ref::<StructArray>()
            .map(|entries| {
                let inner = entries
                    .columns()
                    .iter()
                    .zip(struct_field_names(entries))
                    .map(|(column, name)| {
                        format!("{name}: {}", format_arrow(column, index, depth + 1))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{inner}}}")
            })
            .unwrap_or_else(|| "{…}".to_string()),
        DataType::List(_) => array
            .as_any()
            .downcast_ref::<ListArray>()
            .map(|list| format_list_cell(&list.value(index), depth))
            .unwrap_or_else(|| "[…]".to_string()),
        DataType::LargeList(_) => array
            .as_any()
            .downcast_ref::<LargeListArray>()
            .map(|list| format_list_cell(&list.value(index), depth))
            .unwrap_or_else(|| "[…]".to_string()),
        // A map: its entries struct's key/value pair up as `{key: value, …}`.
        DataType::Map(..) => array
            .as_any()
            .downcast_ref::<MapArray>()
            .map(|map| {
                let entries = map.value(index);
                let keys = entries.column(0);
                let values = entries.column(1);
                let shown = entries.len().min(MAX_INLINE);
                let mut cells: Vec<String> = (0..shown)
                    .map(|i| {
                        format!(
                            "{}: {}",
                            format_arrow(keys, i, depth + 1),
                            format_arrow(values, i, depth + 1)
                        )
                    })
                    .collect();
                if entries.len() > shown {
                    cells.push("…".to_string());
                }
                format!("{{{}}}", cells.join(", "))
            })
            .unwrap_or_else(|| "{…}".to_string()),
        DataType::Null => "null".to_string(),
        // A union (an `optional`'s storage): unwrap into the active variant and format
        // that child — recursing on the *child*, never the union itself, so it always
        // terminates (formatting the union directly would loop forever).
        DataType::Union(..) => array
            .as_any()
            .downcast_ref::<arrow_array::UnionArray>()
            .map(|union| format_arrow(union.value(index).as_ref(), 0, depth + 1))
            .unwrap_or_else(|| "null".to_string()),
        // Numeric / other leaves: read the one element through a one-row AnyScalar. The
        // bounce carries `depth`, so a container type with no arm above still hits the
        // depth backstop instead of looping.
        _ => {
            let one = AnyScalar::from_arrow(array.slice(index, 1));
            format_any_at(&one, depth)
        }
    }
}

/// Format a list element (already sliced out of its parent) as `[a, b, …]`, showing
/// the first [`MAX_INLINE`] items — shared by the `List` and `LargeList` cell arms.
fn format_list_cell(element: &dyn Array, depth: usize) -> String {
    let shown = element.len().min(MAX_INLINE);
    let mut cells: Vec<String> = (0..shown)
        .map(|i| format_arrow(element, i, depth + 1))
        .collect();
    if element.len() > shown {
        cells.push("…".to_string());
    }
    format!("[{}]", cells.join(", "))
}

/// The field names of a struct array (its child field names), in column order.
fn struct_field_names(entries: &arrow_array::StructArray) -> Vec<String> {
    match entries.data_type() {
        DataType::Struct(fields) => fields.iter().map(|f| f.name().to_string()).collect(),
        _ => (0..entries.num_columns()).map(|i| i.to_string()).collect(),
    }
}

/// A byte slice as `0x`-prefixed hex, the first 16 bytes then `…`.
fn hex(bytes: &[u8]) -> String {
    let shown = bytes.len().min(16);
    let mut out = String::from("0x");
    for byte in &bytes[..shown] {
        out.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > shown {
        out.push('…');
    }
    out
}

/// Format a type-erased [`AnyScalar`] as a compact cell value — the shared value
/// formatter behind every table cell and the atomic scalars' own display.
pub(crate) fn format_any(any: &AnyScalar) -> String {
    format_any_at(any, 0)
}

/// [`format_any`] carrying the current nesting `depth`, so the Arrow bounce keeps
/// counting against [`MAX_DEPTH`] rather than resetting it (the recursion backstop).
fn format_any_at(any: &AnyScalar, depth: usize) -> String {
    if any.is_null() {
        return "null".to_string();
    }
    // The decomposed numeric fast paths read the concrete scalar directly.
    macro_rules! numeric {
        ($($accessor:ident),+ $(,)?) => {
            $(if let Some(scalar) = any.$accessor() {
                if let Some(value) = crate::Scalar::value(scalar) {
                    return value.to_string();
                }
            })+
        };
    }
    numeric!(int8, int16, int32, int64, uint8, uint16, uint32, uint64, float16, float32, float64);
    // Everything else (utf8, binary, null, nested) is held as Arrow.
    match any.arrow() {
        Some(array) => format_arrow(array.as_ref(), 0, depth + 1),
        None => "null".to_string(),
    }
}

/// A single table column: its two-line header (name, then type signature) and its
/// already-formatted cells.
pub(crate) struct Column {
    pub name: String,
    pub type_signature: String,
    pub cells: Vec<String>,
}

/// Render `columns` (all the same cell count) as a box-drawn table, honouring
/// [`DisplayOptions`]: at most `max_rows` body rows (with a `… (total more)` footer
/// past that) and a best-effort fit to `max_width` — trailing columns that overflow
/// collapse into a single `…` column, and every cell is elided to [`MAX_CELL`].
///
/// `total` is the full element count (so the footer can report how many rows were
/// hidden).
pub(crate) fn render_table(
    mut columns: Vec<Column>,
    total: usize,
    options: DisplayOptions,
) -> String {
    if columns.is_empty() {
        return "(empty)".to_string();
    }
    // Elide every cell and header up front.
    for column in &mut columns {
        column.name = elide(&column.name, MAX_CELL);
        column.type_signature = elide(&column.type_signature, MAX_CELL);
        for cell in &mut column.cells {
            *cell = elide(cell, MAX_CELL);
        }
    }

    // Drop trailing columns that would overflow `max_width`, standing in a `…` column.
    let mut budget = options.max_width;
    let mut kept = 0usize;
    for column in &columns {
        let cell_width = column
            .cells
            .iter()
            .map(|c| width(c))
            .chain([width(&column.name), width(&column.type_signature)])
            .max()
            .unwrap_or(0);
        let needed = cell_width + 3; // "│ " + trailing space
        if kept > 0 && budget < needed + 4 {
            break;
        }
        budget = budget.saturating_sub(needed);
        kept += 1;
    }
    let hidden_columns = columns.len() - kept;
    columns.truncate(kept);
    if hidden_columns > 0 {
        columns.push(Column {
            name: "…".to_string(),
            type_signature: format!("+{hidden_columns}"),
            cells: vec!["…".to_string(); columns.first().map_or(0, |c| c.cells.len())],
        });
    }

    // Column widths.
    let widths: Vec<usize> = columns
        .iter()
        .map(|column| {
            column
                .cells
                .iter()
                .map(|c| width(c))
                .chain([width(&column.name), width(&column.type_signature)])
                .max()
                .unwrap_or(0)
        })
        .collect();

    let pad = |text: &str, w: usize| format!(" {text}{} ", " ".repeat(w - width(text)));
    let rule = |left: &str, mid: &str, right: &str| {
        let segments: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{left}{}{right}", segments.join(mid))
    };

    let mut out = String::new();
    out.push_str(&rule("┌", "┬", "┐"));
    out.push('\n');
    // Two header lines: names, then type signatures.
    let names: Vec<String> = columns
        .iter()
        .zip(&widths)
        .map(|(c, w)| pad(&c.name, *w))
        .collect();
    out.push_str(&format!("│{}│\n", names.join("│")));
    // The type line is only drawn when at least one column carries a type signature
    // (a record's `field | value` table has none, so it stays a single-line header).
    if columns.iter().any(|c| !c.type_signature.is_empty()) {
        let types: Vec<String> = columns
            .iter()
            .zip(&widths)
            .map(|(c, w)| pad(&c.type_signature, *w))
            .collect();
        out.push_str(&format!("│{}│\n", types.join("│")));
    }
    out.push_str(&rule("├", "┼", "┤"));
    out.push('\n');

    let shown = columns.first().map_or(0, |c| c.cells.len());
    for row in 0..shown {
        let cells: Vec<String> = columns
            .iter()
            .zip(&widths)
            .map(|(c, w)| pad(&c.cells[row], *w))
            .collect();
        out.push_str(&format!("│{}│\n", cells.join("│")));
    }
    out.push_str(&rule("└", "┴", "┘"));
    if total > shown {
        out.push_str(&format!("\n… ({} more)", total - shown));
    }
    out
}

/// One column per field of a struct array, each field's values formatted, capped at
/// `max_rows` rows — the shared builder behind a struct serie's table and a record's.
fn struct_columns(entries: &arrow_array::StructArray, max_rows: usize) -> Vec<Column> {
    let (names, types): (Vec<String>, Vec<String>) = match entries.data_type() {
        DataType::Struct(fields) => fields
            .iter()
            .map(|f| {
                (
                    f.name().to_string(),
                    yggdryl_dtype::signature(f.data_type()),
                )
            })
            .unzip(),
        _ => (Vec::new(), Vec::new()),
    };
    entries
        .columns()
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let shown = child.len().min(max_rows);
            Column {
                name: names.get(index).cloned().unwrap_or_default(),
                type_signature: types.get(index).cloned().unwrap_or_default(),
                cells: (0..shown).map(|row| format_arrow(child, row, 0)).collect(),
            }
        })
        .collect()
}

/// Render a serie **column** as a table: a struct column becomes one column per field
/// (recursively, nested values shown compactly), any other column a single column
/// headed by `item_name` and the element type. The shared renderer behind every
/// serie's `display`.
pub(crate) fn render_serie(
    column: &crate::AnySerie,
    item_name: &str,
    options: DisplayOptions,
) -> String {
    render_serie_with_total(column, item_name, column.len(), options)
}

/// [`render_serie`] with the true element `total` supplied separately, so a caller
/// that has already trimmed `column` to the first [`max_rows`](DisplayOptions::max_rows)
/// entries (a typed map, which must assemble its entries) still gets the right
/// `… (N more)` footer without materializing the whole thing.
pub(crate) fn render_serie_with_total(
    column: &crate::AnySerie,
    item_name: &str,
    total: usize,
    options: DisplayOptions,
) -> String {
    let arrow = column.to_arrow();
    if let DataType::Struct(_) = arrow.data_type() {
        if let Some(entries) = arrow.as_any().downcast_ref::<arrow_array::StructArray>() {
            return render_table(struct_columns(entries, options.max_rows), total, options);
        }
    }
    let shown = column.len().min(options.max_rows);
    let single = Column {
        name: item_name.to_string(),
        type_signature: yggdryl_dtype::signature(arrow.data_type()),
        cells: (0..shown)
            .map(|index| {
                column
                    .get_scalar(index)
                    .map_or_else(|| "null".to_string(), |any| format_any(&any))
            })
            .collect(),
    };
    render_table(vec![single], total, options)
}

/// Render a single struct row (a [`RecordScalar`](crate::RecordScalar)) as a
/// one-row-per-field table (`field | type | value`), so a record reads like a
/// transposed one-row serie.
pub(crate) fn render_record(record: &crate::RecordScalar, _options: DisplayOptions) -> String {
    use crate::Scalar;
    if record.is_null() {
        return "null".to_string();
    }
    let arrow = record.to_arrow_scalar();
    let entries = match arrow.as_any().downcast_ref::<arrow_array::StructArray>() {
        Some(entries) => entries,
        None => return "null".to_string(),
    };
    let columns = struct_columns(entries, 1);
    // Transpose to a two-column `field | value` table so a wide record still fits.
    let rows: Vec<Column> = vec![
        Column {
            name: "field".to_string(),
            type_signature: "".to_string(),
            cells: columns
                .iter()
                .map(|c| format!("{}: {}", c.name, c.type_signature))
                .collect(),
        },
        Column {
            name: "value".to_string(),
            type_signature: "".to_string(),
            cells: columns
                .iter()
                .map(|c| c.cells.first().cloned().unwrap_or_default())
                .collect(),
        },
    ];
    render_table(rows, columns.len(), DisplayOptions::default())
}
