//! Categorical FIX CRUD, ingestion, inspection, and one shared shell dispatcher.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};
use yggdryl::holder::Holder;
use yggdryl::{
    DataType, Field, FixCategory, FixDirection, FixMerge, FixRegistry, IOKind, Result, Url,
};

use crate::{diff, quality, registry, schema, shell, style, warnings};

/// What the dictionary tool was asked to do.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  yggdryl fix fields list Party\n  yggdryl fix fields read 453 --json\n  yggdryl fix components create Party 'struct<PartyID: utf8>'\n  yggdryl fix groups create Parties 'serie<Party: struct<PartyID: utf8> not null>' --counter 453 --component Party\n  yggdryl fix components create --input Order.json\n  yggdryl fix codesets write msgdirectioncodeset --codes '[{\"value\":\"R\",\"name\":\"Receive\"},{\"value\":\"S\",\"name\":\"Send\"}]'\n  yggdryl fix fields update MsgDirection utf8 --tag 385 --codes msgdirectioncodeset --directions '[{\"code\":\"S\",\"patterns\":[\"(?i)^TX\\\\b\"]},{\"code\":\"R\",\"patterns\":[\"(?i)^RX\\\\b\"]}]'\n\nEach category supports list, read, create, update, and delete.\nUse <category> <operation> --help for inputs and examples.\nA field reads its values by a named code set the dictionary holds: --codes names one, and codesets list/read/write/delete states its members.\nTag 385's direction rules live in FIX:directions metadata; --directions accepts that JSON document, one entry per code of the set, and an empty list removes it so the crate's defaults read again."
)]
pub enum Command {
    /// Tagged scalar fields, including int32 repeating-group counters.
    Fields {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Named structs: one occurrence of a group, or a message when it
    /// carries a FIX message type.
    Components {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Repeating series with a separate counter tag and component.
    Groups {
        #[command(subcommand)]
        command: CategoryCommand,
    },
    /// Named code sets: the vocabularies fields read their values by.
    Codesets {
        #[command(subcommand)]
        command: CodesetCommand,
    },
    /// Fold Ullink `CBlock`s into the dictionary: files, folders or globs of them.
    ///
    /// Every file is parsed side by side and folded into one staged
    /// dictionary in ascending URL order, which is then committed once,
    /// writing only the documents that moved. A tag declared at another
    /// precision than the datatype held - text, a number of another kind or
    /// width, a date beside a datetime - folds under the held one; one that
    /// contradicts it - a flag to one counterparty, an integer to another -
    /// is passed over and named, and the rest still folds. What a file states
    /// in a way the reader cannot keep is named with its line and column and
    /// what the reader did instead.
    #[command(
        after_help = "Examples:\n  yggdryl fix ingest cblocks/venue.cfb --dialect venue\n  yggdryl fix ingest 'cblocks/*.cfb'\n  yggdryl fix ingest 'cblocks/**/*.cfb' --annotate\n  yggdryl fix ingest cblocks/a.cfb cblocks/b.cfb\n  yggdryl fix ingest cblocks/\n\nA folder folds the .cfb files directly inside it. Quote a glob to have it walked here - `*` stays inside one name, `**` spans folders - or let the shell expand it; either way every file folds in one staged dictionary and one commit.\nWithout --dialect each file's own stem names its dialect (MSFIX44.cfb stamps msfix44).\nWhere two files disagree about one tag, the first in URL order is held; a coarser datatype folds under the held one (restated), and only a contradiction is passed over.\nWhat a file states in a way the reader cannot keep is named with its line and column and what the reader did instead; a file that cannot be folded at all is left out and named, and every other file still folds.\n--annotate prints every one of them as a workflow warning."
    )]
    Ingest {
        /// `.cfb` files, folders of them, or glob patterns such as
        /// `cblocks/*.cfb`.
        ///
        /// A path naming nothing, and paths holding no file at all, are
        /// refused, so a mistyped location never reads as a silent success.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// The dictionary name stamped on every definition the files produce.
        ///
        /// Membership (`FIX:branches`) is provenance a listing filters on; it
        /// never decides how a tag or a name resolves. With none given, each
        /// file's own stem names its dialect.
        #[arg(long)]
        dialect: Option<String>,
    },
    /// Fold another dictionary folder into this one.
    Sync {
        /// A folder holding another dictionary.
        ///
        /// Its fields carry the membership they were written with, which the
        /// fold unions onto what this dictionary holds. A `.cfb` is
        /// `yggdryl fix ingest`'s.
        source: PathBuf,
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

/// Operations over one named code set.
///
/// A code set is a vocabulary rather than a definition: it holds no tag, no
/// datatype and no reference, so it has its own verbs rather than a category
/// of the definition ones. `merge` is the difference that matters - two
/// sources state one set, and folding them keeps every spelling either named.
#[derive(Subcommand)]
#[command(
    after_help = "Examples:\n  yggdryl fix codesets list side\n  yggdryl fix codesets read sidecodeset --json\n  yggdryl fix codesets write sidecodeset --codes '[{\"value\":\"1\",\"name\":\"Buy\"},{\"value\":\"2\",\"name\":\"Sell\"}]'\n  yggdryl fix codesets write sidecodeset --merge --codes '[{\"value\":\"7\",\"name\":\"Undisclosed\"}]'\n  yggdryl fix fields update Side utf8 --tag 54 --codes sidecodeset\n\nwrite replaces the set; --merge folds by wire value instead, keeping every name and alias either side declared.\ndelete refuses a set a field still reads by."
)]
pub enum CodesetCommand {
    /// List the code sets held, with how many fields read by each.
    List {
        /// Match part of a name, ignoring case.
        filter: Option<String>,
        /// Maximum number of rows printed.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Read one code set: every member, or the document a store writes.
    Read {
        /// The set's name.
        name: String,
        /// Emit the JSON document a store writes under `codesets/<name>.json`.
        #[arg(long)]
        json: bool,
    },
    /// State a code set's members, replacing or folding into what it held.
    #[command(
        after_help = "Examples:\n  yggdryl fix codesets write sidecodeset --codes '[{\"value\":\"1\",\"name\":\"Buy\"},{\"value\":\"2\",\"name\":\"Sell\"}]'\n  yggdryl fix codesets write sidecodeset --merge --codes '[{\"value\":\"7\",\"name\":\"Undisclosed\",\"aliases\":[\"Anon\"]}]'\n\n--codes takes the whole set as compact JSON, value before name; aliases, doc and group are optional on each code.\nWithout --merge the set is replaced; with it the codes fold in by wire value, the reading the dictionary already holds winning a shared one and every spelling either side declared kept as an alias.\nAn empty list removes the set, which is refused while a field still reads by it."
    )]
    Write {
        /// The set's name.
        name: String,
        /// Compact code JSON, with value before name: [{"value":"1","name":"Buy"}].
        #[arg(long)]
        codes: String,
        /// Fold into what the set already holds rather than replacing it.
        #[arg(long)]
        merge: bool,
    },
    /// Delete a code set; absence or a field still reading by it is an error.
    Delete {
        /// The set's name.
        name: String,
    },
}

