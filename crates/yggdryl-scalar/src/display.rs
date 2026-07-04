//! Pretty, debug-oriented display for scalars.
//!
//! An **atomic** scalar renders as its value (`42`, `1.5`, `"hi"`, `0x0102`,
//! `null`); a **serie** renders as a one-column table headed by its field (name and
//! type) with the first [`max_rows`](DisplayOptions::max_rows) elements; a **struct**
//! serie / record renders as a multi-column table (one column per field), and a
//! **map** as a two-column `key | value` table. Any value that lands in a *cell*
//! (a struct field, a list element, a map key/value) is shown compactly inline —
//! `{x: 1, name: "a"}`, `[1, 2, …]`, `{7: 42, …}`. Nesting past [`MAX_DEPTH`] collapses
//! to a bare `{…}` / `[…]`, which both keeps a pathological value readable and is the
//! backstop that stops a self-referential value from recursing forever.
//!
//! **Every value is formatted through the crate's own scalars** — the decomposed
//! numerics, then [`Utf8Scalar`](crate::Utf8Scalar) / [`BinaryScalar`](crate::BinaryScalar)
//! / [`RecordScalar`](crate::RecordScalar) / [`Serie`](crate::Serie) /
//! [`MapScalar`](crate::MapScalar) / [`OptionalScalar`](crate::OptionalScalar) — never by
//! reaching into an Arrow array; the data type only *routes* to the right scalar.
//!
//! Column widths are **adaptive**: each column is discovered from its content, then a
//! table too wide for [`max_width`](DisplayOptions::max_width) squeezes its *elastic*
//! columns (variable-length utf8 / binary / nested values — see [`value_profile`]) and
//! only drops whole columns into a trailing `…` column as a last resort, so a **rigid**
//! numeric column never has its digits truncated. The [`Display`](std::fmt::Display)
//! impls use the defaults ([`DisplayOptions::default`]); `display_with` takes an explicit
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
    /// The width, in characters, the table tries to fit — elastic (variable-length)
    /// columns are squeezed first, then trailing columns are dropped into a `…` column;
    /// rigid numeric columns keep their digits. Default `100`.
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
/// because every recursing arm (including an optional unwrapping its inner value) counts
/// against it — it is the hard backstop that stops a self-referential value from
/// recursing forever.
const MAX_DEPTH: usize = 5;

/// The most elements a nested `list` or `map` cell prints inline before it elides the
/// tail with `…`.
const MAX_INLINE: usize = 6;

/// The narrowest a column is ever shrunk to when fitting the width budget — narrow
/// enough to reclaim space, wide enough that `ab…` still says something.
const MIN_COL: usize = 3;

/// The width an **elastic** column (a variable-length utf8 / binary / nested value)
/// may be squeezed down to before whole columns start being dropped instead.
const ELASTIC_FLOOR: usize = 8;

/// Elide `text` to `max` characters (counting `char`s, good enough for debug), adding
/// a trailing `…` when cut. The result is always at most `max` characters.
fn elide(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    match max {
        0 => String::new(),
        1 => "…".to_string(),
        _ => {
            let mut out: String = text.chars().take(max - 1).collect();
            out.push('…');
            out
        }
    }
}

/// The datatype-derived rendering profile of a column value: the widest a single value
/// can print (`cap`), and whether the column is **elastic** — a variable-length value
/// (utf8, binary, or a nested list/struct/map) that may be abbreviated to fit the
/// screen, as opposed to a fixed-width number / bool / null whose digits must never be
/// truncated. This is where display leans on *our* data types: the type, not the
/// stringified value, decides how a column may be squeezed.
fn value_profile(data_type: &DataType) -> (usize, bool) {
    match data_type {
        DataType::Boolean => (5, false), // "false"
        DataType::Int8 => (4, false),    // "-128"
        DataType::Int16 => (6, false),   // "-32768"
        DataType::Int32 => (11, false),  // "-2147483648"
        DataType::Int64 => (20, false),  // "-9223372036854775808"
        DataType::UInt8 => (3, false),   // "255"
        DataType::UInt16 => (5, false),  // "65535"
        DataType::UInt32 => (10, false), // "4294967295"
        DataType::UInt64 => (20, false), // "18446744073709551615"
        DataType::Float16 => (12, false),
        DataType::Float32 => (16, false),
        DataType::Float64 => (24, false),
        DataType::Null => (4, false), // "null"
        // Everything variable-length (utf8, binary, list, struct, map, union, …) may
        // be elided to fit.
        _ => (MAX_CELL, true),
    }
}

/// The display width of `text` in characters.
fn width(text: &str) -> usize {
    text.chars().count()
}

