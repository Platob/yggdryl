//! The styles part: which cells hold a date, a time or a duration.
//!
//! A cell's `s` attribute indexes `cellXfs`, whose `xf` names a `numFmtId`;
//! the id is a built-in format or a `numFmt` the part declares. That is the
//! only fact this crate reads out of `styles.xml`, because it is the only
//! fact that changes what a number *is*: `45292` under `yyyy-mm-dd` is a
//! date, under `General` a number. Fonts, fills, borders, alignment and
//! colours are display, and are neither read nor kept.
//!
//! The part this crate writes is the smallest one Excel opens without
//! repair: the reserved fills, one font, one border, the `Normal` style, and
//! six cell formats - General, then the five temporal formats
//! [`NumberFormat`] names, each a `cellXfs` index a cell states in `s`.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use quick_xml::Reader;
use quick_xml::events::Event;
use smol_str::SmolStr;

use crate::Result;

use super::package::{attribute, codec_error, local_name};

/// What a cell's number format says its number is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NumberFormat {
    /// `General`, a number format, text, or anything that is not a date: the
    /// number is a number.
    #[default]
    General,
    /// A date without a clock, `yyyy-mm-dd`: the serial is a day.
    Date,
    /// A clock without a date, `h:mm:ss`: the serial is a fraction of a day.
    Time,
    /// A date and a clock, `yyyy-mm-dd hh:mm:ss`.
    DateTime,
    /// A date and a clock to the millisecond, `yyyy-mm-dd hh:mm:ss.000`.
    DateTimeFraction,
    /// An elapsed time past a day, `[h]:mm:ss`: the serial is a duration.
    Duration,
}

impl NumberFormat {
    /// Every format, in `cellXfs` order of the part this crate writes.
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Date,
        Self::DateTime,
        Self::DateTimeFraction,
        Self::Time,
        Self::Duration,
    ];

    /// The format's name: `general`, `date`, `time`, `datetime`,
    /// `datetime_fraction` or `duration`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
            Self::DateTimeFraction => "datetime_fraction",
            Self::Duration => "duration",
        }
    }

    /// The `cellXfs` index a cell of this format states in `s`, in the part
    /// this crate writes.
    #[must_use]
    pub const fn style_index(self) -> u32 {
        match self {
            Self::General => 0,
            Self::Date => 1,
            Self::DateTime => 2,
            Self::DateTimeFraction => 3,
            Self::Time => 4,
            Self::Duration => 5,
        }
    }

    /// Whether the format reads a number as a date, a time or a duration.
    #[must_use]
    pub const fn is_temporal(self) -> bool {
        !matches!(self, Self::General)
    }

    /// The format a built-in `numFmtId` implies, `None` for an id no
    /// built-in table lists - a custom format, which the part declares.
    ///
    /// The ids are ECMA-376's implied formats: 14 to 17 dates, 18 to 21
    /// times, 22 a datetime, 45 and 47 times, 46 the elapsed `[h]:mm:ss`,
    /// and the East Asian locale formats 27 to 36 and 50 to 58, dates and
    /// times as their codes spell them.
    #[must_use]
    pub const fn builtin(id: u32) -> Option<Self> {
        Some(match id {
            14..=17 | 27..=31 | 36 | 50 | 51 | 54 | 57 | 58 => Self::Date,
            18..=21 | 32..=35 | 45 | 47 | 52 | 53 | 55 | 56 => Self::Time,
            22 => Self::DateTime,
            46 => Self::Duration,
            0..=13 | 37..=44 | 48 | 49 => Self::General,
            _ => return None,
        })
    }

    /// The format a `numFmt` code spells.
    ///
    /// Only the first section counts (`positive;negative;zero;text`). Text
    /// in quotes, a bracketed colour or condition, and the character after
    /// `\`, `_` or `*` are literals and say nothing; `[h]`, `[m]` or `[s]`
    /// spell an elapsed time. Among what is left, `y` or `d` makes a date,
    /// `h` or `s` a clock, and `m` alone a date, as Excel reads it; a clock
    /// whose seconds carry a fraction (`ss.000`) beside a date is a datetime
    /// at the fraction; a code with neither is a number.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        let section = code.split(';').next().unwrap_or("");
        let mut date = false;
        let mut time = false;
        let mut month = false;
        let mut fraction = false;
        let mut characters = section.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                // `ss.0`, `ss.00`, `ss.000`: seconds shown with a fraction.
                '.' if time && characters.peek() == Some(&'0') => fraction = true,
                '"' => {
                    for quoted in characters.by_ref() {
                        if quoted == '"' {
                            break;
                        }
                    }
                }
                '[' => {
                    let mut bracketed = String::new();
                    for inner in characters.by_ref() {
                        if inner == ']' {
                            break;
                        }
                        bracketed.push(inner);
                    }
                    let lowered = bracketed.to_ascii_lowercase();
                    if !lowered.is_empty()
                        && lowered.bytes().all(|byte| byte == lowered.as_bytes()[0])
                        && matches!(lowered.as_bytes()[0], b'h' | b'm' | b's')
                    {
                        return Self::Duration;
                    }
                }
                '\\' | '_' | '*' => {
                    characters.next();
                }
                'y' | 'Y' | 'd' | 'D' => date = true,
                'h' | 'H' | 's' | 'S' => time = true,
                'm' | 'M' => month = true,
                _ => {}
            }
        }
        if date && time && fraction {
            Self::DateTimeFraction
        } else if date && time {
            Self::DateTime
        } else if date || (month && !time) {
            Self::Date
        } else if time {
            Self::Time
        } else {
            Self::General
        }
    }

    /// The format code this crate writes for the format, `None` for
    /// General, which states none.
    #[must_use]
    pub const fn code(self) -> Option<&'static str> {
        match self {
            Self::General => None,
            Self::Date => Some("yyyy-mm-dd"),
            Self::DateTime => Some("yyyy-mm-dd hh:mm:ss"),
            Self::DateTimeFraction => Some("yyyy-mm-dd hh:mm:ss.000"),
            Self::Time => Some("hh:mm:ss"),
            Self::Duration => Some("[h]:mm:ss"),
        }
    }

    /// The `numFmtId` the format is written under: a built-in where one
    /// spells it, else a custom id from 164.
    const fn written_id(self) -> u32 {
        match self {
            Self::General => 0,
            Self::Date => 164,
            Self::DateTime => 165,
            Self::DateTimeFraction => 166,
            Self::Time => 21,
            Self::Duration => 46,
        }
    }
}

