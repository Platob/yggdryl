//! [`DisplayOptions`] and the [`render`] routine behind
//! [`Serie::display`](crate::Serie::display) — the single, parametrised, readable string
//! view of a column. A leaf column renders **vertically** (one value per line); a struct
//! [frame](crate::StructSerie) renders as an **aligned table** (one column per child).
//! There is no separate `show` — `display` is the one render entry point.

use crate::serie::Serie;

/// Formatting parameters for [`Serie::display`](crate::Serie::display).
///
/// ```
/// use yggdryl_serie::{DisplayOptions, Serie, Int32Serie};
///
/// let serie = Int32Serie::from_values("n", (0..100).map(|i| Some(i)));
/// let text = serie.display(&DisplayOptions::default().with_max_rows(3));
/// assert!(text.contains("n: int32"));
/// assert!(text.contains("97 more rows"));
/// ```
#[derive(Debug, Clone)]
pub struct DisplayOptions {
    /// Maximum number of rows to render (`None` = all rows). Default `Some(10)`.
    pub max_rows: Option<usize>,
    /// Whether to print the `name: dtype` header (and its underline). Default `true`.
    pub header: bool,
    /// Fixed cell width — longer values are truncated with `…`, shorter ones padded
    /// (`None` = size to the widest shown value). Default `None`.
    pub width: Option<usize>,
    /// How a null value is rendered. Default `"null"`.
    pub null: String,
    /// Whether to print a leading row-index column. Default `false`.
    pub index: bool,
}

impl Default for DisplayOptions {
    fn default() -> DisplayOptions {
        DisplayOptions {
            max_rows: Some(10),
            header: true,
            width: None,
            null: "null".to_string(),
            index: false,
        }
    }
}

impl DisplayOptions {
    /// Sets the maximum number of rows to render.
    pub fn with_max_rows(mut self, max_rows: usize) -> DisplayOptions {
        self.max_rows = Some(max_rows);
        self
    }

    /// Renders every row (no row limit).
    pub fn with_all_rows(mut self) -> DisplayOptions {
        self.max_rows = None;
        self
    }

    /// Toggles the `name: dtype` header.
    pub fn with_header(mut self, header: bool) -> DisplayOptions {
        self.header = header;
        self
    }

    /// Fixes the cell width (truncating / padding to it).
    pub fn with_width(mut self, width: usize) -> DisplayOptions {
        self.width = Some(width);
        self
    }

    /// Sets the null rendering.
    pub fn with_null(mut self, null: impl Into<String>) -> DisplayOptions {
        self.null = null.into();
        self
    }

    /// Toggles the leading row-index column.
    pub fn with_index(mut self, index: bool) -> DisplayOptions {
        self.index = index;
        self
    }
}

/// Truncates `text` to `width` display characters, appending `…` when it overflows.
fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        format!("{text:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let kept: String = text.chars().take(width - 1).collect();
        format!("{kept}…")
    }
}

