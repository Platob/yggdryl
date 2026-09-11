//! Categorical FIX CRUD, ingestion, inspection, and one shared shell dispatcher.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};
use yggdryl::holder::Holder;
use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixCategory, FixCode, FixRegistry, IOKind, Result};

use crate::{diff, quality, registry, schema, shell, style};

/// What the dictionary tool was asked to do.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  ygg fix fields list Party\n  ygg fix fields read 453 --json\n  ygg fix components create Party 'struct<PartyID: utf8>'\n  ygg fix groups create Parties 'list<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party\n  ygg fix messages create --input Order.json\n\nEach category supports list, read, create, update, and delete.\nUse <category> <operation> --help for inputs and examples.\nField enums live in fix:codes metadata; --codes accepts that JSON document."
)]
pub enum Command {
    /// Tagged scalar fields, including int32 repeating-group counters.
    Fields {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Message definitions: named structs with a FIX message type.
    Messages {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Reusable named structs, including one occurrence of a group.
    Components {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Repeating lists with a separate counter tag and component.
    Groups {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Read an Ullink `CBlock` into the dictionary.
    Ingest {
        /// The `.cfb` file.
        path: PathBuf,
        /// The dictionary name stamped on every definition the file produces.
        ///
        /// Membership (`fix:branches`) is provenance a listing filters on; it
        /// never decides how a tag or a name resolves.
        #[arg(long)]
        dialect: Option<String>,
        /// Fold each field into what is already there rather than replacing.
        #[arg(long)]
        merge: bool,
    },
    /// Fold another source into the dictionary.
    Sync {
        /// A folder holding another dictionary, or a `.cfb` file.
        source: PathBuf,
        /// The dictionary name stamped on every definition a `.cfb` produces.
        ///
        /// A `CBlock` never names itself, so with none given the file's own
        /// stem names the dialect. A folder says nothing to this: its fields
        /// carry the membership they were written with.
        #[arg(long)]
        dialect: Option<String>,
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

/// Operations common to each explicitly selected category.
#[derive(Subcommand)]
#[command(
    after_help = "Create refuses an existing definition; update replaces a definition and refuses absence.\nRead --json emits the native Field document accepted by create/update --input.\nDelete refuses definitions still referenced by other definitions.\nThe registry is one namespace: a key resolves the same way whatever dictionaries a definition belongs to. --dialect on create/update records membership (fix:branches); on list it filters by it."
)]
pub enum CategoryCommand {
    /// List definitions, optionally filtered by name/tag and dictionary membership.
    List {
        /// Match part of a name or decimal tag, ignoring case.
        filter: Option<String>,
        /// Only definitions whose fix:branches membership names this dictionary.
        #[arg(long)]
        dialect: Option<String>,
        /// Maximum number of rows printed.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Read one definition, its references, lineage, and inline enum codes.
    Read {
        /// Definition name; fields also accept a tag.
        key: String,
        /// Emit a native Field JSON document for create/update --input.
        #[arg(long)]
        json: bool,
    },
    /// Create a definition; an existing name or identity is an error.
    Create(DefinitionArgs),
    /// Replace an existing definition in full, preserving its identity.
    ///
    /// Omitted metadata is removed. For metadata-only edits, read --json,
    /// edit that document, then update --input; name and field tag remain
    /// the same.
    Update(DefinitionArgs),
    /// Delete a definition; absence or a live reference is an error.
    Delete {
        /// Definition name; fields also accept a tag.
        key: String,
    },
}

/// Native Field intake; category semantics remain in the core registry.
#[derive(Args)]
#[command(
    after_help = "Examples:\n  ygg fix fields create NoPartyIDs int32 --tag 453\n  ygg fix fields create --input Side.json\n  ygg fix components create Party 'struct<PartyID: utf8>'\n  ygg fix messages create Order 'struct<ClOrdID: utf8>' --msgtype D\n\nQuote datatype expressions containing spaces or shell metacharacters.\n--input accepts one complete native Field JSON document, including metadata and children.\nField enum records belong to fix:codes metadata. --codes accepts compact JSON with value before name, for example {\"codes\":[{\"value\":\"1\",\"name\":\"Buy\"}]}."
)]
pub struct DefinitionArgs {
    /// Canonical definition name, preserving its spelling.
    #[arg(required_unless_present = "input")]
    name: Option<String>,
    /// Core datatype expression: int32, utf8, struct<...>, list<...>.
    #[arg(required_unless_present = "input")]
    dtype: Option<String>,
    /// Read one native Field JSON document; replaces positional inputs and flags.
    #[arg(long, conflicts_with_all = ["name", "dtype", "tag", "dialect", "description", "counter", "component", "codes", "msgtype", "required"])]
    input: Option<PathBuf>,
    /// Numeric tag for a scalar field, including a group counter.
    #[arg(long)]
    tag: Option<i32>,
    /// A dictionary this definition belongs to (fix:branches); repeat for several.
    #[arg(long)]
    dialect: Vec<String>,
    /// Definition's purpose.
    #[arg(long)]
    description: Option<String>,
    /// Counter field's numeric tag for a group (for example, 453).
    #[arg(long)]
    counter: Option<i32>,
    /// Existing component defining one occurrence of this group.
    #[arg(long)]
    component: Option<String>,
    /// Compact inline enum JSON, with value before name: {"codes":[{"value":"1","name":"Buy"}]}.
    #[arg(long)]
    codes: Option<String>,
    /// FIX message type for a message definition (for example, D).
    #[arg(long)]
    msgtype: Option<String>,
    /// Make the value required. Messages are always required.
    #[arg(long)]
    required: bool,
}

impl DefinitionArgs {
    fn field(&self, category: FixCategory) -> Result<Field> {
        if let Some(path) = &self.input {
            return Field::from_json_bytes(&std::fs::read(path)?);
        }
        let name = self.name.as_deref().ok_or_else(|| yggdryl::Error::Absent {
            expected: "a definition name or --input",
            path: "fix".into(),
        })?;
        let dtype = self
            .dtype
            .as_deref()
            .ok_or_else(|| yggdryl::Error::Absent {
                expected: "a datatype or --input",
                path: name.into(),
            })?;
        let mut field = DataType::from_str(dtype)?.nullable_field(name);
        field.set_nullable(!self.required && category != FixCategory::Messages);
        if let Some(document) = &self.codes {
            field.set_metadata([("fix:codes", document.clone())])?;
            let codes = field
                .as_fix()
                .codes()
                .map(|code| code.map(FixCode::from))
                .collect::<Result<Vec<_>>>()?;
            field.as_fix_mut().set_codes(&codes)?;
        }
        let mut view = field.as_fix_mut();
        view.set_branches(&self.dialect)?;
        if let Some(tag) = self.tag {
            view.set_tag(tag)?;
        }
        if let Some(value) = &self.description {
            view.set_description(value)?;
        }
        if let Some(value) = self.counter {
            view.set_counter(value)?;
        }
        if let Some(value) = &self.component {
            view.set_component(value)?;
        }
        if let Some(value) = &self.msgtype {
            view.set_msgtype(value)?;
        }
        Ok(field)
    }
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
    let outcome = execute(&mut store, annotate, command)?;
    if store.changed() {
        store.save()?;
    }
    Ok(outcome)
}

fn execute(store: &mut registry::Store, annotate: bool, command: &Command) -> Result<ExitCode> {
    match command {
        Command::Fields { command } => category(store, FixCategory::Fields, command)?,
        Command::Messages { command } => category(store, FixCategory::Messages, command)?,
        Command::Components { command } => category(store, FixCategory::Components, command)?,
        Command::Groups { command } => category(store, FixCategory::Groups, command)?,
        Command::Ingest {
            path,
            dialect,
            merge,
        } => {
            ingest(store, path, dialect.as_deref(), *merge)?;
        }
        Command::Sync { source, dialect } => {
            sync(store, source, dialect.as_deref())?;
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

fn category(
    store: &mut registry::Store,
    category: FixCategory,
    command: &CategoryCommand,
) -> Result<()> {
    match command {
        CategoryCommand::List {
            filter,
            dialect,
            limit,
        } => {
            registry::list(
                store,
                category,
                filter.as_deref(),
                dialect.as_deref(),
                *limit,
            );
            Ok(())
        }
        CategoryCommand::Read { key, json } => registry::read(store, category, key, *json),
        CategoryCommand::Create(args) => registry::create(store, category, args.field(category)?),
        CategoryCommand::Update(args) => registry::update(store, category, args.field(category)?),
        CategoryCommand::Delete { key } => registry::delete(store, category, key),
    }
}

/// Folds whatever one location holds into the dictionary.
///
/// The location decides which reader answers it, and nothing else does: a
/// folder is another dictionary, a `.cfb` is one counterparty's vocabulary,
/// and anything else is refused rather than guessed at. Both sources arrive
/// through the one fold, so a tag this dictionary lacks is added, one it holds
/// keeps every key only it declares, and the membership either source stamps
/// is unioned onto them - and because that fold is one mutation, a source it
/// refuses leaves the dictionary exactly as it was.
fn sync(store: &mut registry::Store, source: &Path, dialect: Option<&str>) -> Result<()> {
    let mut progress = style::Progress::start(format!("reading {}", source.display()));
    progress.tick();
    let held = Holder::local(registry::located(source)?)?;
    let (added, folded) = match held.as_io().kind() {
        IOKind::Directory => {
            let other = FixRegistry::from_handle(held.as_io())?;
            progress.tick();
            store.registry_mut().merge_with(&other)?
        }
        // A CBlock declares no media type of its own, so the name is the only
        // thing that says what the bytes are before they are read. Read whole
        // rather than for its vocabulary alone, so its groups, components and
        // messages arrive with its fields, each stamped with the dialect.
        IOKind::File
            if source
                .extension()
                .is_some_and(|held| held.eq_ignore_ascii_case("cfb")) =>
        {
            progress.tick();
            store.registry_mut().add_cfb_file(held.as_io(), dialect)?
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
    dialect: Option<&str>,
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
    let (parsed, roots) = FixRegistry::from_cfb_file(&handle, dialect)?;
    progress.tick();

    let (added, folded) = if merge {
        store.registry_mut().merge_with(&parsed)?
    } else {
        let mut next = store.registry().clone();
        let mut added = 0;
        let mut replaced = 0;
        for category in [
            FixCategory::Fields,
            FixCategory::Components,
            FixCategory::Groups,
            FixCategory::Messages,
        ] {
            for field in parsed.definitions(category) {
                if next.insert_definition(category, field.clone())?.is_some() {
                    replaced += 1;
                } else {
                    added += 1;
                }
            }
        }
        *store.registry_mut() = next;
        (added, replaced)
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
    for category in FixCategory::ALL {
        style::entry(
            category.as_str(),
            &store.registry().definitions(category).count().to_string(),
        );
    }
    style::note("tab completes · ↑ recalls · ctrl-d leaves · `help` lists commands");

    let commands: Vec<String> = [
        "fields",
        "messages",
        "components",
        "groups",
        "ingest",
        "sync",
        "schema",
        "check",
        "diff",
        "save",
        "help",
        "quit",
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
    let mut words: Vec<String> = [
        "list",
        "read",
        "create",
        "update",
        "delete",
        "--help",
        "--input",
        "--json",
        "--dialect",
        "--tag",
        "--counter",
        "--component",
        "--codes",
        "--msgtype",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for category in FixCategory::ALL {
        for field in registry.definitions(category) {
            words.push(field.name().to_owned());
            if let Ok(Some(tag)) = field.as_fix().tag() {
                words.push(tag.to_string());
            }
        }
    }
    words.sort();
    words.dedup();
    words
}

#[derive(Parser)]
#[command(name = "fix")]
struct ShellCommand {
    #[command(subcommand)]
    command: Command,
}

/// Runs one shell line through the same commands the flags reach.
fn dispatch(store: &mut registry::Store, line: &str) -> Result<()> {
    if line == "save" {
        store.save()?;
        style::good("written");
        return Ok(());
    }
    if line == "help" {
        ShellCommand::command().print_long_help()?;
        println!();
        style::note(
            "save writes pending changes; quit leaves. Category commands use the same flags as ygg fix.",
        );
        return Ok(());
    }
    let words = shlex::split(line).ok_or_else(|| yggdryl::Error::InvalidRecord {
        path: "fix shell".into(),
        reason: "expected balanced quotes and escapes".into(),
    })?;
    match ShellCommand::try_parse_from(std::iter::once("fix".to_owned()).chain(words)) {
        Ok(command) => {
            execute(store, false, &command.command)?;
        }
        Err(error) => error.print()?,
    }
    Ok(())
}
