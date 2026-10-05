//! The formula tokenizer: a formula's text cut into the pieces its grammar
//! is made of, each located by its byte offset.
//!
//! Lexing is total. Every byte of the text lands in exactly one lexeme, and
//! a run the grammar does not recognize - a stray character, an unclosed
//! quote - is an [`Kind::Opaque`] lexeme holding it verbatim, so a formula
//! another producer wrote is always carried as it was spelled even where it
//! cannot be read. What a piece means is decided here once: a reference is
//! told from a defined name and a function name by the characters around
//! it (`LOG10(` is a function, `LOG10` a cell, `Sheet1!A1` a qualified
//! cell, `A1#` a spill), and nothing downstream re-reads the text.

use smol_str::SmolStr;

use crate::excel::cell::{CellRef, MAX_ROWS};

/// The prefix a function added to Excel after 2007 is stored under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Prefix {
    /// None: a function every version reads.
    #[default]
    None,
    /// `_xlfn.`: a function a later version added.
    Future,
    /// `_xlfn._xlws.`: a later function tied to the worksheet.
    FutureWorksheet,
    /// `_xlws.` alone.
    Worksheet,
}

impl Prefix {
    /// The prefix as the file spells it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Future => "_xlfn.",
            Self::FutureWorksheet => "_xlfn._xlws.",
            Self::Worksheet => "_xlws.",
        }
    }

    /// Split the prefix off a function name as a file spells it.
    pub(crate) fn split(name: &str) -> (Self, &str) {
        let strip = |text: &'_ str, prefix: &str| -> Option<usize> {
            text.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| prefix.len())
        };
        if let Some(at) = strip(name, "_xlfn.") {
            let rest = &name[at..];
            if let Some(more) = strip(rest, "_xlws.") {
                return (Self::FutureWorksheet, &rest[more..]);
            }
            return (Self::Future, rest);
        }
        if let Some(at) = strip(name, "_xlws.") {
            return (Self::Worksheet, &name[at..]);
        }
        (Self::None, name)
    }
}

/// A sheet prefix as the text spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RawSheet {
    /// No prefix.
    Own,
    /// `Sheet1!`, `'Q1 ''24'!`: the name unquoted, and whether it was quoted.
    Named { name: SmolStr, quoted: bool },
    /// `Jan:Mar!`, `'Jan 1:Mar 1'!`.
    Span {
        first: SmolStr,
        last: SmolStr,
        quoted: bool,
    },
    /// `[1]Sheet1!`, `'[Book.xlsx]Q 1'!`: the text before the `!`, verbatim.
    External(SmolStr),
    /// `#REF!`: a sheet that was removed.
    Invalid,
}

/// One corner coordinate as written: the zero-based index and its `$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawCoord {
    pub(crate) index: u32,
    pub(crate) absolute: bool,
}

/// The cells a reference names, as written: absolute indices, not yet
/// relative to a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RawTarget {
    Cell {
        row: RawCoord,
        column: RawCoord,
    },
    Area {
        first: (RawCoord, RawCoord),
        last: (RawCoord, RawCoord),
    },
    Rows {
        first: RawCoord,
        last: RawCoord,
    },
    Columns {
        first: RawCoord,
        last: RawCoord,
    },
    /// A defined name qualified by a sheet: `Sheet1!Print_Area`.
    Name(SmolStr),
    /// `#REF!` after a prefix.
    Invalid,
}

/// A reference as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawReference {
    pub(crate) sheet: RawSheet,
    pub(crate) target: RawTarget,
}

/// What a lexeme is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Spaces, tabs and line breaks: the intersection operator between two
    /// references, else layout.
    Space,
    /// A number literal: `1`, `2.5`, `.5`, `1E+10`.
    Number,
    /// A text literal in double quotes, `""` its escaped quote; `closed`
    /// false when the text ends inside it.
    String { closed: bool },
    /// `TRUE` or `FALSE`, not called.
    Boolean,
    /// An error literal: `#N/A`, `#DIV/0!`.
    Error,
    /// An arithmetic, text or comparison operator: `+ - * / ^ & % = <> < >
    /// <= >=`.
    Operator,
    /// `:` between two operands that are not one reference.
    Range,
    /// `(`.
    Open,
    /// `)`.
    Close,
    /// `{`, opening an array constant.
    ArrayOpen,
    /// `}`.
    ArrayClose,
    /// `,`: an argument separator, or the union operator in parentheses.
    Comma,
    /// `;`: the row separator of an array constant.
    Semicolon,
    /// `@`: implicit intersection, as entry spells it.
    At,
    /// A function name followed by its `(`: the prefix and the bare name.
    Function { prefix: Prefix, name: SmolStr },
    /// A defined name, or any identifier that is none of the above.
    Name,
    /// A reference to cells.
    Reference(RawReference),
    /// A structured reference to a table: `Table1[Column]`, `[@Column]`.
    Structured,
    /// `#` after a reference: the range a dynamic array spills to.
    Spill,
    /// A run the grammar does not read, kept verbatim.
    Opaque,
}