/// Renders `serie` to a readable, parametrised string. The shared implementation behind
/// [`Serie::display`](crate::Serie::display): a struct [frame](crate::StructSerie) renders
/// as an aligned table, every other column vertically.
pub(crate) fn render(serie: &(impl Serie + ?Sized), opts: &DisplayOptions) -> String {
    if let Some(frame) = serie.as_any().downcast_ref::<crate::StructSerie>() {
        return render_table(frame, opts);
    }
    let len = serie.len();
    let shown = opts.max_rows.map_or(len, |m| m.min(len));

    // cell text for each shown row
    let cells: Vec<String> = (0..shown)
        .map(|i| {
            let value = serie.value_at(i);
            if value.is_null() {
                opts.null.clone()
            } else {
                value.to_string()
            }
        })
        .collect();

    let header_text = format!("{}: {}", serie.name(), serie.data_type().to_str());
    let natural = cells
        .iter()
        .map(|c| c.chars().count())
        .chain(opts.header.then(|| header_text.chars().count()))
        .max()
        .unwrap_or(0);
    let width = opts.width.unwrap_or(natural).max(1);

    // row-index gutter width
    let gutter = if opts.index {
        shown.saturating_sub(1).to_string().len().max(1)
    } else {
        0
    };
    // The full gutter width incl. the two trailing spaces `pad` emits (0 when off) —
    // used to indent the header rows without formatting a throwaway sample string.
    let gutter_width = if opts.index { gutter + 2 } else { 0 };
    let pad = |idx: usize| {
        if opts.index {
            format!("{idx:>gutter$}  ")
        } else {
            String::new()
        }
    };

    let mut lines = Vec::with_capacity(shown + 3);
    if opts.header {
        lines.push(format!(
            "{}{}",
            " ".repeat(gutter_width),
            fit(&header_text, width)
        ));
        lines.push(format!("{}{}", " ".repeat(gutter_width), "─".repeat(width)));
    }
    for (i, cell) in cells.iter().enumerate() {
        lines.push(format!("{}{}", pad(i), fit(cell, width)));
    }
    if len > shown {
        lines.push(format!("… ({} more rows)", len - shown));
    }
    lines.join("\n")
}

/// Renders a struct [`StructSerie`](crate::StructSerie) as an aligned table — one column
/// per child field, `name: type` headers, at most `opts.max_rows` rows, honouring the
/// same [`DisplayOptions`] (`header` / `null` / `width` / `index`). The frame view behind
/// [`Serie::display`](crate::Serie::display) for struct columns (the former `show`).
fn render_table(frame: &crate::StructSerie, opts: &DisplayOptions) -> String {
    let cols = frame.children();
    let rows = frame.len();
    if cols.is_empty() {
        return format!("empty frame ({rows} rows, 0 columns)");
    }
    let shown = opts.max_rows.map_or(rows, |m| m.min(rows));

    let headers: Vec<String> = cols
        .iter()
        .map(|c| format!("{}: {}", c.name(), c.data_type().to_str()))
        .collect();
    // cell text, column-major (one Vec per column).
    let cells: Vec<Vec<String>> = cols
        .iter()
        .map(|c| {
            (0..shown)
                .map(|r| {
                    let value = c.value_at(r);
                    if value.is_null() {
                        opts.null.clone()
                    } else {
                        value.to_string()
                    }
                })
                .collect()
        })
        .collect();
    let widths: Vec<usize> = (0..cols.len())
        .map(|ci| {
            let natural = cells[ci]
                .iter()
                .map(|s| s.chars().count())
                .chain(opts.header.then(|| headers[ci].chars().count()))
                .max()
                .unwrap_or(0);
            opts.width.unwrap_or(natural).max(1)
        })
        .collect();

    // optional leading row-index gutter (`Some(i)` numbers a data row, `None` indents a
    // header row by the full gutter width).
    let gutter = if opts.index {
        shown.saturating_sub(1).to_string().len().max(1)
    } else {
        0
    };
    let gutter_width = if opts.index { gutter + 2 } else { 0 };
    let pad = |idx: Option<usize>| -> String {
        match (opts.index, idx) {
            (false, _) => String::new(),
            (true, Some(i)) => format!("{i:>gutter$}  "),
            (true, None) => " ".repeat(gutter_width),
        }
    };
    let join_row = |fields: &[String]| -> String {
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| fit(f, widths[i]))
            .collect::<Vec<_>>()
            .join(" | ")
    };

    let mut lines = Vec::with_capacity(shown + 3);
    if opts.header {
        lines.push(format!("{}{}", pad(None), join_row(&headers)));
        let underline: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
        lines.push(format!("{}{}", pad(None), underline.join("─┼─")));
    }
    for r in 0..shown {
        let row: Vec<String> = cells.iter().map(|col| col[r].clone()).collect();
        lines.push(format!("{}{}", pad(Some(r)), join_row(&row)));
    }
    if rows > shown {
        lines.push(format!("… ({} more rows)", rows - shown));
    }
    lines.join("\n")
}