/// Format a struct row as a compact inline cell `{name: v, …}`, reading each field
/// through the record's own [`AnyScalar`](crate::AnyScalar) — never an Arrow downcast.
fn format_record_cell(record: &crate::RecordScalar, depth: usize) -> String {
    use crate::Scalar;
    let Some(scalars) = record.value() else {
        return "null".to_string();
    };
    let fields = yggdryl_dtype::Struct::fields(record.data_type());
    let inner = scalars
        .iter()
        .zip(fields.iter())
        .map(|(scalar, field)| format!("{}: {}", field.name(), format_any_at(scalar, depth + 1)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{inner}}}")
}

/// Format a serie's items as a compact inline cell `[a, b, …]` (the first [`MAX_INLINE`]
/// shown), reading each element through the column's own [`AnyScalar`](crate::AnyScalar).
fn format_serie_cell(items: &crate::AnySerie, depth: usize) -> String {
    let shown = items.len().min(MAX_INLINE);
    let mut cells: Vec<String> = (0..shown)
        .map(|index| {
            items.get_any_scalar_at(index).map_or_else(
                || "null".to_string(),
                |scalar| format_any_at(&scalar, depth + 1),
            )
        })
        .collect();
    if items.len() > shown {
        cells.push("…".to_string());
    }
    format!("[{}]", cells.join(", "))
}

/// Format a map's entries as a compact inline cell `{k: v, …}` (the first [`MAX_INLINE`]
/// shown), reading the key and value columns through the map's own
/// [`AnySerie`](crate::AnySerie) projections.
fn format_map_cell(map: &crate::MapScalar, depth: usize) -> String {
    use crate::NestedSerie;
    let (Some(keys), Some(values)) = (map.child_serie_by("key"), map.child_serie_by("value"))
    else {
        return "null".to_string();
    };
    let len = keys.len().min(values.len());
    let shown = len.min(MAX_INLINE);
    let cell = |serie: &crate::AnySerie, index: usize| {
        serie.get_any_scalar_at(index).map_or_else(
            || "null".to_string(),
            |scalar| format_any_at(&scalar, depth + 1),
        )
    };
    let mut cells: Vec<String> = (0..shown)
        .map(|index| format!("{}: {}", cell(&keys, index), cell(&values, index)))
        .collect();
    if len > shown {
        cells.push("…".to_string());
    }
    format!("{{{}}}", cells.join(", "))
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
/// formatter behind every table cell and the atomic scalars' own display. Every value
/// is read through the crate's **own** scalars: the decomposed numerics, then
/// [`Utf8Scalar`](crate::Utf8Scalar) / [`BinaryScalar`](crate::BinaryScalar) /
/// [`RecordScalar`](crate::RecordScalar) / [`Serie`](crate::Serie) /
/// [`MapScalar`](crate::MapScalar) / [`OptionalScalar`](crate::OptionalScalar) — the
/// data type only *routes* to the right scalar, it is never formatted out of Arrow.
pub(crate) fn format_any(any: &AnyScalar) -> String {
    format_any_at(any, 0)
}

/// [`format_any`] carrying the current nesting `depth` (see [`MAX_DEPTH`]).
fn format_any_at(any: &AnyScalar, depth: usize) -> String {
    use crate::{BinaryScalar, MapScalar, OptionalScalar, RecordScalar, Scalar, Serie, Utf8Scalar};
    if any.is_null() {
        return "null".to_string();
    }
    // The decomposed numeric fast paths read the concrete scalar directly.
    macro_rules! numeric {
        ($($accessor:ident),+ $(,)?) => {
            $(if let Some(scalar) = any.$accessor() {
                if let Some(value) = Scalar::value(scalar) {
                    return value.to_string();
                }
            })+
        };
    }
    numeric!(int8, int16, int32, int64, uint8, uint16, uint32, uint64, float16, float32, float64);
    let data_type = any.data_type();
    // Past the depth budget a nested value collapses to a bare marker — both to keep a
    // pathological value readable and as the hard stop against a container type
    // recursing forever.
    if depth > MAX_DEPTH {
        return match data_type {
            DataType::Struct(_) | DataType::Map(..) | DataType::Union(..) => "{…}".to_string(),
            DataType::List(_) | DataType::LargeList(_) => "[…]".to_string(),
            _ => "…".to_string(),
        };
    }
    // Recover the value as its concrete scalar and read through *that*.
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 => any
            .unwrap::<Utf8Scalar>()
            .ok()
            .and_then(|scalar| scalar.value().map(|text| format!("{text:?}")))
            .unwrap_or_else(|| "null".to_string()),
        DataType::Binary | DataType::LargeBinary => any
            .unwrap::<BinaryScalar>()
            .ok()
            .and_then(|scalar| scalar.value().map(hex))
            .unwrap_or_else(|| "null".to_string()),
        DataType::Struct(_) => match any.unwrap::<RecordScalar>() {
            Ok(record) => format_record_cell(&record, depth),
            Err(_) => "{…}".to_string(),
        },
        DataType::List(_) | DataType::LargeList(_) => match any.unwrap::<Serie>() {
            Ok(serie) => serie.value().map_or_else(
                || "null".to_string(),
                |items| format_serie_cell(items, depth),
            ),
            Err(_) => "[…]".to_string(),
        },
        DataType::Map(..) => match any.unwrap::<MapScalar>() {
            Ok(map) => format_map_cell(&map, depth),
            Err(_) => "{…}".to_string(),
        },
        // An optional's union storage: recover it and format the inner value (or null),
        // recursing on the *inner* — never the union — so it always terminates.
        DataType::Union(..) => match any.unwrap::<OptionalScalar>() {
            Ok(optional) => match optional.value() {
                Some(child) => {
                    let inner = AnyScalar::from_arrow(arrow_array::make_array(child.to_data()));
                    format_any_at(&inner, depth + 1)
                }
                None => "null".to_string(),
            },
            Err(_) => "null".to_string(),
        },
        // `null`, and anything the crate's scalars do not model.
        _ => "null".to_string(),
    }
}

