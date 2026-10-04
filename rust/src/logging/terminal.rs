//! What a terminal shows of a record: the colour rule every handler and
//! the command line share, and the escape sequences a coloured record is
//! spelled with.

/// Turns faint text on.
pub(crate) const DIM: &str = "\x1b[2m";
/// Turns bold text on.
pub(crate) const BOLD: &str = "\x1b[1m";
/// Turns every style off.
pub(crate) const RESET: &str = "\x1b[0m";

/// Whether output to a stream that `is_terminal` answers for is written in
/// colour: `NO_COLOR` set to anything but the empty text turns colour off,
/// `FORCE_COLOR` or `CLICOLOR_FORCE` set to anything but the empty text or
/// a false spelling of the crate's one boolean table (`0`, `false`, `no`,
/// `off`) turns it on, `TERM=dumb` turns it off, and otherwise a
/// terminal is coloured and a file or a pipe is not - the conventions every
/// tool in a shell honours.
///
/// ```
/// use std::io::IsTerminal;
///
/// let colored = yggdryl::logging::is_color_enabled(std::io::stderr().is_terminal());
/// assert!(colored || !colored);
/// ```
pub fn is_color_enabled(is_terminal: bool) -> bool {
    colors(is_terminal, |name| std::env::var_os(name))
}

/// The colour rule over `variable`, the environment's lookup.
fn colors(is_terminal: bool, variable: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    let set = |name: &str| variable(name).filter(|value| !value.is_empty());
    if set("NO_COLOR").is_some() {
        return false;
    }
    let forced = |name: &str| {
        set(name).is_some_and(|value| crate::boolean::truthy_text(&value.to_string_lossy()))
    };
    if forced("FORCE_COLOR") || forced("CLICOLOR_FORCE") {
        return true;
    }
    if variable("TERM").is_some_and(|term| term == "dumb") {
        return false;
    }
    is_terminal
}

/// How many columns `text` takes on a terminal: one a character, two an
/// East Asian wide or fullwidth one or an emoji, none a combining mark, a
/// zero-width character or the escape sequences that style it.
pub(crate) fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\x1b' {
            for ended in characters.by_ref() {
                if ended.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        width += columns(character);
    }
    width
}

/// The columns one character takes: Unicode's East Asian Width read over
/// the blocks a name or a thread is spelled in, never a whole table.
const fn columns(character: char) -> usize {
    match character as u32 {
        // Combining marks, zero-width spaces and joiners, variation selectors.
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200B..=0x200F
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F => 0,
        // Hangul Jamo, the CJK blocks, Hangul syllables, the compatibility
        // ideographs and forms, the fullwidth forms, the emoji and the
        // supplementary ideographic planes.
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F7E0..=0x1F7EB
        | 0x1F90C..=0x1F9FF
        | 0x1FA70..=0x1FAFF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/logging/terminal.rs` pins and a caller cannot reach.

    /// The colour rule over the variables `environment` holds, as a
    /// process whose environment held them would decide it.
    #[must_use]
    pub fn colors(is_terminal: bool, environment: &[(&str, &str)]) -> bool {
        super::colors(is_terminal, |name| {
            environment
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        })
    }

    /// How many columns `text` takes on a terminal.
    #[must_use]
    pub fn visible_width(text: &str) -> usize {
        super::visible_width(text)
    }
}
