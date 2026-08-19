//! Timestamp-anchored record detection - the log case, with no pattern.
//!
//! A record opens where a **timestamp** opens. That is exact where a regex is
//! approximate, it yields the header span for free, and it avoids compiling and
//! running an expression against every line of a stack-trace-heavy log.
//!
//! Detection is **per line, never sniffed once**. The corpus's timestamp shape
//! is not decided from the first line and applied to the rest - the same rule
//! [`LineSep`](super::LineSep) follows, and for the same reason: rotated and
//! concatenated files are mixed.
//!
//! Non-ISO shapes - syslog's `Mar 15 10:23:45`, Apache's
//! `[10/Oct/2000:13:55:36 -0700]` - go through a
//! [`pattern`](super::TextLineOptions::set_pattern). Detection covers
//! ISO-shaped timestamps; anything else is a one-line expression rather than a
//! second sniffing heuristic per vendor format.

/// Whether `line` opens a record: a timestamp parses at its start.
///
/// The cheap guard comes first. Most lines in a stack-trace-heavy log are
/// continuations, so the common rejection is **one byte** - a plausible opener
/// is a digit, or `[` for the bracketed forms - rather than a parse attempt.
///
/// The guard is deliberately not "starts with a digit" alone: a line reading
/// `2 items processed` starts with a digit and is a continuation, and only the
/// parse can tell. The guard skips the parse for the overwhelming majority; the
/// parse decides the rest.
pub(crate) fn opens_with_timestamp(line: &[u8]) -> bool {
    let candidate = match line.first() {
        Some(byte) if byte.is_ascii_digit() => line,
        // A bracketed opener - `[2024-02-01 10:00:00]` - is the same reading
        // one byte in.
        Some(b'[') => &line[1..],
        _ => return false,
    };
    // An ISO datetime is at least `YYYY-MM-DDThh:mm:ss`; anything shorter
    // cannot be one, and rejecting on length costs no parse.
    if candidate.len() < 19 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(candidate) else {
        return false;
    };
    crate::generic::iso::parse_datetime_prefix(text).is_ok()
}

/// How far a log-mode header extends: the timestamp plus its known tokens.
///
/// In log mode there is no header *expression* - the timestamp parse is what
/// opened the record, and it yields the header's span for free. The closed
/// token table then extends that span over the conventional prefix tokens, so
/// `level`, `logger`, and `thread` land in their own columns and the message is
/// what remains.
///
/// `None` when no timestamp opens the line, which is what makes a preamble a
/// preamble.
pub(crate) fn header_extent(line: &str) -> Option<usize> {
    let bracketed = usize::from(line.starts_with('['));
    let (_, _, end) = crate::generic::iso::parse_datetime_prefix(&line[bracketed..]).ok()?;
    let mut extent = bracketed + end;
    // A bracketed timestamp closes its own bracket.
    if bracketed == 1 && line[extent..].starts_with(']') {
        extent += 1;
    }
    // Then every token the closed table recognizes, and not one byte more.
    let mut rest = &line[extent..];
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        let skipped = rest.len() - trimmed.len();
        let Some(taken) = recognized_prefix(trimmed) else {
            break;
        };
        extent += skipped + taken;
        rest = &trimmed[taken..];
    }
    Some(extent)
}

/// How many bytes of `rest` the next recognized token occupies, if any.
fn recognized_prefix(rest: &str) -> Option<usize> {
    if let Some((token, _)) = bracketed(rest) {
        // Every bracketed token is one of the three columns.
        let _ = token;
        return Some(token.len() + 2);
    }
    if let Some((token, _)) = parenthesized(rest) {
        // A parenthesized token is consumed only when it spells a level;
        // parentheses open ordinary prose far too often to claim more.
        if is_level(token) {
            return Some(token.len() + 2);
        }
    }
    let token = match rest.find([' ', '\t']) {
        Some(at) => &rest[..at],
        None => rest,
    };
    if token.is_empty() {
        return None;
    }
    if is_level(token) || key_value(token, "thread").is_some() || key_value(token, "tid").is_some()
    {
        return Some(token.len());
    }
    None
}

