//! What the terminal looks like: colour, rules, tables, and a spinner.
//!
//! Written here rather than pulled in, because what a dictionary tool draws
//! is a handful of shapes - a heading, a table, a key/value block, a progress
//! line - and each of them is a few lines over what the terminal already
//! offers. A table crate and a spinner crate would be two dependencies for
//! two functions.
//!
//! # It degrades rather than insisting
//!
//! Colour, box drawing and animation are all conditional on the output being
//! a terminal a person is watching. Redirected to a file, piped to `grep`, or
//! run under a CI runner that sets `NO_COLOR`, every one of them turns off
//! and the same command prints plain, stable, greppable text. That is what
//! makes one command usable both at a desk and in a workflow. `FORCE_COLOR`
//! or `CLICOLOR_FORCE` brings the colour back into a pipe, and only the
//! colour.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use crossterm::style::Stylize;
use yggdryl::logging::Level;

/// Whether standard output is written in colour: the core's colour rule,
/// the one its log handlers follow. `NO_COLOR`, `FORCE_COLOR`,
/// `CLICOLOR_FORCE` and `TERM=dumb` are the conventions every other tool a
/// person has in their shell honours, and a pipe is honoured because
/// nothing downstream of one wants escape codes unless it asked for them.
#[must_use]
pub fn colored() -> bool {
    yggdryl::logging::is_color_enabled(std::io::stdout().is_terminal())
}

/// Whether the terminal should be drawn on rather than written to - box
/// drawing and animation: only a terminal a person watches, coloured, ever
/// is. A forced colour asks for escape codes in a pipe, never for a
/// spinner's frames in it.
#[must_use]
pub fn decorated() -> bool {
    std::io::stdout().is_terminal() && colored()
}

/// One string in a colour, or plainly where colour is not wanted.
macro_rules! paint {
    ($name:ident, $method:ident) => {
        #[doc = concat!("Renders `text` ", stringify!($name), ", where colour is wanted.")]
        #[must_use]
        pub fn $name(text: &str) -> String {
            if colored() {
                text.$method().to_string()
            } else {
                text.to_owned()
            }
        }
    };
}

paint!(bold, bold);
paint!(dim, dim);
paint!(cyan, cyan);
paint!(green, green);
paint!(yellow, yellow);
paint!(red, red);
paint!(magenta, magenta);

