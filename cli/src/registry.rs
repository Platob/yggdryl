//! Reading, writing and reporting on one dictionary.
//!
//! Every command is a function of a loaded registry and its arguments, so the
//! interactive shell and the one-shot invocation run exactly the same code -
//! there is no second path where a shell command could drift from the flag it
//! mirrors.

use std::path::{Path, PathBuf};

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixBranch, FixCodeValue, FixRegistry, Result};

use crate::style;

/// Where a dictionary lives, and what it holds.
/// One path as a handle can address it.
///
/// Absolute, because a handle is addressed by URL and a relative path is not
/// one. Resolved against the working directory the way every other tool
/// resolves a path argument, so every location this tool is given passes
/// through here before it becomes a URL.
///
/// # Errors
///
/// Returns the failure reading the working directory raises.
pub fn located(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub struct Store {
    root: PathBuf,
    registry: FixRegistry,
    /// What was loaded, kept so a change can be described rather than guessed.
    original: FixRegistry,
}

impl Store {
    /// Opens the dictionary at `root`, or an empty one where there is none.
    ///
    /// # Errors
    ///
    /// Returns the store's own refusal when the folder exists and does not
    /// hold a dictionary.
    pub fn open(root: &Path) -> Result<Self> {
        let root = located(root)?;
        let registry = if root.join("primitive").exists() || root.join("nested").exists() {
            FixRegistry::from_handle(&Folder::new(root.clone())?)?
        } else {
            FixRegistry::new()
        };
        Ok(Self {
            root,
            original: registry.clone(),
            registry,
        })
    }

    /// The dictionary as loaded and edited.
    #[must_use]
    pub const fn registry(&self) -> &FixRegistry {
        &self.registry
    }

    /// The dictionary, to edit.
    pub const fn registry_mut(&mut self) -> &mut FixRegistry {
        &mut self.registry
    }

    /// Whether anything has been changed since it was loaded.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.registry != self.original
    }

    /// Writes the dictionary back where it came from.
    ///
    /// # Errors
    ///
    /// Returns the store's own refusal when the folder cannot be written.
    pub fn save(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let mut folder = Folder::new(self.root.clone())?;
        self.registry.write_into(&mut folder)?;
        self.original = self.registry.clone();
        Ok(())
    }

    /// Where this dictionary lives.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// One field's row in a listing.
fn row(field: &Field) -> Vec<String> {
    let view = field.as_fix();
    let tag = view
        .tag()
        .ok()
        .flatten()
        .map_or_else(|| "-".to_owned(), |held| held.to_string());
    let branch = view
        .branch()
        .ok()
        .filter(|held| !held.is_standard())
        .map_or_else(String::new, |held| held.name().to_owned());
    let codes = view.codes().count();
    vec![
        tag,
        field.name().to_owned(),
        field.dtype().to_string(),
        branch,
        if codes == 0 {
            String::new()
        } else {
            codes.to_string()
        },
        view.description().unwrap_or_default().to_owned(),
    ]
}

/// The header every listing shares.
const COLUMNS: [&str; 6] = ["tag", "name", "type", "branch", "codes", "description"];

/// Lists the fields whose name or tag contains `filter`.
pub fn list(store: &Store, filter: Option<&str>, limit: usize) {
    let folded = filter.map(str::to_lowercase);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut matched = 0_usize;
    for field in store.registry() {
        if let Some(held) = &folded {
            let tag = field
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .map_or_else(String::new, |tag| tag.to_string());
            if !field.name().to_lowercase().contains(held) && !tag.contains(held) {
                continue;
            }
        }
        matched += 1;
        if rows.len() < limit {
            rows.push(row(field));
        }
    }
    style::table(&COLUMNS, &rows);
    if matched > rows.len() {
        style::note(&format!(
            "{matched} matched, {} shown - raise --limit to see more",
            rows.len()
        ));
    } else {
        style::note(&format!("{matched} field(s)"));
    }
}

/// Shows one field in full: what it is, what it was, and what it may hold.
pub fn show(store: &Store, key: &str) -> Result<()> {
    let field = resolve(store.registry(), key)?;
    let view = field.as_fix();

    style::heading(field.name());
    style::entry(
        "tag",
        &view.tag()?.map_or_else(|| "-".into(), |t| t.to_string()),
    );
    style::entry(
        "identity",
        &view.id()?.map_or_else(|| "-".into(), |id| id.to_string()),
    );
    style::entry("type", &field.dtype().to_string());
    style::entry("nullable", if field.is_nullable() { "yes" } else { "no" });
    if let Ok(branch) = view.branch() {
        if !branch.is_standard() {
            style::entry("branch", branch.name());
        }
    }
    if let Some(described) = view.description() {
        style::entry("description", described);
    }
    let aliases: Vec<&str> = view.aliases().collect();
    if !aliases.is_empty() {
        style::entry("aliases", &aliases.join(", "));
    }

    let lineage: Vec<Vec<String>> = view
        .lineage()
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            vec![
                entry.since().to_string(),
                entry.ep().map_or_else(String::new, |ep| ep.to_string()),
                entry.name().unwrap_or_default().to_owned(),
                entry
                    .parse_dtype()
                    .ok()
                    .flatten()
                    .map(|dtype| dtype.to_string())
                    .unwrap_or_default(),
                if entry.is_deprecated() {
                    "deprecated"
                } else {
                    ""
                }
                .to_owned(),
            ]
        })
        .collect();
    if !lineage.is_empty() {
        style::heading("lineage");
        style::table(&["since", "ep", "name", "type", "state"], &lineage);
    }

    let codes: Vec<Vec<String>> = view
        .codes()
        .filter_map(std::result::Result::ok)
        .map(|code| {
            vec![
                code.value().to_owned(),
                code.name().to_owned(),
                code.since().map_or_else(String::new, |v| v.to_string()),
                code.deprecated()
                    .map_or_else(String::new, |v| v.to_string()),
                code.aliases().collect::<Vec<_>>().join(", "),
                code.doc().unwrap_or_default().to_owned(),
            ]
        })
        .collect();
    if !codes.is_empty() {
        style::heading("codes");
        style::table(
            &["value", "name", "since", "deprecated", "aliases", "doc"],
            &codes,
        );
    }
    Ok(())
}

