//! `ygg` - the yggdryl command line.
//!
//! One binary over the core's namespaces, each a subcommand that owns its own
//! verbs and its own state. Today there is one: [`fix`], the FIX dictionary
//! tool. The top level parses, dispatches, and prints a refusal; every verb
//! lives in the namespace it belongs to.
//!
//! | namespace | what it is |
//! | --- | --- |
//! | `fix` | a FIX dictionary: read it, change it, ingest a counterparty's configuration, check what came out - and with no verb, all of that interactively |

mod diff;
mod fix;
mod quality;
mod registry;
mod schema;
mod shell;
mod style;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// The yggdryl command line.
#[derive(Parser)]
#[command(name = "ygg", version, about, long_about = None)]
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
        /// Turned on by itself where the environment says it is a GitHub
        /// Actions runner, so a workflow needs no extra flag.
        #[arg(long, global = true)]
        annotate: bool,

        #[command(subcommand)]
        command: Option<fix::Command>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match &cli.command {
        Command::Fix {
            root,
            annotate,
            command,
        } => fix::run(
            root,
            *annotate || std::env::var_os("GITHUB_ACTIONS").is_some(),
            command.as_ref(),
        ),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            style::bad(&error.to_string());
            ExitCode::FAILURE
        }
    }
}