/// Operations common to each explicitly selected category.
#[derive(Subcommand)]
#[command(
    after_help = "Create refuses an existing definition; update replaces a definition and refuses absence.\nRead --json emits the native Field document accepted by create/update --input.\nDelete refuses definitions still referenced by other definitions.\nThe registry is one namespace: a key resolves the same way whatever dictionaries a definition belongs to. --dialect on create/update records membership (FIX:branches); on list it filters by it."
)]
pub enum CategoryCommand {
    /// List definitions, optionally filtered by name/tag and dictionary membership.
    List {
        /// Match part of a name or decimal tag, ignoring case.
        filter: Option<String>,
        /// Only definitions whose FIX:branches membership names this dictionary.
        #[arg(long)]
        dialect: Option<String>,
        /// Maximum number of rows printed.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Read one definition, its references and the code set it reads by.
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
    after_help = "Examples:\n  yggdryl fix fields create NoPartyIDs int32 --tag 453\n  yggdryl fix fields create --input Side.json\n  yggdryl fix components create Party 'struct<PartyID: utf8>'\n  yggdryl fix components create Order 'struct<ClOrdID: utf8>' --msgtype D\n  yggdryl fix codesets write msgdirectioncodeset --codes '[{\"value\":\"R\",\"name\":\"Receive\"},{\"value\":\"S\",\"name\":\"Send\"}]'\n  yggdryl fix fields update MsgDirection utf8 --tag 385 --codes msgdirectioncodeset --directions '[{\"code\":\"S\",\"patterns\":[\"(?i)^TX\\\\b\"]},{\"code\":\"R\",\"patterns\":[\"(?i)^RX\\\\b\"]}]'\n\nQuote datatype expressions containing spaces or shell metacharacters.\n--input accepts one complete native Field JSON document, including metadata and children.\nA field names the code set it reads its values by; --codes takes that name, and `yggdryl fix codesets write` states its members.\nTag 385's direction rules belong to FIX:directions metadata. --directions accepts compact JSON with one entry per code of the set, each pattern a regex read against the prose in front of a payload, for example [{\"code\":\"S\",\"patterns\":[\"(?i)^TX\\\\b\"]}]; an empty list removes the property so the crate's defaults read again."
)]
pub struct DefinitionArgs {
    /// Canonical definition name, preserving its spelling.
    #[arg(required_unless_present = "input")]
    name: Option<String>,
    /// Core datatype expression: int32, utf8, struct<...>, serie<...>.
    #[arg(required_unless_present = "input")]
    dtype: Option<String>,
    /// Read one native Field JSON document; replaces positional inputs and flags.
    #[arg(long, conflicts_with_all = ["name", "dtype", "tag", "dialect", "description", "counter", "component", "codes", "directions", "identifiers", "msgtype", "required"])]
    input: Option<PathBuf>,
    /// Numeric tag for a scalar field, including a group counter.
    #[arg(long)]
    tag: Option<i32>,
    /// A dictionary this definition belongs to (FIX:branches); repeat for several.
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
    /// The code set this field reads its values by, by name (for example, sidecodeset).
    #[arg(long)]
    codes: Option<String>,
    /// Direction rules JSON for tag 385: [{"code":"S","patterns":["(?i)^TX\\b"]}].
    #[arg(long)]
    directions: Option<String>,
    /// Direct scalar identifier member; repeat for several, in any input order.
    #[arg(long)]
    identifiers: Vec<String>,
    /// FIX message type making a component a message (for example, D).
    #[arg(long)]
    msgtype: Option<String>,
    /// Make the value required. A message is always required.
    #[arg(long)]
    required: bool,
}