/// The closed table of conventional prefix tokens a header may open with.
///
/// After the timestamp, a small, **closed, documented** set of shapes is split
/// off the header so `level`, `logger`, and `thread` land in their own columns.
/// The table is exact, in the same way the capture-type inference table is
/// exact: an unrecognized token stays in the header untouched. There is no
/// scoring, no heuristic, and no "most likely" token, because this codebase
/// refuses to guess.
///
/// Recognized, in this order, each optional and each consumed at most once:
///
/// | shape | example | column |
/// | ----- | ------- | ------ |
/// | a bracketed level | `[ERROR]`, `[ee]` | `level` |
/// | a parenthesized level | `(DEBUG)`, `(WARNING)` | `level` |
/// | a bare level | `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`, `FATAL` | `level` |
/// | a bracketed id token (digits, hex, `-`, `:`, with a `:`) | `[250-e7256676:9effef3a6a:72503]` | `thread` |
/// | a bracketed non-numeric token | `[engine]` | `logger` |
/// | a bracketed all-digit token | `[42]` | `thread` |
/// | a `thread=` or `tid=` key-value | `thread=42` | `thread` |
///
/// A bracketed token is read as `logger` before `thread` only when it is not
/// all digits and not an id, so `[engine] [42]` fills both and `[42] [engine]`
/// fills both the same way - position does not decide, shape does. The id
/// shape requires a `:` **and** only `0-9a-f-:` characters, so a logger like
/// `al-iris:RiskManager` keeps its letters and stays a logger.
///
/// A parenthesized token is consumed only when it spells a level - `(DEBUG)`
/// is unambiguous where a parenthesis opening ordinary prose is not.
///
/// Returns the three column values in `LOG_COLUMNS` order.
pub fn recognized(header: &str) -> [Option<&str>; 3] {
    let mut level = None;
    let mut logger = None;
    let mut thread = None;

    // Everything after the timestamp; the timestamp itself opened the record.
    let mut rest = match crate::generic::iso::parse_datetime_prefix(header.trim_start_matches('['))
    {
        Ok((_, _, end)) => {
            let opened = usize::from(header.starts_with('['));
            let after = opened + end;
            // A bracketed timestamp closes its own bracket.
            header
                .get(after..)
                .map(|rest| rest.trim_start_matches(']'))
                .unwrap_or("")
        }
        Err(_) => header,
    }
    .trim_start_matches([' ', '\t']);

    while !rest.is_empty() {
        if let Some((token, tail)) = bracketed(rest) {
            if is_level(token) && level.is_none() {
                level = Some(token);
            } else if (token.bytes().all(|byte| byte.is_ascii_digit()) || is_id(token))
                && thread.is_none()
            {
                thread = Some(token);
            } else if logger.is_none() {
                logger = Some(token);
            } else {
                break;
            }
            rest = tail.trim_start_matches([' ', '\t']);
            continue;
        }
        if let Some((token, tail)) = parenthesized(rest) {
            // Only a level is claimed from parentheses; anything else is the
            // message's own prose, so the header stops here.
            if is_level(token) && level.is_none() {
                level = Some(token);
                rest = tail.trim_start_matches([' ', '\t']);
                continue;
            }
            break;
        }
        let (token, tail) = match rest.find([' ', '\t']) {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, ""),
        };
        if is_level(token) && level.is_none() {
            level = Some(token);
        } else if let Some(value) = key_value(token, "thread").or_else(|| key_value(token, "tid")) {
            if thread.is_none() {
                thread = Some(value);
            }
        } else {
            // Unrecognized: everything from here stays in the header untouched.
            break;
        }
        rest = tail.trim_start_matches([' ', '\t']);
    }
    [level, logger, thread]
}

/// A `[token]` at the front of `rest`, and what follows it.
fn bracketed(rest: &str) -> Option<(&str, &str)> {
    let body = rest.strip_prefix('[')?;
    let close = body.find(']')?;
    Some((&body[..close], &body[close + 1..]))
}

/// A `(token)` at the front of `rest`, and what follows it.
fn parenthesized(rest: &str) -> Option<(&str, &str)> {
    let body = rest.strip_prefix('(')?;
    let close = body.find(')')?;
    Some((&body[..close], &body[close + 1..]))
}

/// Whether `token` spells a thread-like id: digits, hex, dashes, and colons,
/// with at least one colon and one digit.
///
/// The shape of `250-e7256676:9effef3a6a:72503` - exact, in the same way the
/// level table is exact: a token with any letter outside `a-f` keeps being a
/// logger, so `al-iris:RiskManager` is never claimed.
fn is_id(token: &str) -> bool {
    token.contains(':')
        && token.bytes().any(|byte| byte.is_ascii_digit())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-' || byte == b':')
}

/// Whether `token` spells a level, in the closed set of spellings.
///
/// ASCII case-insensitive, and the two-letter forms some loggers write.
fn is_level(token: &str) -> bool {
    const LEVELS: [&str; 14] = [
        "TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "FATAL", "CRITICAL", "tt", "dd",
        "ii", "ww", "ee", "ff",
    ];
    LEVELS.iter().any(|level| level.eq_ignore_ascii_case(token))
}

/// The value of a `key=value` token, when the key matches.
fn key_value<'token>(token: &'token str, key: &str) -> Option<&'token str> {
    let (found, value) = token.split_once('=')?;
    found.eq_ignore_ascii_case(key).then_some(value)
}