/// One piece of a formula's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lexeme<'a> {
    pub(crate) kind: Kind,
    /// The piece as written.
    pub(crate) text: &'a str,
    /// Its byte offset in the formula.
    pub(crate) start: usize,
}

/// The error literals Excel spells, longest first where one opens another.
const ERRORS: [&str; 18] = [
    "#GETTING_DATA",
    "#EXTERNAL!",
    "#CONNECT!",
    "#BLOCKED!",
    "#UNKNOWN!",
    "#TIMEOUT!",
    "#PYTHON!",
    "#DIV/0!",
    "#VALUE!",
    "#SPILL!",
    "#FIELD!",
    "#NULL!",
    "#NAME?",
    "#CALC!",
    "#BUSY!",
    "#NUM!",
    "#REF!",
    "#N/A",
];

/// Cut `text` into lexemes, every byte in one of them.
pub(crate) fn lex(text: &str) -> Vec<Lexeme<'_>> {
    let mut lexer = Lexer {
        text,
        position: 0,
        lexemes: Vec::new(),
    };
    while lexer.position < text.len() {
        lexer.step();
    }
    lexer.lexemes
}

/// Whether a character separates tokens as whitespace.
fn is_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n' | '\u{a0}')
}

/// Whether a character continues a name: a letter or digit of any script,
/// `_`, `.`, `\` or `?`.
fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '.' | '\\' | '?')
}

/// Whether a character may open a name.
fn opens_name(character: char) -> bool {
    character.is_alphabetic() || matches!(character, '_' | '\\')
}

/// Whether a character belongs to a sheet name written without quotes.
fn is_sheet_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '.')
}

struct Lexer<'a> {
    text: &'a str,
    position: usize,
    lexemes: Vec<Lexeme<'a>>,
}