/// The characters a table is drawn with.
///
/// Box drawing where a terminal will render it, ASCII where the output is
/// going somewhere that may not - a log file read on a system with a
/// different code page shows `+---+` correctly and `┌───┐` as noise.
struct Rules {
    corner: [&'static str; 6],
    horizontal: &'static str,
    vertical: &'static str,
}

/// The rules this run draws with.
fn rules() -> Rules {
    if decorated() {
        Rules {
            corner: ["┌", "┬", "┐", "└", "┴", "┘"],
            horizontal: "─",
            vertical: "│",
        }
    } else {
        Rules {
            corner: ["+", "+", "+", "+", "+", "+"],
            horizontal: "-",
            vertical: "|",
        }
    }
}

/// Write to standard output, ending the process quietly when the reader has
/// gone.
///
/// A reader that stops early - `yggdryl fix schema | head` - closes the pipe, and
/// every Unix filter then ends without a word; `print!` panics instead. Any
/// other write failure still panics as `print!` would, naming the error.
pub fn write_out(text: std::fmt::Arguments<'_>) {
    use std::io::Write as _;
    if let Err(error) = std::io::stdout().lock().write_fmt(text) {
        if error.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        panic!("failed printing to stdout: {error}");
    }
}

/// `text` as a workflow command's message carries it: the three characters
/// the runner reads as structure escaped, so a reason quoting a document
/// arrives whole on one line.
#[must_use]
pub fn annotation(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// A heading over a section of output.
pub fn heading(text: &str) {
    outln!("\n{}", bold(&cyan(text)));
}

/// One key and its value, aligned under a heading.
pub fn entry(key: &str, value: &str) {
    outln!("  {:<18} {value}", dim(key));
}

/// A note the reader should see but not act on: marked as the core's log
/// lines mark a record below `INFO`.
pub fn note(text: &str) {
    outln!("{} {text}", dim(Level::DEBUG.glyph()));
}

/// A statement that something is right.
pub fn good(text: &str) {
    outln!("{} {text}", green("✓"));
}

/// A statement that something needs attention but is not a failure: the
/// core's `WARNING` glyph.
pub fn warn(text: &str) {
    outln!("{} {text}", yellow(Level::WARNING.glyph()));
}

/// A statement that something is wrong: the core's `ERROR` glyph.
pub fn bad(text: &str) {
    outln!("{} {text}", red(Level::ERROR.glyph()));
}

/// How wide one cell renders, counted in what a terminal shows.
///
/// Characters rather than bytes, so a description with an accent in it does
/// not shift the column after it. Not grapheme clusters: that needs a table
/// this crate would have to carry, and a FIX dictionary is ASCII with the
/// occasional Latin-1 name.
fn width(cell: &str) -> usize {
    cell.chars().count()
}

/// One cell padded to `width`, truncated with an ellipsis where it overflows.
fn cell(text: &str, wanted: usize) -> String {
    let held = width(text);
    if held <= wanted {
        return format!("{text}{}", " ".repeat(wanted - held));
    }
    let kept: String = text.chars().take(wanted.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// How wide one column may grow before it is truncated.
///
/// One long description must not push every other column off the screen.
const CAP: usize = 48;

/// A table with a header row, drawn to fit its own contents.
///
/// Columns are as wide as what is in them, capped so one long description
/// cannot push every other column off the screen. A row shorter than the
/// header is padded rather than refused: a partial row is still a row.
pub fn table(header: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        note("nothing to show");
        return;
    }
    let mut widths: Vec<usize> = header.iter().map(|held| width(held)).collect();
    for row in rows {
        for (at, held) in row.iter().enumerate() {
            if at < widths.len() {
                widths[at] = widths[at].max(width(held)).min(CAP);
            }
        }
    }

    let rules = rules();
    let line = |left: &str, join: &str, right: &str| {
        let middle: Vec<String> = widths
            .iter()
            .map(|held| rules.horizontal.repeat(held + 2))
            .collect();
        outln!("{}", dim(&format!("{left}{}{right}", middle.join(join))));
    };

    line(rules.corner[0], rules.corner[1], rules.corner[2]);
    let titles: Vec<String> = header
        .iter()
        .zip(&widths)
        .map(|(held, wanted)| bold(&cell(held, *wanted)))
        .collect();
    outln!(
        "{} {} {}",
        dim(rules.vertical),
        titles.join(&format!(" {} ", dim(rules.vertical))),
        dim(rules.vertical),
    );
    line(rules.corner[3], rules.corner[1], rules.corner[5]);
    for row in rows {
        let cells: Vec<String> = widths
            .iter()
            .enumerate()
            .map(|(at, wanted)| cell(row.get(at).map_or("", String::as_str), *wanted))
            .collect();
        outln!(
            "{} {} {}",
            dim(rules.vertical),
            cells.join(&format!(" {} ", dim(rules.vertical))),
            dim(rules.vertical),
        );
    }
    line(rules.corner[3], rules.corner[4], rules.corner[5]);
}

/// The frames a spinner cycles through.
const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A line that spins while something slow happens, and clears when it ends.
///
/// Only where a person is watching: redirected output gets one line saying
/// what started and one saying what it cost, which is what a CI log wants and
/// what a spinner would fill with escape codes.
pub struct Progress {
    label: String,
    started: Instant,
    frame: usize,
    animated: bool,
}

impl Progress {
    /// Starts a progress line.
    #[must_use]
    pub fn start(label: impl Into<String>) -> Self {
        let held = Self {
            label: label.into(),
            started: Instant::now(),
            frame: 0,
            animated: decorated(),
        };
        if !held.animated {
            outln!("{}…", held.label);
        }
        held
    }

    /// Advances the animation, at most as often as the eye resolves.
    pub fn tick(&mut self) {
        if !self.animated {
            return;
        }
        self.frame = self.frame.wrapping_add(1);
        let held = FRAMES[self.frame % FRAMES.len()];
        out!("\r{} {} ", cyan(held), self.label);
        let _ = std::io::stdout().flush();
    }

    /// Ends the line, saying what it did and what it cost.
    pub fn finish(self, outcome: &str) {
        let spent = self.started.elapsed();
        if self.animated {
            out!("\r{}\r", " ".repeat(self.label.chars().count() + 4));
        }
        outln!("{} {outcome} {}", green("✓"), dim(&rendered(spent)));
    }
}

/// One duration, in the largest unit that still reads as a number.
#[must_use]
pub fn rendered(spent: Duration) -> String {
    let millis = spent.as_millis();
    if millis < 1_000 {
        return format!("{millis}ms");
    }
    format!("{:.1}s", spent.as_secs_f64())
}