/// The field a key reaches, by tag, identifier, name or path.
pub fn resolve<'registry>(registry: &'registry FixRegistry, key: &str) -> Result<&'registry Field> {
    if let Ok(tag) = key.parse::<i32>() {
        return registry.field_by_tag(tag);
    }
    if key.contains(':') {
        return registry.field_by_id(key.parse()?);
    }
    registry.field_by_path(key, None)
}

/// Creates or replaces one field.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the type does not parse, or the
/// registry's when the identity is taken by something else.
pub fn put(
    store: &mut Store,
    name: &str,
    dtype: &str,
    tag: i32,
    branch: Option<&str>,
    description: Option<&str>,
) -> Result<()> {
    let parsed = DataType::from_str(dtype)?;
    let mut field = parsed.nullable_field(name.to_ascii_lowercase());
    match branch {
        Some(held) => field
            .as_fix_mut()
            .set_id(&FixBranch::from_str(held)?, tag)?,
        None => field.as_fix_mut().set_tag(tag)?,
    }
    if let Some(described) = description {
        field.as_fix_mut().set_description(described)?;
    }
    let replaced = store.registry_mut().insert(field)?;
    if replaced.is_some() {
        style::good(&format!("replaced {name} at tag {tag}"));
    } else {
        style::good(&format!("added {name} at tag {tag}"));
    }
    Ok(())
}

/// Removes the field a key reaches.
///
/// # Errors
///
/// Returns a typed absence when nothing holds that key.
pub fn remove(store: &mut Store, key: &str) -> Result<()> {
    let name = resolve(store.registry(), key)?.name().to_owned();
    let tag = resolve(store.registry(), key)?
        .as_fix()
        .tag()?
        .unwrap_or_default();
    store
        .registry_mut()
        .remove(tag)
        .ok_or_else(|| yggdryl::Error::Absent {
            expected: "a field",
            path: name.clone().into(),
        })?;
    style::good(&format!("removed {name}"));
    Ok(())
}

/// What one field's code set holds, for a caller rendering it.
#[allow(dead_code)]
pub fn codes(field: &Field) -> Vec<FixCodeValue<'_>> {
    field
        .as_fix()
        .codes()
        .filter_map(std::result::Result::ok)
        .collect()
}