impl std::str::FromStr for NumberFormat {
    type Err = crate::Error;

    /// Read a format by its name, as [`as_str`](Self::as_str) spells it.
    fn from_str(text: &str) -> crate::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.as_str() == text.trim())
            .ok_or_else(|| crate::Error::Parse {
                target: "number format",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "expected general, date, time, datetime, datetime_fraction or duration for a \
                     number format, got {text:?}"
                ),
            })
    }
}

impl std::fmt::Display for NumberFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The styles part as read: one format per `cellXfs` index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Styles {
    formats: Vec<NumberFormat>,
    /// The highest `numFmtId` the part declares, so formats appended to it
    /// take ids above every one in use.
    max_declared_id: u32,
}

impl Styles {
    /// Read `styles.xml`.
    ///
    /// A `numFmtId` the part declares no code for and no built-in table
    /// lists is General, as Excel displays it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes are not
    /// the part.
    pub(crate) fn from_xml(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::from_reader(bytes);
        let mut buffer = Vec::new();
        let mut codes: BTreeMap<u32, SmolStr> = BTreeMap::new();
        let mut xf_ids: Vec<u32> = Vec::new();
        let mut in_cell_xfs = false;
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start) => {
                    let empty = matches!(event, Event::Empty(_));
                    match local_name(start.name().as_ref()) {
                        b"numFmt" => {
                            let id = attribute(start, b"numFmtId", position)?
                                .and_then(|id| id.trim().parse::<u32>().ok());
                            let code = attribute(start, b"formatCode", position)?;
                            if let (Some(id), Some(code)) = (id, code) {
                                codes.insert(id, SmolStr::new(code));
                            }
                        }
                        b"cellXfs" if !empty => in_cell_xfs = true,
                        b"xf" if in_cell_xfs => {
                            let id = attribute(start, b"numFmtId", position)?
                                .and_then(|id| id.trim().parse::<u32>().ok())
                                .unwrap_or(0);
                            xf_ids.push(id);
                        }
                        _ => {}
                    }
                }
                Event::End(end) => {
                    if local_name(end.name().as_ref()) == b"cellXfs" {
                        in_cell_xfs = false;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        let max_declared_id = codes.keys().next_back().copied().unwrap_or(0);
        let formats = xf_ids
            .into_iter()
            .map(|id| match codes.get(&id) {
                Some(code) => NumberFormat::from_code(code),
                None => NumberFormat::builtin(id).unwrap_or_default(),
            })
            .collect();
        Ok(Self {
            formats,
            max_declared_id,
        })
    }

    /// The `numFmtId` the first of the crate's three custom formats takes
    /// when appended to this part: above every built-in and every id the
    /// part declares.
    pub(crate) fn next_custom_id(&self) -> u32 {
        self.max_declared_id.max(163) + 1
    }

    /// Splice the crate's formats into an existing styles part: its three
    /// custom `numFmt`s appended to `numFmts` (created when absent) from
    /// [`Self::next_custom_id`], its six `xf`s appended to `cellXfs`, the
    /// counts patched. The part's own styles stand untouched, so every cell
    /// of every other sheet keeps its `s`; the crate's cells state their
    /// format's index plus [`Self::len`], the offset this answers.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes are not
    /// the part.
    pub(crate) fn spliced(&self, bytes: &[u8]) -> Result<(Vec<u8>, u32)> {
        // A part this crate already spliced holds the six formats in a run:
        // they are reused at their offset, so a rewrite adds nothing.
        if let Some(offset) = self
            .formats
            .windows(NumberFormat::ALL.len())
            .position(|window| window == NumberFormat::ALL)
        {
            return Ok((bytes.to_vec(), u32::try_from(offset).unwrap_or(u32::MAX)));
        }
        let base = self.next_custom_id();
        let id_of = |format: NumberFormat| match format {
            NumberFormat::Date => base,
            NumberFormat::DateTime => base + 1,
            NumberFormat::DateTimeFraction => base + 2,
            other => other.written_id(),
        };
        let mut num_fmts = String::new();
        for format in [
            NumberFormat::Date,
            NumberFormat::DateTime,
            NumberFormat::DateTimeFraction,
        ] {
            num_fmts.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                id_of(format),
                format.code().expect("a temporal format states a code")
            ));
        }
        let mut xfs = String::new();
        for format in NumberFormat::ALL {
            xfs.push_str(&format!(
                "<xf numFmtId=\"{}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"{}/>",
                id_of(format),
                if format.is_temporal() {
                    " applyNumberFormat=\"1\""
                } else {
                    ""
                }
            ));
        }
        let has_num_fmts = super::package::has_element(bytes, b"numFmts")?;
        let offset = u32::try_from(self.formats.len()).unwrap_or(u32::MAX);
        let rewritten = super::package::rewrite(
            bytes,
            &super::package::Rewrite {
                skip: &|_| false,
                patch_count: &|name| match name {
                    b"numFmts" => Some(3),
                    b"cellXfs" => Some(NumberFormat::ALL.len() as u32),
                    _ => None,
                },
                before_end: &|name| match name {
                    b"numFmts" => Some(num_fmts.clone()),
                    b"cellXfs" => Some(xfs.clone()),
                    _ => None,
                },
                after_start: &|name| {
                    (name == b"styleSheet" && !has_num_fmts)
                        .then(|| format!("<numFmts count=\"3\">{num_fmts}</numFmts>"))
                },
                before_start: &|_| None,
                set_attribute: &|_| None,
            },
        )?;
        Ok((rewritten, offset))
    }

    /// The format of `cellXfs` index `style`; General for one the part does
    /// not declare.
    pub(crate) fn format(&self, style: u32) -> NumberFormat {
        usize::try_from(style)
            .ok()
            .and_then(|index| self.formats.get(index))
            .copied()
            .unwrap_or_default()
    }

    /// Write the styles part this crate's workbooks carry.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure.
    pub(crate) fn write<W: Write>(writer: &mut W) -> Result<()> {
        write!(
            writer,
            "<styleSheet xmlns=\"{}\"><numFmts count=\"3\">",
            super::NAMESPACE
        )?;
        for format in [
            NumberFormat::Date,
            NumberFormat::DateTime,
            NumberFormat::DateTimeFraction,
        ] {
            write!(
                writer,
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                format.written_id(),
                format.code().expect("a temporal format states a code")
            )?;
        }
        write!(
            writer,
            "</numFmts><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
             <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>\
             <fill><patternFill patternType=\"gray125\"/></fill></fills>\
             <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
             <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
             <cellXfs count=\"{}\">",
            NumberFormat::ALL.len()
        )?;
        for format in NumberFormat::ALL {
            write!(
                writer,
                "<xf numFmtId=\"{}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"",
                format.written_id()
            )?;
            if format.is_temporal() {
                write!(writer, " applyNumberFormat=\"1\"")?;
            }
            write!(writer, "/>")?;
        }
        write!(
            writer,
            "</cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
             </styleSheet>"
        )?;
        Ok(())
    }
}

/// The reader every part of the package is parsed with: whitespace kept,
/// because a string cell's spaces are its own.
pub(super) fn reader<R: BufRead>(source: R) -> Reader<R> {
    let mut reader = Reader::from_reader(source);
    reader.config_mut().trim_text(false);
    reader
}
