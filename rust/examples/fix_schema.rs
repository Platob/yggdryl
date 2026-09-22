//! Prints the fixed FIX row's Arrow batch schema, one column a line.
//!
//! The row every parsed message answers as - [`yggdryl::fix_schema`] over the
//! committed dictionary, which is also what `components/fixmsg.json` states -
//! read as the Arrow schema a batch of them carries. Arrow's own rendering of
//! a nested column is its whole subtree with every field's metadata inline,
//! which is thousands of characters for one repeating group; this prints the
//! shape instead - the members a group's occurrence declares, the key and
//! value a map holds - so the row fits on a screen.
//!
//! ```text
//! cargo run --example fix_schema --features arrow
//! ```

use std::sync::Arc;

use arrow_schema::{DataType, Field};
use yggdryl::local::LocalFolder;
use yggdryl::{FixRegistry, fix_schema};

fn main() -> yggdryl::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let schema = fix_schema(&registry, "fixmsg")?;
    let arrow = schema.clone().into_arrow_schema()?;
    println!("fixmsg: {} columns", arrow.fields().len());
    for (column, field) in schema.fields().iter().zip(arrow.fields()) {
        // The tag is the column's own; a column no tag names - the arrival
        // record - is this crate's and says so with a dash.
        let tag = column
            .as_fix()
            .tag()
            .ok()
            .flatten()
            .map_or_else(|| "-".to_owned(), |tag| tag.to_string());
        let required = if field.is_nullable() {
            ""
        } else {
            "  not null"
        };
        println!("{tag:>6}  {:<22} {}{required}", field.name(), spell(field));
    }
    Ok(())
}

/// One Arrow type as this row reads it: a leaf by its logical name, a map by
/// its key and value, a list by its item, and a struct by its members' names
/// alone - a member's own type is that member's line when it has one, and
/// naming it here would print the dictionary twice.
fn spell(field: &Field) -> String {
    match field.data_type() {
        DataType::Struct(members) => {
            let names: Vec<&str> = members
                .iter()
                .map(|member| member.name().as_str())
                .collect();
            format!("{{{}}}", names.join(", "))
        }
        // A group's item is the occurrence component, and its name is what
        // the dictionary calls one member of the group; a list of anything
        // else is named by nothing but its type.
        DataType::List(item) | DataType::LargeList(item) => match item.data_type() {
            DataType::Struct(_) => format!("list<{}{}>", item.name(), spell(item)),
            _ => format!("list<{}>", spell(item)),
        },
        DataType::Map(entries, _) => match entries.data_type() {
            DataType::Struct(pair) if pair.len() == 2 => {
                format!("map<{}, {}>", spell(&pair[0]), spell(&pair[1]))
            }
            _ => "map".to_owned(),
        },
        DataType::Timestamp(unit, zone) => {
            let unit = match unit {
                arrow_schema::TimeUnit::Second => "s",
                arrow_schema::TimeUnit::Millisecond => "ms",
                arrow_schema::TimeUnit::Microsecond => "us",
                arrow_schema::TimeUnit::Nanosecond => "ns",
            };
            match zone {
                Some(zone) => format!("timestamp[{unit}, {zone}]"),
                None => format!("timestamp[{unit}]"),
            }
        }
        DataType::Decimal128(precision, scale) => format!("decimal({precision}, {scale})"),
        DataType::FixedSizeBinary(width) => match field.metadata().get("ARROW:extension:name") {
            Some(name) => name.clone(),
            None => format!("binary[{width}]"),
        },
        held => format!("{held}").to_lowercase(),
    }
}