impl<'a> Lexer<'a> {
    /// The text from `at` on.
    fn rest(&self, at: usize) -> &'a str {
        self.text.get(at..).unwrap_or("")
    }

    /// The character at `at`.
    fn char_at(&self, at: usize) -> Option<char> {
        self.rest(at).chars().next()
    }

    /// Push the lexeme `kind` over `start..end` and move past it.
    fn push(&mut self, kind: Kind, start: usize, end: usize) {
        // An opaque run joins the one before it: one piece per run.
        if kind == Kind::Opaque
            && let Some(last) = self.lexemes.last_mut()
            && last.kind == Kind::Opaque
            && last.start + last.text.len() == start
        {
            last.text = &self.text[last.start..end];
            self.position = end;
            return;
        }
        self.lexemes.push(Lexeme {
            kind,
            text: &self.text[start..end],
            start,
        });
        self.position = end;
    }

    /// Read the lexeme at the cursor.
    fn step(&mut self) {
        let start = self.position;
        let Some(character) = self.char_at(start) else {
            return;
        };
        let next = start + character.len_utf8();
        match character {
            c if is_space(c) => {
                let end = start
                    + self
                        .rest(start)
                        .find(|c: char| !is_space(c))
                        .unwrap_or(self.text.len() - start);
                self.push(Kind::Space, start, end);
            }
            '"' => self.string(start),
            '\'' => self.quoted(start),
            '[' => self.bracket(start),
            '#' => self.hash(start),
            '{' => self.push(Kind::ArrayOpen, start, next),
            '}' => self.push(Kind::ArrayClose, start, next),
            '(' => self.push(Kind::Open, start, next),
            ')' => self.push(Kind::Close, start, next),
            ',' => self.push(Kind::Comma, start, next),
            ';' => self.push(Kind::Semicolon, start, next),
            '@' => self.push(Kind::At, start, next),
            ':' => self.push(Kind::Range, start, next),
            '+' | '-' | '*' | '/' | '^' | '&' | '%' | '=' => {
                self.push(Kind::Operator, start, next);
            }
            '<' => {
                let end = if matches!(self.char_at(next), Some('=' | '>')) {
                    next + 1
                } else {
                    next
                };
                self.push(Kind::Operator, start, end);
            }
            '>' => {
                let end = if self.char_at(next) == Some('=') {
                    next + 1
                } else {
                    next
                };
                self.push(Kind::Operator, start, end);
            }
            c if c == '$' || c.is_ascii_digit() || c == '.' || opens_name(c) => self.word(start),
            _ => self.push(Kind::Opaque, start, next),
        }
    }

    /// A text literal from the `"` at `start`.
    fn string(&mut self, start: usize) {
        let bytes = self.text.as_bytes();
        let mut at = start + 1;
        while at < bytes.len() {
            if bytes[at] == b'"' {
                if bytes.get(at + 1) == Some(&b'"') {
                    at += 2;
                    continue;
                }
                self.push(Kind::String { closed: true }, start, at + 1);
                return;
            }
            at += 1;
        }
        self.push(Kind::String { closed: false }, start, self.text.len());
    }

    /// The end of the quoted run opening at `start`, past its closing `'`,
    /// `''` read as one apostrophe; `None` when the text ends inside it.
    fn quoted_end(&self, start: usize) -> Option<usize> {
        let bytes = self.text.as_bytes();
        let mut at = start + 1;
        while at < bytes.len() {
            if bytes[at] == b'\'' {
                if bytes.get(at + 1) == Some(&b'\'') {
                    at += 2;
                    continue;
                }
                return Some(at + 1);
            }
            at += 1;
        }
        None
    }

    /// A quoted sheet prefix from the `'` at `start`, and the reference it
    /// qualifies; anything else is opaque.
    fn quoted(&mut self, start: usize) {
        let Some(end) = self.quoted_end(start) else {
            self.push(Kind::Opaque, start, self.text.len());
            return;
        };
        if self.char_at(end) != Some('!') {
            self.push(Kind::Opaque, start, end);
            return;
        }
        let inner = self.text[start + 1..end - 1].replace("''", "'");
        let sheet = if inner.contains('[') {
            RawSheet::External(SmolStr::new(&self.text[start..end]))
        } else if let Some((first, last)) = inner.split_once(':') {
            RawSheet::Span {
                first: SmolStr::new(first),
                last: SmolStr::new(last),
                quoted: true,
            }
        } else {
            RawSheet::Named {
                name: SmolStr::new(inner),
                quoted: true,
            }
        };
        self.qualified(start, sheet, end + 1);
    }

    /// What follows a sheet prefix ending at `at`: the reference it
    /// qualifies, from `start`; a prefix qualifying nothing is opaque.
    fn qualified(&mut self, start: usize, sheet: RawSheet, at: usize) {
        if let Some((target, end)) = self.target_at(at) {
            self.push(Kind::Reference(RawReference { sheet, target }), start, end);
            return;
        }
        if self
            .rest(at)
            .get(..5)
            .is_some_and(|head| head.eq_ignore_ascii_case("#REF!"))
        {
            self.push(
                Kind::Reference(RawReference {
                    sheet,
                    target: RawTarget::Invalid,
                }),
                start,
                at + 5,
            );
            return;
        }
        let name_end = self.name_end(at);
        if name_end > at && self.char_at(at).is_some_and(opens_name) {
            self.push(
                Kind::Reference(RawReference {
                    sheet,
                    target: RawTarget::Name(SmolStr::new(&self.text[at..name_end])),
                }),
                start,
                name_end,
            );
            return;
        }
        self.push(Kind::Opaque, start, at);
    }

    /// A `[` at `start`: the prefix of a sheet in another workbook
    /// (`[1]Sheet1!A1`, `[1]!Name`), else a structured reference inside a
    /// table (`[@Column]`).
    fn bracket(&mut self, start: usize) {
        if let Some(close) = self.rest(start).find(']').map(|at| start + at) {
            let sheet_end = self.sheet_end(close + 1);
            if self.char_at(sheet_end) == Some('!') {
                let sheet = RawSheet::External(SmolStr::new(&self.text[start..sheet_end]));
                self.qualified(start, sheet, sheet_end + 1);
                return;
            }
        }
        match self.brackets_end(start) {
            Some(end) => self.push(Kind::Structured, start, end),
            None => self.push(Kind::Opaque, start, self.text.len()),
        }
    }

    /// The end of the balanced brackets opening at `start`, a `'` escaping
    /// the character after it; `None` when they never close.
    fn brackets_end(&self, start: usize) -> Option<usize> {
        let mut depth = 0_usize;
        let mut escaped = false;
        for (offset, character) in self.rest(start).char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\'' => escaped = true,
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(start + offset + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// A `#` at `start`: the spill of the reference just before it, an
    /// error literal, `#REF!` qualifying a reference to a removed sheet, or
    /// an opaque run.
    fn hash(&mut self, start: usize) {
        let adjacent = self.lexemes.last().is_some_and(|last| {
            last.start + last.text.len() == start
                && matches!(last.kind, Kind::Reference(_) | Kind::Name)
        });
        if adjacent {
            self.push(Kind::Spill, start, start + 1);
            return;
        }
        let rest = self.rest(start);
        let Some(error) = ERRORS.iter().find(|error| {
            rest.get(..error.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(error))
        }) else {
            self.push(Kind::Opaque, start, start + 1);
            return;
        };
        let end = start + error.len();
        if *error == "#REF!"
            && let Some((target, target_end)) = self.target_at(end)
        {
            self.push(
                Kind::Reference(RawReference {
                    sheet: RawSheet::Invalid,
                    target,
                }),
                start,
                target_end,
            );
            return;
        }
        self.push(Kind::Error, start, end);
    }

    /// The end of the unquoted sheet name starting at `at`.
    fn sheet_end(&self, at: usize) -> usize {
        at + self
            .rest(at)
            .find(|c: char| !is_sheet_character(c))
            .unwrap_or(self.text.len() - at)
    }

    /// The end of the name starting at `at`.
    fn name_end(&self, at: usize) -> usize {
        at + self
            .rest(at)
            .find(|c: char| !is_name_character(c))
            .unwrap_or(self.text.len() - at)
    }

    /// A word at `start`: a sheet-qualified reference, a reference, a
    /// number, a function name, a boolean, a structured reference or a
    /// defined name.
    fn word(&mut self, start: usize) {
        // `Sheet1!` or `Jan:Mar!`.
        let first_end = self.sheet_end(start);
        if first_end > start {
            match self.char_at(first_end) {
                Some('!') => {
                    let sheet = RawSheet::Named {
                        name: SmolStr::new(&self.text[start..first_end]),
                        quoted: false,
                    };
                    self.qualified(start, sheet, first_end + 1);
                    return;
                }
                Some(':') => {
                    let last_end = self.sheet_end(first_end + 1);
                    // A side Excel would have quoted - `A1`, `C`, `R1C1` -
                    // makes `A1:Sheet1!A1` a range beside a reference,
                    // never an unquoted span of sheets.
                    let unquoted = |from: usize, to: usize| {
                        !super::reference::needs_quotes(&self.text[from..to])
                    };
                    if last_end > first_end + 1
                        && self.char_at(last_end) == Some('!')
                        && unquoted(start, first_end)
                        && unquoted(first_end + 1, last_end)
                    {
                        let sheet = RawSheet::Span {
                            first: SmolStr::new(&self.text[start..first_end]),
                            last: SmolStr::new(&self.text[first_end + 1..last_end]),
                            quoted: false,
                        };
                        // `A1:B2!` is no span of sheets a reference can name
                        // unless a reference follows.
                        if self.target_at(last_end + 1).is_some() {
                            self.qualified(start, sheet, last_end + 1);
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some((target, end)) = self.target_at(start) {
            self.push(
                Kind::Reference(RawReference {
                    sheet: RawSheet::Own,
                    target,
                }),
                start,
                end,
            );
            return;
        }
        let first = self.char_at(start).unwrap_or(' ');
        if first.is_ascii_digit() || first == '.' {
            let end = self.number_end(start);
            if end > start && !self.char_at(end).is_some_and(is_name_character) {
                self.push(Kind::Number, start, end);
                return;
            }
        }
        if !opens_name(first) {
            self.push(Kind::Opaque, start, start + first.len_utf8());
            return;
        }
        let end = self.name_end(start);
        let name = &self.text[start..end];
        match self.char_at(end) {
            Some('(') => {
                let (prefix, bare) = Prefix::split(name);
                self.push(
                    Kind::Function {
                        prefix,
                        name: SmolStr::new(bare),
                    },
                    start,
                    end,
                );
            }
            Some('[') => match self.brackets_end(end) {
                Some(close) => self.push(Kind::Structured, start, close),
                None => self.push(Kind::Opaque, start, self.text.len()),
            },
            _ if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") => {
                self.push(Kind::Boolean, start, end);
            }
            _ => self.push(Kind::Name, start, end),
        }
    }

    /// The end of the number literal at `at`, `at` itself for none.
    fn number_end(&self, at: usize) -> usize {
        let bytes = self.text.as_bytes();
        let mut end = at;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'.' {
            end += 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        if end == at || &self.text[at..end] == "." {
            return at;
        }
        if end < bytes.len() && matches!(bytes[end], b'e' | b'E') {
            let mut exponent = end + 1;
            if exponent < bytes.len() && matches!(bytes[exponent], b'+' | b'-') {
                exponent += 1;
            }
            let digits = exponent;
            while exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
                exponent += 1;
            }
            if exponent > digits {
                end = exponent;
            }
        }
        end
    }

    /// The reference to cells at `at` and where it ends, `None` when the
    /// text there is no reference: a cell, an area between two cells, whole
    /// rows or whole columns, ended by a character no name continues with
    /// and never by the `(` of a call.
    fn target_at(&self, at: usize) -> Option<(RawTarget, usize)> {
        let ends = |end: usize| {
            !self
                .char_at(end)
                .is_some_and(|c| is_name_character(c) || matches!(c, '(' | '!' | '$' | '[' | '\''))
        };
        match self.corner(at) {
            Some((Corner::Cell(row, column), end)) => {
                if self.char_at(end) == Some(':')
                    && let Some((Corner::Cell(last_row, last_column), last_end)) =
                        self.corner(end + 1)
                    && ends(last_end)
                {
                    return Some((
                        RawTarget::Area {
                            first: (row, column),
                            last: (last_row, last_column),
                        },
                        last_end,
                    ));
                }
                ends(end).then_some((RawTarget::Cell { row, column }, end))
            }
            Some((Corner::Column(first), end)) if self.char_at(end) == Some(':') => {
                match self.corner(end + 1) {
                    Some((Corner::Column(last), last_end)) if ends(last_end) => {
                        Some((RawTarget::Columns { first, last }, last_end))
                    }
                    _ => None,
                }
            }
            Some((Corner::Row(first), end)) if self.char_at(end) == Some(':') => {
                match self.corner(end + 1) {
                    Some((Corner::Row(last), last_end)) if ends(last_end) => {
                        Some((RawTarget::Rows { first, last }, last_end))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// One corner at `at`: `$A$1`, `A1`, a column `$A` or a row `$1`.
    fn corner(&self, at: usize) -> Option<(Corner, usize)> {
        let bytes = self.text.as_bytes();
        let mut end = at;
        let column_absolute = bytes.get(end) == Some(&b'$');
        if column_absolute {
            end += 1;
        }
        let letters_start = end;
        while end < bytes.len() && bytes[end].is_ascii_alphabetic() && end - letters_start < 4 {
            end += 1;
        }
        let letters = &self.text[letters_start..end];
        if letters.is_empty() {
            // A row alone: `$3`, `3`.
            let digits_start = end;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let row = row_index(&self.text[digits_start..end])?;
            return Some((
                Corner::Row(RawCoord {
                    index: row,
                    absolute: column_absolute,
                }),
                end,
            ));
        }
        let column = CellRef::column_index(letters)?;
        let column = RawCoord {
            index: column,
            absolute: column_absolute,
        };
        let row_absolute = bytes.get(end) == Some(&b'$');
        let digits_start = end + usize::from(row_absolute);
        let mut digits_end = digits_start;
        while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
            digits_end += 1;
        }
        if digits_end == digits_start {
            return (!row_absolute).then_some((Corner::Column(column), end));
        }
        let row = row_index(&self.text[digits_start..digits_end])?;
        Some((
            Corner::Cell(
                RawCoord {
                    index: row,
                    absolute: row_absolute,
                },
                column,
            ),
            digits_end,
        ))
    }
}

/// One corner of a reference.
enum Corner {
    /// A cell: its row, then its column.
    Cell(RawCoord, RawCoord),
    Column(RawCoord),
    Row(RawCoord),
}

/// The zero-based row a row number spells, `None` off the grid.
fn row_index(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    digits
        .parse::<u32>()
        .ok()
        .filter(|row| (1..=MAX_ROWS).contains(row))
        .map(|row| row - 1)
}