/// A single table column: its two-line header (name, then type signature), its
/// already-formatted cells, and — from the column's data type — how wide a value may
/// print (`cap`) and whether it is `elastic` (abbreviable to fit the screen).
pub(crate) struct Column {
    pub name: String,
    pub type_signature: String,
    pub cells: Vec<String>,
    cap: usize,
    elastic: bool,
}

impl Column {
    /// A column of `data_type` values — its width cap and elasticity come from the
    /// type (see [`value_profile`]).
    fn typed(
        name: String,
        type_signature: String,
        cells: Vec<String>,
        data_type: &DataType,
    ) -> Self {
        let (cap, elastic) = value_profile(data_type);
        Self {
            name,
            type_signature,
            cells,
            cap,
            elastic,
        }
    }

    /// A column of free-form text with no single data type — a record's `field` /
    /// `value` sides and the overflow `…` marker: elastic, capped generously.
    fn text(name: String, type_signature: String, cells: Vec<String>) -> Self {
        Self {
            name,
            type_signature,
            cells,
            cap: MAX_CELL,
            elastic: true,
        }
    }

    /// The width this column would take unsqueezed: the widest of its header lines and
    /// its cells.
    fn discovered_width(&self) -> usize {
        self.cells
            .iter()
            .map(|c| width(c))
            .chain([width(&self.name), width(&self.type_signature)])
            .max()
            .unwrap_or(0)
    }

    /// The narrowest this column may be squeezed to: the name always stays legible, a
    /// rigid number/bool/null keeps every character of its widest value, and an elastic
    /// value may shrink to [`ELASTIC_FLOOR`] — never below [`MIN_COL`].
    fn floor_width(&self) -> usize {
        let cell_max = self.cells.iter().map(|c| width(c)).max().unwrap_or(0);
        let value_floor = if self.elastic {
            cell_max.min(ELASTIC_FLOOR)
        } else {
            cell_max
        };
        width(&self.name).max(value_floor).max(MIN_COL)
    }
}

/// Shrink `widths` (each column's discovered width) toward the per-column `floors` so
/// the table fits `max_width` — squeezing the widest shrinkable column first — then
/// report how many leading columns survive alongside a trailing `…` marker (dropping
/// happens only once nothing more can be squeezed). Rigid columns floor at their value
/// width, so a table of only numbers drops columns rather than truncating digits.
fn fit_widths(widths: &mut [usize], floors: &[usize], max_width: usize) -> usize {
    // The rendered line is `│ w0 │ w1 │ … │`: one border plus `w + 3` per column.
    let line = |ws: &[usize]| 1 + ws.iter().map(|w| w + 3).sum::<usize>();
    let mut current = line(widths);
    while current > max_width {
        let widest = (0..widths.len())
            .filter(|&i| widths[i] > floors[i])
            .max_by_key(|&i| widths[i]);
        match widest {
            Some(i) => {
                widths[i] -= 1;
                current -= 1;
            }
            None => break, // nothing left to squeeze — fall through to dropping columns
        }
    }
    if current <= max_width {
        return widths.len();
    }
    // Still too wide: keep the longest leading run that fits beside a `…` marker
    // (reserve room for a `+NNN` column), always keeping at least the first column.
    const MARKER_COST: usize = 4 + 3;
    let mut kept = widths.len();
    while kept > 1 && line(&widths[..kept]) + MARKER_COST > max_width {
        kept -= 1;
    }
    kept
}