impl DefinitionArgs {
    fn field(&self) -> Result<Field> {
        if let Some(path) = &self.input {
            return yggdryl::from_fix_document(yggdryl::from_json_scalar(std::fs::read(path)?)?);
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
        field.set_nullable(!self.required && self.msgtype.is_none());
        if let Some(name) = &self.codes {
            field.as_fix_mut().set_codeset(name)?;
        }
        if let Some(document) = &self.directions {
            field.update_metadata([("FIX:directions", document.clone())])?;
            let rules = field
                .as_fix()
                .directions()
                .map(|rule| rule.map(FixDirection::from))
                .collect::<Result<Vec<_>>>()?;
            field.as_fix_mut().set_directions(&rules)?;
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
        view.set_identifiers(&self.identifiers)?;
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
        let report = store.save()?;
        style::good(&registry::committed(&report));
    }
    Ok(outcome)
}

fn execute(store: &mut registry::Store, annotate: bool, command: &Command) -> Result<ExitCode> {
    match command {
        Command::Fields { command } => category(store, FixCategory::Fields, command)?,
        Command::Components { command } => category(store, FixCategory::Components, command)?,
        Command::Groups { command } => category(store, FixCategory::Groups, command)?,
        Command::Codesets { command } => codesets(store, command)?,
        Command::Ingest { paths, dialect } => {
            ingest(store, paths, dialect.as_deref(), annotate)?;
        }
        Command::Sync { source } => {
            sync(store, source, annotate)?;
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
        CategoryCommand::Create(args) => registry::create(store, category, args.field()?),
        CategoryCommand::Update(args) => registry::update(store, category, args.field()?),
        CategoryCommand::Delete { key } => registry::delete(store, category, key),
    }
}

fn codesets(store: &mut registry::Store, command: &CodesetCommand) -> Result<()> {
    match command {
        CodesetCommand::List { filter, limit } => {
            registry::list_codesets(store, filter.as_deref(), *limit);
            Ok(())
        }
        CodesetCommand::Read { name, json } => registry::read_codeset(store, name, *json),
        CodesetCommand::Write { name, codes, merge } => {
            registry::write_codeset(store, name, codes, *merge)
        }
        CodesetCommand::Delete { name } => registry::delete_codeset(store, name),
    }
}

/// Folds another dictionary folder into this one.
///
/// A folder is another dictionary and nothing else is: a `.cfb` is one
/// counterparty's vocabulary, which [`ingest`] reads, and anything else is
/// refused rather than guessed at. The fold is the core's one: a tag this
/// dictionary lacks is added, one it holds keeps every key only it declares,
/// the membership the other stamps is unioned onto it, and what it declares
/// otherwise than this dictionary does is passed over and named.
fn sync(store: &mut registry::Store, source: &Path, annotate: bool) -> Result<()> {
    let mut progress = style::Progress::start(format!("reading {}", source.display()));
    progress.tick();
    let held = Holder::local(registry::located(source)?)?;
    let kind = held.as_io().kind();
    if kind != IOKind::Directory {
        return Err(yggdryl::Error::InvalidRecord {
            path: source.display().to_string().into(),
            reason: format!(
                "expected a folder holding a dictionary, got {kind}; a .cfb is `yggdryl fix ingest`'s"
            )
            .into(),
        });
    }
    let other = FixRegistry::from_handle(held.as_io())?;
    progress.tick();
    let merge = store.registry_mut().merge_with(&other)?;
    progress.finish(&folded(&merge, "dictionary"));
    warnings::report(annotate);
    passed_over(&merge, annotate);
    Ok(())
}

/// Folds every `CBlock` the paths hold into the dictionary, in one fold.
///
/// Each path is a file, a folder or a glob, handed to the core as the holder
/// it is and nothing more: a glob the shell left alone is walked by the core,
/// its fixed prefix descended, `*` inside one name, `**` across folders and
/// private entries never matched; a folder gives up the `.cfb` files
/// directly inside it, and a file is itself; a shell that expanded a glob
/// hands over the files it matched. Every path arrives in one call, which
/// parses the files side by side, folds them in ascending URL order into one
/// staged dictionary and resolves once. So a hundred files cost one load,
/// one resolution and one commit, and where two of them disagree about one
/// tag the first in URL order is held and the other is named. A file the
/// core cannot fold is left out and named, and every other file still folds.
///
/// A path naming nothing is refused, and so is a call whose paths hold no
/// file at all: a mistyped location is a mistake, never a silent success. A
/// location beside others that holds no file is named in a warning, and a
/// call whose every file is left out fails once each is named.
fn ingest(
    store: &mut registry::Store,
    paths: &[PathBuf],
    dialect: Option<&str>,
    annotate: bool,
) -> Result<()> {
    let mut progress = style::Progress::start(match paths {
        [path] => format!("reading {}", path.display()),
        paths => format!("reading {} locations", paths.len()),
    });
    progress.tick();
    let mut locations = Vec::with_capacity(paths.len());
    for path in paths {
        let held = Holder::local(registry::located(path)?)?;
        let glob = held.as_io().url().is_some_and(Url::is_glob);
        if !glob && held.as_io().kind() == IOKind::Unknown {
            return Err(yggdryl::Error::Absent {
                expected: ".cfb file, a folder of them or a glob",
                path: path.display().to_string().into(),
            });
        }
        locations.push(held);
    }
    progress.tick();
    let merge = store.registry_mut().add_cfb_files(&locations, dialect)?;
    if merge.sources == 0 && merge.failed.is_empty() {
        return Err(yggdryl::Error::Absent {
            expected: ".cfb file the locations hold",
            path: paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
        });
    }
    progress.finish(&folded(&merge, "file"));
    warnings::report(annotate);
    passed_over(&merge, annotate);
    left_out(&merge, annotate);
    // Every file left out folded nothing: each is named above, and the
    // command says it failed rather than committing an unchanged store.
    if merge.sources == 0 {
        return Err(yggdryl::Error::Absent {
            expected: ".cfb file that folds",
            path: paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" ")
                .into(),
        });
    }
    Ok(())
}

/// What one fold did, in one line: the merged fields restated under the
/// datatype held, where any were, beside the count they are among.
fn folded(merge: &FixMerge, source: &str) -> String {
    let restated = if merge.restated > 0 {
        format!(" ({} restated)", merge.restated)
    } else {
        String::new()
    };
    let failed = if merge.failed.is_empty() {
        String::new()
    } else {
        format!(", {} {source}(s) left out", merge.failed.len())
    };
    format!(
        "{} {source}(s): {} added, {} merged{restated}, {} passed over{failed}",
        merge.sources,
        merge.added,
        merge.merged,
        merge.dropped.len()
    )
}

/// Names every source a fold left out whole: one line each, or one workflow
/// warning each under `--annotate`. A file left out contributed nothing,
/// while every other file still folded.
fn left_out(merge: &FixMerge, annotate: bool) {
    if merge.failed.is_empty() {
        return;
    }
    if annotate {
        for failure in &merge.failed {
            outln!(
                "::warning title=fix left out::{}",
                style::annotation(&failure.to_string())
            );
        }
        return;
    }
    style::warn(&format!(
        "{} file(s) left out: each contributed nothing, and every other file still folded",
        merge.failed.len()
    ));
    for failure in &merge.failed {
        let source = failure
            .source
            .as_deref()
            .and_then(|source| Url::from_str(source).ok())
            .and_then(|url| url.file_name().map(str::to_owned));
        let mut line: Vec<String> = source.into_iter().collect();
        line.push(failure.reason.to_string());
        style::note(&line.join(" "));
    }
}

/// Names every declaration a fold passed over: one line each, or one
/// workflow warning each under `--annotate`.
fn passed_over(merge: &FixMerge, annotate: bool) {
    if merge.dropped.is_empty() {
        return;
    }
    if annotate {
        for drop in &merge.dropped {
            outln!(
                "::warning title=fix passed over::{}",
                style::annotation(&drop.to_string())
            );
        }
        return;
    }
    style::warn(&format!(
        "{} declaration(s) passed over: the dictionary already declares them otherwise",
        merge.dropped.len()
    ));
    for drop in &merge.dropped {
        let source = drop
            .source
            .as_deref()
            .and_then(|source| Url::from_str(source).ok())
            .and_then(|url| url.file_name().map(str::to_owned));
        let dialects = drop
            .incoming
            .as_fix()
            .branches()
            .collect::<Vec<_>>()
            .join(", ");
        let mut line: Vec<String> = source.into_iter().collect();
        if !dialects.is_empty() {
            line.push(format!("[{dialects}]"));
        }
        line.push(drop.reason.to_string());
        style::note(&line.join(" "));
    }
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
        "components",
        "groups",
        "codesets",
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
        let outcome = dispatch(store, line);
        warnings::report(false);
        if let Err(error) = outcome {
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
        "--directions",
        "--identifiers",
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
        let report = store.save()?;
        style::good(&registry::committed(&report));
        return Ok(());
    }
    if line == "help" {
        ShellCommand::command().print_long_help()?;
        outln!();
        style::note(
            "save writes pending changes; quit leaves. Category commands use the same flags as yggdryl fix.",
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
