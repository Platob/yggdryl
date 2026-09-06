//! What one dictionary changed against another.
//!
//! The command a review reads and a workflow gates on. Two dictionaries are
//! compared field by field on identity, so a rename shows as a rename rather
//! than as an addition beside a removal, and a retype shows what it was and
//! what it became.

use yggdryl::{Field, FixRegistry};

use crate::style;

/// What happened to one field.
pub enum Change {
    /// It was not there before.
    Added(String, String),
    /// It is not there now.
    Removed(String, String),
    /// It is there and different.
    Changed {
        /// The tag it is under.
        tag: String,
        /// What it is called now.
        name: String,
        /// What differs, one phrase per difference.
        details: Vec<String>,
    },
}

impl Change {
    /// The word this change prints as.
    const fn word(&self) -> &'static str {
        match self {
            Self::Added(..) => "added",
            Self::Removed(..) => "removed",
            Self::Changed { .. } => "changed",
        }
    }
}

/// Every difference between two dictionaries, in tag order.
#[must_use]
pub fn compare(before: &FixRegistry, after: &FixRegistry) -> Vec<Change> {
    let mut changes = Vec::new();
    for held in after {
        let tag = tag_of(held);
        match before.get_field_by_id(identity(held)) {
            None => changes.push(Change::Added(tag, held.name().to_owned())),
            Some(was) => {
                let details = differences(was, held);
                if !details.is_empty() {
                    changes.push(Change::Changed {
                        tag,
                        name: held.name().to_owned(),
                        details,
                    });
                }
            }
        }
    }
    for held in before {
        if after.get_field_by_id(identity(held)).is_none() {
            changes.push(Change::Removed(tag_of(held), held.name().to_owned()));
        }
    }
    changes
}

/// One field's identity, or a standard tag where it declares none.
fn identity(field: &Field) -> yggdryl::FixId {
    field
        .as_fix()
        .id()
        .ok()
        .flatten()
        .unwrap_or_else(|| yggdryl::FixId::standard(0))
}

/// One field's tag, rendered.
fn tag_of(field: &Field) -> String {
    field
        .as_fix()
        .tag()
        .ok()
        .flatten()
        .map_or_else(|| "-".to_owned(), |held| held.to_string())
}

/// What differs between two versions of one field.
fn differences(was: &Field, now: &Field) -> Vec<String> {
    let mut held = Vec::new();
    if was.name() != now.name() {
        held.push(format!("renamed {:?} to {:?}", was.name(), now.name()));
    }
    if was.dtype() != now.dtype() {
        held.push(format!("retyped {} to {}", was.dtype(), now.dtype()));
    }
    if was.is_nullable() != now.is_nullable() {
        held.push(if now.is_nullable() {
            "became nullable".to_owned()
        } else {
            "became required".to_owned()
        });
    }
    let before = was.as_fix().codes().count();
    let after = now.as_fix().codes().count();
    if before != after {
        held.push(format!("code set {before} to {after}"));
    }
    let before = was.as_fix().description().unwrap_or_default();
    let after = now.as_fix().description().unwrap_or_default();
    if before != after {
        held.push("description changed".to_owned());
    }
    held
}

/// Prints the changes the way a person reads them.
pub fn render(changes: &[Change]) {
    if changes.is_empty() {
        style::good("no changes");
        return;
    }
    let rows: Vec<Vec<String>> = changes
        .iter()
        .map(|held| match held {
            Change::Added(tag, name) | Change::Removed(tag, name) => vec![
                held.word().to_owned(),
                tag.clone(),
                name.clone(),
                String::new(),
            ],
            Change::Changed { tag, name, details } => vec![
                held.word().to_owned(),
                tag.clone(),
                name.clone(),
                details.join("; "),
            ],
        })
        .collect();
    style::table(&["change", "tag", "field", "detail"], &rows);
    style::note(&format!("{} change(s)", changes.len()));
}

/// Prints the changes the way a workflow reads them.
pub fn annotate(changes: &[Change]) {
    for held in changes {
        match held {
            Change::Added(tag, name) | Change::Removed(tag, name) => {
                println!("::notice title=fix {}::{name} ({tag})", held.word());
            }
            Change::Changed { tag, name, details } => {
                println!(
                    "::notice title=fix changed::{name} ({tag}): {}",
                    details.join("; ")
                );
            }
        }
    }
}
