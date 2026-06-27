//! [`DisplayOptions`] and the [`render`] routine behind
//! [`Serie::display`](crate::Serie::display) — a parametrised, readable string view of a
//! column (and the building block for a future `Frame`'s table rendering).

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
/// [`Serie::display`](crate::Serie::display).
pub(crate) fn render(serie: &(impl Serie + ?Sized), opts: &DisplayOptions) -> String {
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
            " ".repeat(pad(0).len()),
            fit(&header_text, width)
        ));
        lines.push(format!("{}{}", " ".repeat(pad(0).len()), "─".repeat(width)));
    }
    for (i, cell) in cells.iter().enumerate() {
        lines.push(format!("{}{}", pad(i), fit(cell, width)));
    }
    if len > shown {
        lines.push(format!("… ({} more rows)", len - shown));
    }
    lines.join("\n")
}
