//! `ygg` - manage a yggdryl FIX dictionary from a terminal.
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
//! | `list` | every field, filtered |
//! | `show` | one field: identity, lineage, codes |
//! | `set` | create or replace a field |
//! | `rm` | remove a field |
//! | `ingest` | read a `.cfb` into the dictionary, creating or merging |
//! | `schema` | the one row shape a whole capture lands in |
//! | `check` | what the dictionary is wrong about |
//! | `diff` | what changed against another dictionary |
//! | `shell` | all of the above, interactively, with completion |

mod diff;
mod quality;
mod registry;
mod schema;
mod shell;
mod style;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use yggdryl::holder::local::Folder;
use yggdryl::{FixBranch, FixRegistry, Result};

/// Manage a yggdryl FIX dictionary.
#[derive(Parser)]
#[command(name = "ygg", version, about, long_about = None)]
struct Cli {
    /// Where the dictionary lives.
    #[arg(long, short, global = true, default_value = "config/fix")]
    root: PathBuf,

    /// Print findings as workflow annotations rather than as a table.
    ///
    /// Turned on by itself where the environment says it is a GitHub
    /// Actions runner, so a workflow needs no extra flag.
    #[arg(long, global = true)]
    annotate: bool,

    #[command(subcommand)]
    command: Command,
}

/// What the tool was asked to do.
#[derive(Subcommand)]
enum Command {
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
    /// Work interactively, with completion.
    Shell,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let annotate = cli.annotate || std::env::var_os("GITHUB_ACTIONS").is_some();
    match run(&cli, annotate) {
        Ok(code) => code,
        Err(error) => {
            style::bad(&error.to_string());
            ExitCode::FAILURE
        }
    }
}

/// Runs one command, answering what the process should exit with.
fn run(cli: &Cli, annotate: bool) -> Result<ExitCode> {
    let mut store = registry::Store::open(&cli.root)?;
    match &cli.command {
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
        Command::Shell => interactive(&mut store)?,
    }
    Ok(ExitCode::SUCCESS)
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

    let mut added = 0_usize;
    let mut folded = 0_usize;
    for field in &parsed {
        let tag = field.as_fix().tag()?.unwrap_or_default();
        if merge && store.registry().get_field_by_tag(tag).is_some() {
            store.registry_mut().update(field.clone())?;
            folded += 1;
        } else {
            store.registry_mut().insert(field.clone())?;
            added += 1;
        }
    }
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
