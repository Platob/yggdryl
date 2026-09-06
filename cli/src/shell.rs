//! The interactive shell: one line at a time, with completion as you type.
//!
//! A dictionary of six thousand fields is not something anyone remembers the
//! spelling of, so the shell completes from the dictionary itself rather than
//! from a fixed word list - tags, names, and the commands that take them. Tab
//! completes the common prefix and shows the alternatives; the arrows walk
//! what has already been asked.
//!
//! Written directly on the terminal's key events rather than on a line-editor
//! crate, because what is wanted is small: a cursor, a history, a completion
//! that knows the dictionary. Everything a bigger editor adds - vi bindings,
//! multiline, syntax highlighting - is something this would never use.

use std::io::{Write, stdout};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{cursor, execute, terminal};

use crate::style;

/// What the shell knows how to complete.
pub struct Completions {
    /// The commands, which complete at the start of a line.
    pub commands: Vec<String>,
    /// The dictionary's own words: every tag and every field name.
    pub words: Vec<String>,
}

impl Completions {
    /// Everything that could follow what has been typed so far.
    ///
    /// The first word completes from the commands, because a line always
    /// starts with one; anything after it completes from the dictionary,
    /// because that is what every command takes.
    #[must_use]
    pub fn matching(&self, line: &str) -> Vec<String> {
        let (source, prefix) = match line.rsplit_once(' ') {
            None => (&self.commands, line),
            Some((_, tail)) => (&self.words, tail),
        };
        if prefix.is_empty() && source.len() > 40 {
            return Vec::new();
        }
        let folded = prefix.to_lowercase();
        source
            .iter()
            .filter(|held| held.to_lowercase().starts_with(&folded))
            .take(200)
            .cloned()
            .collect()
    }
}

/// The longest prefix every candidate shares.
fn common(candidates: &[String]) -> String {
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let mut held = first.clone();
    for candidate in &candidates[1..] {
        while !candidate.to_lowercase().starts_with(&held.to_lowercase()) {
            held.pop();
            if held.is_empty() {
                return held;
            }
        }
    }
    held
}

/// One line, read with completion and history.
///
/// Answers `None` where the reader asked to leave - end of input, or the
/// interrupt every shell uses for it.
///
/// # Errors
///
/// Returns the terminal's own failure when raw mode cannot be entered, which
/// is what happens where there is no terminal at all.
pub fn read_line(
    prompt: &str,
    history: &mut Vec<String>,
    completions: &Completions,
) -> std::io::Result<Option<String>> {
    print!("{prompt}");
    stdout().flush()?;
    enable_raw_mode()?;
    let held = edit(prompt, history, completions);
    disable_raw_mode()?;
    println!();
    held
}

/// The editing loop, with raw mode already on.
fn edit(
    prompt: &str,
    history: &mut Vec<String>,
    completions: &Completions,
) -> std::io::Result<Option<String>> {
    let mut line = String::new();
    let mut at = 0_usize;
    let mut recall = history.len();

    loop {
        let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        else {
            continue;
        };
        match (code, modifiers) {
            (KeyCode::Enter, _) => {
                if !line.trim().is_empty() {
                    history.push(line.clone());
                }
                return Ok(Some(line));
            }
            // The two ways every shell is left.
            (KeyCode::Char('d'), KeyModifiers::CONTROL) if line.is_empty() => return Ok(None),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(None),
            (KeyCode::Char(held), _) => {
                line.insert(at, held);
                at += held.len_utf8();
            }
            (KeyCode::Backspace, _) => {
                if at > 0 {
                    let previous = line[..at].chars().next_back().map_or(0, char::len_utf8);
                    at -= previous;
                    line.remove(at);
                }
            }
            (KeyCode::Delete, _) if at < line.len() => {
                line.remove(at);
            }
            (KeyCode::Left, _) if at > 0 => {
                at -= line[..at].chars().next_back().map_or(0, char::len_utf8);
            }
            (KeyCode::Right, _) if at < line.len() => {
                at += line[at..].chars().next().map_or(0, char::len_utf8);
            }
            (KeyCode::Home, _) => at = 0,
            (KeyCode::End, _) => at = line.len(),
            (KeyCode::Up, _) if recall > 0 => {
                recall -= 1;
                line.clone_from(&history[recall]);
                at = line.len();
            }
            (KeyCode::Down, _) => {
                recall = (recall + 1).min(history.len());
                line = history.get(recall).cloned().unwrap_or_default();
                at = line.len();
            }
            (KeyCode::Tab, _) => {
                let candidates = completions.matching(&line[..at]);
                match candidates.len() {
                    0 => {}
                    1 => {
                        let held = &candidates[0];
                        let start = line[..at].rfind(' ').map_or(0, |space| space + 1);
                        line.replace_range(start..at, held);
                        at = start + held.len();
                        line.insert(at, ' ');
                        at += 1;
                    }
                    _ => {
                        let shared = common(&candidates);
                        let start = line[..at].rfind(' ').map_or(0, |space| space + 1);
                        if shared.len() > at - start {
                            line.replace_range(start..at, &shared);
                            at = start + shared.len();
                        }
                        show(&candidates)?;
                    }
                }
            }
            _ => {}
        }
        redraw(prompt, &line, at)?;
    }
}

/// Redraws the line and puts the cursor back where it was.
fn redraw(prompt: &str, line: &str, at: usize) -> std::io::Result<()> {
    execute!(
        stdout(),
        cursor::MoveToColumn(0),
        terminal::Clear(terminal::ClearType::UntilNewLine),
    )?;
    print!("{prompt}{line}");
    let column = prompt.chars().count() + line[..at].chars().count();
    let column = u16::try_from(column).unwrap_or(u16::MAX);
    execute!(stdout(), cursor::MoveToColumn(column))?;
    stdout().flush()
}

/// Shows what a prefix could still become, in columns.
fn show(candidates: &[String]) -> std::io::Result<()> {
    println!();
    let widest = candidates.iter().map(String::len).max().unwrap_or(0) + 2;
    let columns = (terminal::size().map_or(80, |(width, _)| width) as usize / widest).max(1);
    for chunk in candidates.chunks(columns) {
        let cells: Vec<String> = chunk
            .iter()
            .map(|held| format!("{held:<widest$}"))
            .collect();
        print!("\r{}\n", style::dim(&cells.concat()));
    }
    if candidates.len() >= 200 {
        print!("\r{}\n", style::dim("… and more"));
    }
    stdout().flush()
}
