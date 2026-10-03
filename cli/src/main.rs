//! `yggdryl` - the command line.
//!
//! One binary over the core's namespaces, each a subcommand that owns its own
//! verbs and its own state. There are three: [`fix`], the FIX dictionary
//! tool, [`xmla`], the XML for Analysis provider, and [`market`], the
//! market-data namespace whose `serve` verb is the book display. The top
//! level parses, dispatches, and prints a refusal and whatever the core
//! warned about that no command printed ([`warnings`]); every verb lives in
//! the namespace it belongs to, [`location`] is how every serving command
//! reads where its data is and [`timeout`] how long it lets a connection
//! stay quiet.
//!
//! | namespace | what it is |
//! | --- | --- |
//! | `fix` | a FIX dictionary: read it, change it, ingest a counterparty's configuration, check what came out - and with no verb, all of that interactively |
//! | `xmla` | the XML for Analysis provider: serve folders of record media as catalogs over HTTP |
//! | `market` | market data: `serve` tables of it as the book display - bid and ask candles, books and audits over HTTP - a FIX bridge capture folded in first |

/// Print to standard output, as `print!` does, ending quietly when the reader
/// has gone - see [`style::write_out`].
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::style::write_out(format_args!($($arg)*))
    };
}

/// Print a line to standard output, as `println!` does, ending quietly when
/// the reader has gone - see [`style::write_out`].
macro_rules! outln {
    () => {
        $crate::style::write_out(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::style::write_out(format_args!("{}\n", format_args!($($arg)*)))
    };
}

mod diff;
mod fix;
mod location;
mod market;
mod quality;
mod registry;
mod schema;
mod shell;
mod style;
mod timeout;
mod warnings;
mod xmla;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use yggdryl::DataType;

/// The yggdryl command line.
#[derive(Parser)]
#[command(name = "yggdryl", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Which namespace the tool was pointed at.
#[derive(Subcommand)]
enum Command {
    /// Manage a FIX dictionary; with no command, work interactively.
    Fix {
        /// Where the dictionary lives.
        #[arg(long, short, global = true, default_value = "config/fix")]
        root: PathBuf,

        /// Print findings as workflow annotations rather than as a table.
        ///
        /// Turned on by itself where `GITHUB_ACTIONS` is true (`true`, `yes`,
        /// `on` or `1`, in any case), so a workflow needs no extra flag;
        /// `false`, `no`, `off`, `0` or nothing at all leaves it off.
        #[arg(long, global = true)]
        annotate: bool,

        // Boxed: the FIX verbs carry far more than the other namespaces, and
        // the enum is one word wide without them inline.
        #[command(subcommand)]
        command: Option<Box<fix::Command>>,
    },
    /// Serve catalogs of record media over XML for Analysis.
    Xmla {
        #[command(subcommand)]
        command: xmla::Command,
    },
    /// Serve tables of market data as the book display over HTTP.
    Market {
        #[command(subcommand)]
        command: market::Command,
    },
}

/// The variable a workflow runner sets to say that it is one.
const RUNNER: &str = "GITHUB_ACTIONS";

/// Whether the environment says this is a workflow runner.
///
/// The variable is read as the boolean it is, through the value door every
/// boolean in the core is read through, so `true` and `1` say a runner and
/// `false` and `0` do not. Unset and blank are no runner, decided here so the
/// answer does not rest on how the door reads empty text.
///
/// # Errors
///
/// Returns the variable's name with what the boolean door refused of its
/// text, or of a value that is no text at all.
fn on_runner() -> Result<bool, String> {
    let Some(value) = std::env::var_os(RUNNER) else {
        return Ok(false);
    };
    let text = value
        .into_string()
        .map_err(|_| format!("{RUNNER}: expected text, got bytes that are not UTF-8"))?;
    if text.trim().is_empty() {
        return Ok(false);
    }
    DataType::Boolean
        .scalar(text.as_str())
        .map(|reading| reading.as_bool() == Some(true))
        .map_err(|error| format!("{RUNNER}: {error}"))
}

fn main() -> ExitCode {
    warnings::install();
    let cli = Cli::parse();
    let on_runner = match on_runner() {
        Ok(on_runner) => on_runner,
        Err(refusal) => {
            style::bad(&refusal);
            return ExitCode::FAILURE;
        }
    };
    let annotate = on_runner || matches!(cli.command, Command::Fix { annotate: true, .. });
    let outcome = match &cli.command {
        Command::Fix { root, command, .. } => fix::run(root, annotate, command.as_deref()),
        Command::Xmla { command } => xmla::run(command),
        Command::Market { command } => market::run(command),
    };
    // What the core warned about and no command printed yet - a refusal
    // included, so it still shows what was read before it.
    warnings::report(annotate);
    match outcome {
        Ok(code) => code,
        Err(error) => {
            style::bad(&error.to_string());
            ExitCode::FAILURE
        }
    }
}
