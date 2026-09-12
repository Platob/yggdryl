//! Reading, writing and reporting on one dictionary.
//!
//! Every command is a function of a loaded registry and its arguments, so the
//! interactive shell and the one-shot invocation run exactly the same code -
//! there is no second path where a shell command could drift from the flag it
//! mirrors.

use std::path::{Path, PathBuf};

use yggdryl::holder::local::Folder;
use yggdryl::{Field, FixCategory, FixRegistry, Result};

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
        let registry = FixRegistry::from_handle(&Folder::new(root.clone())?)?;
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
    vec![
        tag,
        field.name().to_owned(),
        field.dtype().to_string(),
        view.branches().collect::<Vec<_>>().join(", "),
        view.description().unwrap_or_default().to_owned(),
    ]
}

/// The header every listing shares.
const COLUMNS: [&str; 5] = ["tag", "name", "type", "dialects", "description"];

/// Lists the fields whose name or tag contains `filter`.
///
/// `dialect` keeps only the definitions whose `fix:branches` membership
/// names that dictionary; it is a filter on provenance and changes nothing
/// about how a key resolves.
pub fn list(
    store: &Store,
    category: FixCategory,
    filter: Option<&str>,
    dialect: Option<&str>,
    limit: usize,
) {
    let folded = filter.map(str::to_lowercase);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut matched = 0_usize;
    for field in store.registry().definitions(category) {
        if let Some(dialect) = dialect {
            if !field.as_fix().has_branch(dialect) {
                continue;
            }
        }
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
        style::note(&format!("{matched} {category}"));
    }
}

/// Shows one field in full: what it is, what it was, and what it may hold.
pub fn read(store: &Store, category: FixCategory, key: &str, json: bool) -> Result<()> {
    let field = resolve(store.registry(), category, key)?;
    if json {
        println!("{}", field.clone().into_json()?);
        return Ok(());
    }
    let view = field.as_fix();

    style::heading(field.name());
    style::entry("category", category.as_str());
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
    let dialects: Vec<&str> = view.branches().collect();
    if !dialects.is_empty() {
        style::entry("dialects", &dialects.join(", "));
    }
    if let Some(described) = view.description() {
        style::entry("description", described);
    }
    if let Some(counter) = view.counter()? {
        style::entry("counter", &counter.to_string());
    }
    for (key, value) in [("component", view.component()), ("msgtype", view.msgtype())] {
        if let Some(value) = value {
            style::entry(key, value);
        }
    }
    let aliases: Vec<&str> = view.aliases().collect();
    if !aliases.is_empty() {
        style::entry("aliases", &aliases.join(", "));
    }

    let lineage: Vec<Vec<String>> = view
        .lineage()
        .map(|entry| {
            let entry = entry?;
            Ok(vec![
                entry.since().to_string(),
                entry.ep().map_or_else(String::new, |ep| ep.to_string()),
                entry.name().unwrap_or_default().to_owned(),
                entry
                    .parse_dtype()?
                    .map(|dtype| dtype.to_string())
                    .unwrap_or_default(),
                if entry.is_deprecated() {
                    "deprecated"
                } else {
                    ""
                }
                .to_owned(),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    if !lineage.is_empty() {
        style::heading("lineage");
        style::table(&["since", "ep", "name", "type", "state"], &lineage);
    }

    let codes: Vec<Vec<String>> = view
        .codes()
        .map(|code| {
            let code = code?;
            Ok(vec![
                code.value().to_owned(),
                code.name().to_owned(),
                code.since().map_or_else(String::new, |v| v.to_string()),
                code.deprecated()
                    .map_or_else(String::new, |v| v.to_string()),
                code.aliases().collect::<Vec<_>>().join(", "),
                code.doc().unwrap_or_default().to_owned(),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    if !codes.is_empty() {
        style::heading("codes");
        style::table(
            &["value", "name", "since", "deprecated", "aliases", "doc"],
            &codes,
        );
    }
    Ok(())
}

/// The field a key reaches, by tag, name or path.
///
/// A decimal key is a tag and never an identity: the canonical holder of the
/// tag answers, then an alternate. Anything else is a name, resolved under
/// the registry's one fold.
pub fn resolve<'registry>(
    registry: &'registry FixRegistry,
    category: FixCategory,
    key: &str,
) -> Result<&'registry Field> {
    if category == FixCategory::Fields {
        if let Ok(tag) = key.parse::<i32>() {
            return registry.field_by_tag(tag);
        }
    }
    registry.definition(category, key)
}

/// Creates a definition, refusing an existing identity atomically.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the type does not parse, or the
/// registry's when the identity is taken by something else.
pub fn create(store: &mut Store, category: FixCategory, field: Field) -> Result<()> {
    let name = field.name().to_owned();
    store.registry_mut().create_definition(category, field)?;
    style::good(&format!("created {category}/{name}"));
    Ok(())
}

/// Replaces an existing definition, refusing absence atomically.
pub fn update(store: &mut Store, category: FixCategory, field: Field) -> Result<()> {
    let name = field.name().to_owned();
    store.registry_mut().update_definition(category, field)?;
    style::good(&format!("updated {category}/{name}"));
    Ok(())
}

/// Removes the field a key reaches.
///
/// # Errors
///
/// Returns a typed absence when nothing holds that key.
pub fn delete(store: &mut Store, category: FixCategory, key: &str) -> Result<()> {
    let field = resolve(store.registry(), category, key)?;
    let name = field.name().to_owned();
    store
        .registry_mut()
        .remove_definition(category, &name)?
        .ok_or_else(|| yggdryl::Error::Absent {
            expected: "a FIX definition",
            path: name.clone().into(),
        })?;
    style::good(&format!("deleted {category}/{name}"));
    Ok(())
}
