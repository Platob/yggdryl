//! What one dictionary changed against another.
//!
//! The command a review reads and a workflow gates on. Two dictionaries are
//! compared field by field on identity, so a rename shows as a rename rather
//! than as an addition beside a removal, and a retype shows what it was and
//! what it became.

use yggdryl::{Field, FixCategory, FixRegistry};

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
    for category in FixCategory::ALL {
        for held in after.definitions(category) {
            let tag = key_of(category, held);
            match previous(before, category, held) {
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
        for held in before.definitions(category) {
            if previous(after, category, held).is_none() {
                changes.push(Change::Removed(
                    key_of(category, held),
                    held.name().to_owned(),
                ));
            }
        }
    }
    changes
}

fn previous<'registry>(
    registry: &'registry FixRegistry,
    category: FixCategory,
    field: &Field,
) -> Option<&'registry Field> {
    if category == FixCategory::Fields {
        return registry.get_field_by_id(identity(field));
    }
    let branch = field.as_fix().branch().ok()?;
    registry.get_definition(category, field.name(), Some(&branch))
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
fn key_of(category: FixCategory, field: &Field) -> String {
    if category == FixCategory::Fields {
        format!("{category}/{}", identity(field))
    } else {
        let branch = field
            .as_fix()
            .branch()
            .ok()
            .filter(|branch| !branch.is_standard())
            .map_or_else(String::new, |branch| format!("{}/", branch.name()));
        format!("{category}/{branch}{}", field.name())
    }
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
    if was.as_metadata() != now.as_metadata() {
        held.push("metadata changed".to_owned());
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
    style::table(&["change", "identity", "definition", "detail"], &rows);
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