/// Render `columns` (all the same cell count) as a box-drawn table, honouring
/// [`DisplayOptions`]: at most `max_rows` body rows (with a `… (total more)` footer
/// past that) and an adaptive fit to `max_width` — cells are elided to their column's
/// datatype cap, then elastic columns are squeezed and, only as a last resort, trailing
/// columns collapse into a single `…` column.
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
    // Elide headers to a generous cap and every cell to its column's datatype cap.
    for column in &mut columns {
        column.name = elide(&column.name, MAX_CELL);
        column.type_signature = elide(&column.type_signature, MAX_CELL);
        let cap = column.cap;
        for cell in &mut column.cells {
            *cell = elide(cell, cap);
        }
    }

    // Discover each column's natural width and the floor it may be squeezed to, then
    // adaptively fit `max_width`: shrink elastic columns first, drop columns only when
    // nothing more can give.
    let mut widths: Vec<usize> = columns.iter().map(Column::discovered_width).collect();
    let floors: Vec<usize> = columns.iter().map(Column::floor_width).collect();
    let kept = fit_widths(&mut widths, &floors, options.max_width);
    let hidden_columns = columns.len() - kept;
    columns.truncate(kept);
    widths.truncate(kept);
    if hidden_columns > 0 {
        let rows = columns.first().map_or(0, |c| c.cells.len());
        let marker = Column::text(
            "…".to_string(),
            format!("+{hidden_columns}"),
            vec!["…".to_string(); rows],
        );
        widths.push(marker.discovered_width());
        columns.push(marker);
    }

    // Pad `text` into a `w`-wide cell, eliding first when a squeezed column is narrower
    // than its content (the header and rigid values were already sized to fit).
    let pad = |text: &str, w: usize| {
        let shown = elide(text, w);
        format!(" {shown}{} ", " ".repeat(w - width(&shown)))
    };
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

/// One column per field of a struct array, each field's values formatted **through the
/// crate's own scalars** (each child column decomposed to an [`AnySerie`](crate::AnySerie)
/// and read by [`get_any_scalar_at`](crate::AnySerie::get_any_scalar_at)), capped at `max_rows` rows —
/// the shared builder behind a struct serie's table and a record's. The struct layout
/// (field names, types) is navigated on the struct array; the values are not.
fn struct_columns(entries: &arrow_array::StructArray, max_rows: usize) -> Vec<Column> {
    let fields = match entries.data_type() {
        DataType::Struct(fields) => fields.clone(),
        _ => arrow_schema::Fields::empty(),
    };
    entries
        .columns()
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let field = fields.get(index);
            let name = field.map(|f| f.name().to_string()).unwrap_or_default();
            let type_signature = field
                .map(|f| yggdryl_dtype::signature(f.data_type()))
                .unwrap_or_default();
            // Read the field's values through our own column, not the Arrow array.
            let serie = crate::AnySerie::from_arrow(child.clone());
            let shown = serie.len().min(max_rows);
            let cells = (0..shown)
                .map(|row| {
                    serie
                        .get_any_scalar_at(row)
                        .map_or_else(|| "null".to_string(), |scalar| format_any(&scalar))
                })
                .collect();
            Column::typed(name, type_signature, cells, child.data_type())
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
    let cells = (0..shown)
        .map(|index| {
            column
                .get_any_scalar_at(index)
                .map_or_else(|| "null".to_string(), |any| format_any(&any))
        })
        .collect();
    let single = Column::typed(
        item_name.to_string(),
        yggdryl_dtype::signature(arrow.data_type()),
        cells,
        arrow.data_type(),
    );
    render_table(vec![single], total, options)
}

/// Render a single struct row (a [`RecordScalar`](crate::RecordScalar)) as a
/// one-row-per-field table (`field | value`), so a record reads like a transposed
/// one-row serie. The fields are read straight off the record's own
/// [`AnyScalar`](crate::AnyScalar)s — no Arrow round trip.
pub(crate) fn render_record(record: &crate::RecordScalar, options: DisplayOptions) -> String {
    use crate::Scalar;
    let Some(scalars) = record.value() else {
        return "null".to_string();
    };
    let fields = yggdryl_dtype::Struct::fields(record.data_type());
    // Transpose to a two-column `field | value` table so a wide record still fits.
    let field_cells: Vec<String> = fields
        .iter()
        .map(|f| format!("{}: {}", f.name(), yggdryl_dtype::signature(f.data_type())))
        .collect();
    let value_cells: Vec<String> = scalars.iter().map(format_any).collect();
    let rows = vec![
        Column::text("field".to_string(), String::new(), field_cells),
        Column::text("value".to_string(), String::new(), value_cells),
    ];
    render_table(rows, scalars.len(), options)
}
