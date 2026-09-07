//! `ygg fix` - manage a yggdryl FIX dictionary from a terminal.
//!
//! Everything a desk does to a registry: read it, search it, change it,
//! ingest a counterparty's configuration into it, and check that what came
//! out is right. Two audiences, one implementation - a person at a prompt
//! and a workflow gating a pull request run the same code, and the only
//! difference is that one of them gets colour.
//!
//! # The commands
//!
//! | command | what it does |
//! | --- | --- |
//! | *none* | all of the below, interactively, with completion |
//! | `list` | every field, filtered |
//! | `show` | one field: identity, lineage, codes |
//! | `set` | create or replace a field |
//! | `rm` | remove a field |
//! | `ingest` | read a `.cfb` into the dictionary, creating or merging |
//! | `schema` | the one row shape a whole capture lands in |
//! | `check` | what the dictionary is wrong about |
//! | `diff` | what changed against another dictionary |

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use yggdryl::holder::Holder;
use yggdryl::holder::local::Folder;
use yggdryl::{FixBranch, FixField, FixRegistry, IOKind, Result};

use crate::{diff, quality, registry, schema, shell, style};

/// What the dictionary tool was asked to do.
#[derive(Subcommand)]
pub enum Command {
    /// List the fields a dictionary holds.
    List {
        /// Keep only fields whose name or tag contains this.
        filter: Option<String>,
        /// How many to print.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Show one field in full.
    Show {
        /// A tag, an identifier, a name, or a dotted path.
        key: String,
    },
    /// Create or replace one field.
    Set {
        /// What it is called.
        name: String,
        /// Its datatype, in the schema grammar's own spelling.
        dtype: String,
        /// Its tag.
        #[arg(long)]
        tag: i32,
        /// The dialect it belongs to, where it is not the standard one.
        #[arg(long)]
        branch: Option<String>,
        /// What it is for.
        #[arg(long)]
        description: Option<String>,
    },
    /// Remove one field.
    Rm {
        /// A tag, an identifier, or a name.
        key: String,
    },
    /// Read an Ullink `CBlock` into the dictionary.
    Ingest {
        /// The `.cfb` file.
        path: PathBuf,
        /// The dialect its user-range tags belong to.
        #[arg(long)]
        branch: Option<String>,
        /// Fold each field into what is already there rather than replacing.
        #[arg(long)]
        merge: bool,
    },
    /// Fold another source into the dictionary.
    Sync {
        /// A folder holding another dictionary, or a `.cfb` file.
        source: PathBuf,
        /// The dialect a `.cfb`'s user-range tags belong to.
        ///
        /// A `CBlock` never names itself, so with none given the file's own
        /// stem names the dialect. A folder says nothing to this: its fields
        /// carry the branch they were written with.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Print the one row shape a whole capture lands in.
    Schema {
        /// Also carry the columns a capture with this row header supplies.
        ///
        /// The regex the text reader frames lines with. Its named captures
        /// become columns ahead of the FIX ones, typed by what their syntax
        /// can match.
        #[arg(long)]
        rowheader: Option<String>,
        /// What the root is called.
        #[arg(long, default_value = "FixMessage")]
        name: String,
        /// Write it here as JSON rather than printing it.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Check what the dictionary is wrong about.
    Check,
    /// Show what changed against another dictionary.
    Diff {
        /// The dictionary to compare against.
        against: PathBuf,
    },
}

/// Runs one dictionary command, answering what the process should exit with.
///
/// No command is the interactive shell rather than a usage error: every
/// command below is reachable from inside it, so a caller who names none is
/// asking for all of them.
pub fn run(root: &Path, annotate: bool, command: Option<&Command>) -> Result<ExitCode> {
    let mut store = registry::Store::open(root)?;
    let Some(command) = command else {
        return interactive(&mut store).map(|()| ExitCode::SUCCESS);
    };
    match command {
        Command::List { filter, limit } => {
            registry::list(&store, filter.as_deref(), *limit);
        }
        Command::Show { key } => registry::show(&store, key)?,
        Command::Set {
            name,
            dtype,
            tag,
            branch,
            description,
        } => {
            registry::put(
                &mut store,
                name,
                dtype,
                *tag,
                branch.as_deref(),
                description.as_deref(),
            )?;
            store.save()?;
        }
        Command::Rm { key } => {
            registry::remove(&mut store, key)?;
            store.save()?;
        }
        Command::Ingest {
            path,
            branch,
            merge,
        } => {
            ingest(&mut store, path, branch.as_deref(), *merge)?;
            store.save()?;
        }
        Command::Sync { source, branch } => {
            sync(&mut store, source, branch.as_deref())?;
            store.save()?;
        }
        Command::Schema {
            rowheader,
            name,
            out,
        } => {
            let field = schema::build(store.registry(), rowheader.as_deref(), name)?;
            schema::render(&field, out.as_deref())?;
        }
        Command::Check => {
            let report = quality::check(store.registry());
            if annotate {
                quality::annotate(&report);
            } else {
                quality::render(&report);
            }
            if report.failed() {
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::Diff { against } => {
            let other = registry::Store::open(against)?;
            let changes = diff::compare(other.registry(), store.registry());
            if annotate {
                diff::annotate(&changes);
            } else {
                diff::render(&changes);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Folds whatever one location holds into the dictionary.
///
/// The location decides which reader answers it, and nothing else does: a
/// folder is another dictionary, a `.cfb` is one counterparty's vocabulary,
/// and anything else is refused rather than guessed at. Both sources arrive
/// through the one fold, so a tag this dictionary lacks is added and one it
/// holds keeps every key only it declares - and because that fold is one
/// mutation, a source it refuses leaves the dictionary exactly as it was.
fn sync(store: &mut registry::Store, source: &Path, branch: Option<&str>) -> Result<()> {
    let mut progress = style::Progress::start(format!("reading {}", source.display()));
    progress.tick();
    let held = Holder::local(registry::located(source)?)?;
    let fields = match held.as_io().kind() {
        IOKind::Directory => {
            let other = FixRegistry::from_handle(held.as_io())?;
            other.iter().cloned().collect()
        }
        // A CBlock declares no media type of its own, so the name is the only
        // thing that says what the bytes are before they are read.
        IOKind::File
            if source
                .extension()
                .is_some_and(|held| held.eq_ignore_ascii_case("cfb")) =>
        {
            FixField::from_cfb_file(held.as_io(), branch)?
        }
        kind => {
            return Err(yggdryl::Error::InvalidRecord {
                path: source.display().to_string().into(),
                reason: format!(
                    "expected a folder holding a dictionary or a .cfb file, got {kind}"
                )
                .into(),
            });
        }
    };
    progress.tick();
    let (added, folded) = store.registry_mut().add_fields(fields)?;
    progress.finish(&format!("{added} added, {folded} merged"));
    Ok(())
}

/// Reads one `CBlock` into the dictionary.
///
/// Creating is the default and merging is asked for, because the two answer
/// different questions: a new counterparty is a new dictionary, and a revised
/// configuration is a change to one that exists. Merging folds each field
/// into what is already there, so a description a `CBlock` does not carry is
/// not lost by reading one that does not.
fn ingest(
    store: &mut registry::Store,
    path: &std::path::Path,
    branch: Option<&str>,
    merge: bool,
) -> Result<()> {
    let mut progress = style::Progress::start(format!("reading {}", path.display()));
    progress.tick();
    let path = registry::located(path)?;
    let held = Folder::new(
        path.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default(),
    )?;
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let handle = yggdryl::IOBase::child_by_path(&held, name)?;
    let dialect = branch.map(FixBranch::from_str).transpose()?;
    let (parsed, roots) = FixRegistry::from_cfb(&handle, dialect.as_ref())?;
    progress.tick();

    let (added, folded) = if merge {
        store.registry_mut().add_fields(parsed.iter().cloned())?
    } else {
        for field in &parsed {
            store.registry_mut().insert(field.clone())?;
        }
        (parsed.len(), 0)
    };
    progress.finish(&format!(
        "{added} added, {folded} merged, {} message root(s) read",
        roots.len()
    ));
    Ok(())
}

/// The interactive shell.
fn interactive(store: &mut registry::Store) -> Result<()> {
    style::heading("yggdryl fix");
    style::entry("dictionary", &store.root().display().to_string());
    style::entry("fields", &store.registry().len().to_string());
    style::note("tab completes · ↑ recalls · ctrl-d leaves · `help` lists commands");

    let commands: Vec<String> = [
        "list", "show", "set", "rm", "ingest", "schema", "check", "diff", "save", "help", "quit",
    ]
    .iter()
    .map(|held| (*held).to_owned())
    .collect();
    let mut history: Vec<String> = Vec::new();

    loop {
        let words = dictionary_words(store.registry());
        let completions = shell::Completions {
            commands: commands.clone(),
            words,
        };
        let marker = if store.changed() { "*" } else { "" };
        let prompt = format!("{}{marker} ", style::magenta("fix›"));
        let Some(line) = shell::read_line(&prompt, &mut history, &completions)? else {
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "quit" | "exit") {
            break;
        }
        if let Err(error) = dispatch(store, line) {
            style::bad(&error.to_string());
        }
    }

    if store.changed() {
        style::warn("leaving with unsaved changes - `save` writes them");
    }
    Ok(())
}

/// Every word the shell completes a field from.
fn dictionary_words(registry: &FixRegistry) -> Vec<String> {
    let mut words = Vec::with_capacity(registry.len() * 2);
    for field in registry {
        words.push(field.name().to_owned());
        if let Ok(Some(tag)) = field.as_fix().tag() {
            words.push(tag.to_string());
        }
    }
    words
}

/// Runs one shell line through the same commands the flags reach.
fn dispatch(store: &mut registry::Store, line: &str) -> Result<()> {
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return Ok(());
    };
    let rest: Vec<&str> = words.collect();
    match command {
        "help" => {
            style::heading("commands");
            for (name, about) in [
                ("list [filter]", "every field, filtered"),
                ("show <key>", "one field: identity, lineage, codes"),
                ("set <name> <type> <tag>", "create or replace a field"),
                ("rm <key>", "remove a field"),
                ("ingest <path.cfb>", "read a CBlock in"),
                ("schema", "the one row shape a capture lands in"),
                ("check", "what the dictionary is wrong about"),
                ("save", "write the dictionary back"),
                ("quit", "leave"),
            ] {
                style::entry(name, about);
            }
        }
        "list" => registry::list(store, rest.first().copied(), 40),
        "show" => {
            let Some(key) = rest.first() else {
                style::warn("show needs a tag or a name");
                return Ok(());
            };
            registry::show(store, key)?;
        }
        "set" => {
            let [name, dtype, tag, ..] = rest.as_slice() else {
                style::warn("set needs a name, a type and a tag");
                return Ok(());
            };
            let tag: i32 = tag.parse().map_err(|_| yggdryl::Error::InvalidRecord {
                path: (*tag).into(),
                reason: "expected a decimal tag".into(),
            })?;
            registry::put(store, name, dtype, tag, None, None)?;
        }
        "rm" => {
            let Some(key) = rest.first() else {
                style::warn("rm needs a tag or a name");
                return Ok(());
            };
            registry::remove(store, key)?;
        }
        "ingest" => {
            let Some(path) = rest.first() else {
                style::warn("ingest needs a path");
                return Ok(());
            };
            ingest(store, std::path::Path::new(path), None, true)?;
        }
        "schema" => {
            let field = schema::build(store.registry(), None, "FixMessage")?;
            schema::render(&field, rest.first().map(std::path::Path::new))?;
        }
        "check" => quality::render(&quality::check(store.registry())),
        "save" => {
            store.save()?;
            style::good("written");
        }
        _ => style::warn(&format!("no command {command:?} - `help` lists them")),
    }
    Ok(())
}
