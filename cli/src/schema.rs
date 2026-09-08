//! The one row shape a whole capture lands in.
//!
//! A dictionary is what a message means; this is what a table holds. The
//! command exists because the two are asked about at different times: a desk
//! edits fields all day and dumps the schema once, when a table is created or
//! a downstream consumer wants to know what it is going to get.
//!
//! Nothing here decides the shape. [`fix_schema`](yggdryl::fix_schema) does,
//! from the dictionary that was loaded, and this prints it - so a schema
//! printed here and a schema a reader answers `schema()` with are the same
//! object built by the same code, never two spellings of one intention.

use std::path::Path;

use yggdryl::media::text::TextOptions;
use yggdryl::text::Formatting;
use yggdryl::{Field, FixRegistry, Result};

use crate::style;

/// The row a capture lands in, dictionary columns and all.
///
/// With no capture header this is the FIX half alone: the standard header, the
/// fields a consumer reads, the groups worth persisting, the trailer, this
/// crate's derived facts, and the two closing lists.
///
/// With one, the capture's own columns lead it - where the line was read from,
/// which line it was, and whatever the header names - because that is what a
/// monitor orders and joins on and it is gone by the time a frame is parsed.
///
/// # Errors
///
/// Returns the regex grammar's refusal when the header does not compile, or
/// the schema grammar's when the columns do not make a struct.
pub fn build(registry: &FixRegistry, rowheader: Option<&str>, name: &str) -> Result<Field> {
    let read = yggdryl::fix_schema(registry, name.to_owned())?;
    let Some(header) = rowheader else {
        return Ok(read);
    };
    // A capture spans venues, so a naive stamp is read as UTC rather than as
    // whatever the machine printing it happened to be set to.
    let mut options = TextOptions::new().with_timezone(yggdryl::Timezone::UTC);
    // From one, because a line number a person reads is the line they would
    // count to in an editor.
    options.start_rownum = Some(1);
    options.set_rowheader(Some(header))?;
    let carrier = options.source_field()?;
    yggdryl::fix_schema_carrying(&carrier, &read)
}

/// Prints the row, or writes it where it was asked for.
///
/// JSON when it goes to a file, because that is what a downstream consumer
/// reads and it is the crate's own serialization rather than a rendering of
/// it. A table when it goes to a terminal, because nobody reads six thousand
/// lines of JSON at a prompt.
///
/// # Errors
///
/// Returns the serializer's refusal when the field does not render, or the
/// filesystem's when the path cannot be written.
pub fn render(field: &Field, out: Option<&Path>) -> Result<()> {
    let Some(path) = out else {
        show(field);
        return Ok(());
    };
    let rendered = field
        .clone()
        .into_json_with_formatting(Formatting::indented(2))?;
    if let Some(parent) = path.parent().filter(|held| !held.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, rendered.as_bytes())?;
    style::good(&format!(
        "{} column(s) written to {}",
        columns(field).len(),
        path.display()
    ));
    Ok(())
}

/// The columns, as a person reads them.
fn show(field: &Field) {
    let held = columns(field);
    let rows: Vec<Vec<String>> = held
        .iter()
        .map(|column| {
            let view = column.as_fix();
            vec![
                column.name().to_owned(),
                column.dtype().to_string(),
                view.tag()
                    .ok()
                    .flatten()
                    .map_or_else(String::new, |tag| tag.to_string()),
                column
                    .as_metadata()
                    .get("display")
                    .unwrap_or_default()
                    .to_owned(),
                column.description().unwrap_or_default().to_owned(),
            ]
        })
        .collect();
    style::heading(field.name());
    style::table(&["column", "type", "tag", "field", "description"], &rows);
    style::note(&format!("{} column(s)", held.len()));
}

/// One root's columns, or none where it is not a struct.
fn columns(field: &Field) -> Vec<Field> {
    field
        .dtype()
        .as_fields()
        .map(<[Field]>::to_vec)
        .unwrap_or_default()
}
