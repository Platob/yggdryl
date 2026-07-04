//! Pretty, debug-oriented display for scalars.
//!
//! An **atomic** scalar renders as its value (`42`, `1.5`, `"hi"`, `0x0102`,
//! `null`); a **serie** renders as a one-column table headed by its field (name and
//! type) with the first [`max_rows`](DisplayOptions::max_rows) elements; a **struct**
//! serie / record renders as a multi-column table (one column per field), each nested
//! value shown compactly so the whole thing tries to fit the
//! [`max_width`](DisplayOptions::max_width). The [`Display`](std::fmt::Display) impls
//! use the defaults ([`DisplayOptions::default`]); `display_with` takes an explicit
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
/// nested types), and for the cells of a struct column.
fn format_arrow(array: &dyn Array, index: usize) -> String {
    use arrow_array::{
        BinaryArray, BooleanArray, LargeStringArray, ListArray, StringArray, StructArray,
    };
    if index >= array.len() || array.is_null(index) {
        return "null".to_string();
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
                    .map(|(column, name)| format!("{name}: {}", format_arrow(column, index)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{inner}}}")
            })
            .unwrap_or_else(|| "{…}".to_string()),
        DataType::List(_) => array
            .as_any()
            .downcast_ref::<ListArray>()
            .map(|list| {
                let element = list.value(index);
                let shown = element.len().min(6);
                let mut cells: Vec<String> =
                    (0..shown).map(|i| format_arrow(&element, i)).collect();
                if element.len() > shown {
                    cells.push("…".to_string());
                }
                format!("[{}]", cells.join(", "))
            })
            .unwrap_or_else(|| "[…]".to_string()),
        // Numeric / other leaves: read the one element through a one-row AnyScalar.
        _ => {
            let one = AnyScalar::from_arrow(array.slice(index, 1));
            format_any(&one)
        }
    }
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
        Some(array) => format_arrow(array.as_ref(), 0),
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
                cells: (0..shown).map(|row| format_arrow(child, row)).collect(),
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
    let arrow = column.to_arrow();
    if let DataType::Struct(_) = arrow.data_type() {
        if let Some(entries) = arrow.as_any().downcast_ref::<arrow_array::StructArray>() {
            return render_table(
                struct_columns(entries, options.max_rows),
                column.len(),
                options,
            );
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
    render_table(vec![single], column.len(), options)
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
