//! `rust/src/logging/formatter.rs`: Python's `%`-style formats and their
//! `strftime` dates, each parsed once and refused by byte position.

use yggdryl::Timezone;
use yggdryl::logging::{self, Formatter, Level, Record};

/// 2023-11-14 22:13:20.123456789 UTC, a Tuesday.
const CREATED: i64 = 1_700_000_000_123_456_789;

fn record<'a>(message: &'a dyn std::fmt::Display) -> Record<'a> {
    Record::new("trades.feed", Level::WARNING, message)
        .with_location("rust/src/trades/feed.rs", 42)
        .with_created(CREATED)
}

fn spelled(format: &str) -> String {
    Formatter::from_str(format)
        .unwrap_or_else(|error| panic!("{format}: {error}"))
        .format(&record(&"late fill"))
}

#[test]
fn the_default_is_the_message_and_basic_format_is_python_s() {
    assert_eq!(Formatter::default().as_str(), "%(message)s");
    assert_eq!(
        Formatter::default().format(&record(&"late fill")),
        "late fill"
    );
    assert_eq!(
        Formatter::BASIC_FORMAT,
        "%(levelname)s:%(name)s:%(message)s"
    );
    assert_eq!(
        spelled(Formatter::BASIC_FORMAT),
        "WARNING:trades.feed:late fill"
    );
}

#[test]
fn each_key_reads_the_record() {
    for (format, expected) in [
        ("%(name)s", "trades.feed"),
        ("%(levelno)d", "30"),
        ("%(levelname)s", "WARNING"),
        ("%(message)s", "late fill"),
        ("%(asctime)s", "2023-11-14 22:13:20,123"),
        ("%(created)f", "1700000000.123457"),
        ("%(msecs)d", "123"),
        ("%(pathname)s", "rust/src/trades/feed.rs"),
        ("%(filename)s", "feed.rs"),
        ("%(module)s", "feed"),
        ("%(lineno)d", "42"),
        ("%(funcName)s", "(unknown function)"),
        ("%(processName)s", "MainProcess"),
    ] {
        assert_eq!(spelled(format), expected, "{format}");
    }
    assert_eq!(spelled("%(process)d"), std::process::id().to_string());
    assert_eq!(spelled("%(msecs)s"), "123.0");
    assert!(spelled("%(relativeCreated)d").parse::<i64>().is_ok());
}

#[test]
fn a_record_saying_nowhere_reads_as_python_s_unknowns() {
    let formatter =
        Formatter::from_str("%(pathname)s|%(filename)s|%(module)s|%(lineno)d").expect("a format");
    let record = Record::new("trades", Level::INFO, &"up");
    assert_eq!(
        formatter.format(&record),
        "(unknown file)|(unknown file)|(unknown file)|0"
    );
}

#[test]
fn a_spec_pads_truncates_and_converts_as_python_does() {
    for (format, expected) in [
        ("[%(levelname)-8s]", "[WARNING ]"),
        ("[%(levelname)9s]", "[  WARNING]"),
        ("[%(levelname).4s]", "[WARN]"),
        ("[%(levelno)05d]", "[00030]"),
        ("[%(levelno)-5d]", "[30   ]"),
        ("[%(levelno)+d]", "[+30]"),
        ("[%(levelno).4d]", "[0030]"),
        ("[%(msecs)03d]", "[123]"),
        ("[%(created).3f]", "[1700000000.123]"),
        ("[%(lineno)f]", "[42.000000]"),
        ("[%(levelname)r]", "['WARNING']"),
        ("100%% %(levelname)s", "100% WARNING"),
    ] {
        assert_eq!(spelled(format), expected, "{format}");
    }
}

#[test]
fn a_repr_quotes_and_escapes_as_python_s() {
    let formatter = Formatter::from_str("%(message)r").expect("a format");
    assert_eq!(formatter.format(&record(&"it's\n")), "\"it's\\n\"");
    assert_eq!(formatter.format(&record(&"a\tb\\c")), "'a\\tb\\\\c'");
}

#[test]
fn a_format_that_cannot_spell_a_record_is_refused_where_it_fails() {
    for (format, position, reason) in [
        ("%(colour)s", 2, "expected one of name, levelno"),
        ("%(name)d", 7, "name is text"),
        ("%(levelname)x", 12, "levelname is text"),
        ("%(levelname)q", 12, "expected one of the conversions"),
        ("%(created)x", 10, "created is a fraction"),
        ("%(message)99999s", 10, "expected a width of at most 4096"),
        (
            "%(message).99999999999999999999s",
            11,
            "expected a precision of at most 4096",
        ),
        ("%(name", 0, "expected ')' closing the key"),
        ("%s", 0, "expected '(' or '%' after '%'"),
        ("%(name)", 7, "expected a conversion"),
        ("plain text", 0, "naming at least one %(key)"),
    ] {
        let error = Formatter::from_str(format).expect_err(format).to_string();
        assert!(
            error.starts_with(&format!(
                "invalid log format expression at byte {position}:"
            )),
            "{format}: {error}"
        );
        assert!(error.contains(reason), "{format}: {error}");
    }
}

#[test]
fn an_iso_week_belongs_to_the_year_its_thursday_is_in() {
    let week = |created: i64| {
        Formatter::from_str("%(asctime)s")
            .and_then(|formatter| formatter.with_datefmt("%G-W%V %U %W"))
            .expect("a date format")
            .format(&Record::new("x", Level::INFO, &"").with_created(created))
    };
    // Friday 2021-01-01: the last ISO week of 2020.
    assert_eq!(week(1_609_459_200_000_000_000), "2020-W53 00 00");
    // Monday 2024-12-30: the first ISO week of 2025.
    assert_eq!(week(1_735_516_800_000_000_000), "2025-W01 52 53");
}

#[test]
fn the_numeric_conversions_spell_as_pythons() {
    let spelled = |format: &str| spelled(format);
    for (format, expected) in [
        ("%(levelno)x %(levelno)X %(levelno)o", "1e 1E 36"),
        (
            "%(levelno)#x %(levelno)#06X %(levelno)#o",
            "0x1e 0X001E 0o36",
        ),
        ("%(lineno)c", "*"),
        ("%(created)e", "1.700000e+09"),
        ("%(created).2E", "1.70E+09"),
        ("%(created)g", "1.7e+09"),
        ("%(created).12g", "1700000000.12"),
        ("%(msecs)g %(msecs)#g", "123 123.000"),
        ("%(lineno)08.3e", "4.200e+01"),
    ] {
        assert_eq!(spelled(format), expected, "{format}");
    }
    let half_before = Record::new("x", Level::INFO, &"").with_created(-500_000_000);
    assert_eq!(
        Formatter::from_str("%(created)d|%(created)+d")
            .expect("a format")
            .format(&half_before),
        "0|+0",
        "a fraction above -1 truncates to an unsigned zero"
    );
    let wide = Formatter::from_str("%(levelname)4096s")
        .expect("the widest field")
        .format(&record(&""));
    assert_eq!(wide.len(), 4096);
    assert!(wide.ends_with(" WARNING"));
}

#[test]
fn a_date_format_spells_the_instant_with_strftime_directives() {
    let dated = |datefmt: &str| {
        Formatter::from_str("%(asctime)s")
            .and_then(|formatter| formatter.with_datefmt(datefmt))
            .unwrap_or_else(|error| panic!("{datefmt}: {error}"))
            .format(&record(&""))
    };
    for (datefmt, expected) in [
        ("%Y-%m-%dT%H:%M:%S.%fZ", "2023-11-14T22:13:20.123456Z"),
        ("%F %T", "2023-11-14 22:13:20"),
        ("%D %R", "11/14/23 22:13"),
        ("%a %A %b %B %h", "Tue Tuesday Nov November Nov"),
        ("%I %p %e %j %C %y", "10 PM 14 318 20 23"),
        ("%u %w %s %z %Z %%", "2 2 1700000000 +0000 UTC %"),
        ("%U %W %V %G %g", "46 46 46 2023 23"),
        ("%c", "Tue Nov 14 22:13:20 2023"),
        ("%x %X", "11/14/23 22:13:20"),
        ("%k %l %P %r", "22 10 pm 10:13:20 PM"),
        ("%H%n%M%t%S", "22\n13\t20"),
    ] {
        assert_eq!(dated(datefmt), expected, "{datefmt}");
    }
    let error = Formatter::default()
        .with_datefmt("%Y week %Q")
        .expect_err("no such directive")
        .to_string();
    assert!(
        error.starts_with("invalid log date format expression at byte 8:"),
        "{error}"
    );
}

#[test]
fn a_zone_moves_the_clock_and_names_itself() {
    let paris = Timezone::from_str("Europe/Paris").expect("a zone");
    let formatter = Formatter::from_str("%(asctime)s")
        .and_then(|formatter| formatter.with_datefmt("%H:%M %z %Z"))
        .expect("a format")
        .with_timezone(paris);
    assert_eq!(formatter.timezone(), paris);
    assert_eq!(formatter.format(&record(&"")), "23:13 +0100 CET");
    assert_eq!(formatter.datefmt(), Some("%H:%M %z %Z"));
    let default = Formatter::from_str("%(asctime)s")
        .expect("a format")
        .with_timezone(paris);
    assert_eq!(default.format(&record(&"")), "2023-11-14 23:13:20,123");
}

#[test]
fn the_thread_keys_read_the_emitting_thread() {
    let formatter = Formatter::from_str("%(threadName)s %(thread)d").expect("a format");
    let here = formatter.format(&record(&""));
    assert_eq!(
        here,
        formatter.format(&record(&"")),
        "one thread, one number"
    );
    let there = std::thread::Builder::new()
        .name("feed-worker".to_owned())
        .spawn(move || formatter.format(&record(&"")))
        .expect("a thread")
        .join()
        .expect("a joined thread");
    assert!(there.starts_with("feed-worker "), "{there}");
    assert_ne!(
        here.rsplit(' ').next(),
        there.rsplit(' ').next(),
        "two threads, two numbers"
    );
}

#[test]
fn a_thread_set_thread_name_named_reads_that_name_and_an_unnamed_one_its_number() {
    let formatter = Formatter::from_str("%(threadName)s|%(thread)d").expect("a format");
    let (unnamed, named, forgotten, stated) = std::thread::spawn(move || {
        let unnamed = formatter.format(&record(&""));
        logging::set_thread_name("ingest");
        let named = formatter.format(&record(&""));
        let stated = formatter.format(&record(&"").with_thread("MainThread"));
        logging::set_thread_name("");
        (unnamed, named, formatter.format(&record(&"")), stated)
    })
    .join()
    .expect("a joined thread");
    let (name, number) = unnamed.split_once('|').expect("two keys");
    assert_eq!(
        name,
        format!("Thread-{number}"),
        "Python's spelling of an unnamed thread"
    );
    assert_eq!(named, format!("ingest|{number}"));
    assert_eq!(forgotten, unnamed, "an empty name forgets the one set");
    assert_eq!(
        stated,
        format!("MainThread|{number}"),
        "a record's own thread wins"
    );
}

#[test]
fn formatters_compare_by_what_they_spell() {
    let one = Formatter::from_str("%(message)s").expect("a format");
    assert_eq!(one, Formatter::default());
    assert_ne!(one.clone().with_timezone(Timezone::NAIVE), one);
    assert_eq!(one.to_string(), "%(message)s");
}

#[test]
fn the_terminal_format_spells_time_level_thread_logger_caller_and_message() {
    let terminal = Formatter::terminal();
    assert_eq!(terminal.as_str(), Formatter::TERMINAL_FORMAT);
    assert_eq!(terminal.datefmt(), None, "Python's asctime");
    for (level, glyph) in [
        (Level::TRACE, "·"),
        (Level::DEBUG, "·"),
        (Level::INFO, "•"),
        (Level::new(25), "•"),
        (Level::WARNING, "!"),
        (Level::ERROR, "✗"),
        (Level::CRITICAL, "‼"),
    ] {
        assert_eq!(level.glyph(), glyph, "{level}");
    }
    let record = Record::new("trades.feed", Level::INFO, &"opened 3 venues")
        .with_location("src/feed.rs", 42)
        .with_function("open")
        .with_thread("main")
        .with_created(CREATED);
    assert_eq!(
        terminal.format(&record),
        "2023-11-14 22:13:20,123 • INFO     [main] trades.feed open:42 › opened 3 venues"
    );
}

#[test]
fn the_caller_is_the_function_else_the_module_else_the_file() {
    let caller = Formatter::from_str("%(caller)s").expect("a format");
    let base = Record::new("trades", Level::INFO, &"");
    let spelled = |record: Record<'_>| caller.format(&record);
    assert_eq!(spelled(base), "-");
    assert_eq!(spelled(base.with_location("src/feed.rs", 9)), "feed:9");
    assert_eq!(
        spelled(
            base.with_location("rust/src/iceberg/table.rs", 227)
                .with_target("yggdryl::iceberg::table", Some("yggdryl::iceberg::table"))
        ),
        "table:227"
    );
    assert_eq!(
        spelled(base.with_location("src/feed.rs", 9).with_function("open")),
        "open:9"
    );
    assert_eq!(spelled(base.with_function("open")), "open");
    let python = Formatter::from_str("%(funcName)s %(threadName)s").expect("a format");
    assert_eq!(
        python.format(&base.with_function("open").with_thread("MainThread")),
        "open MainThread"
    );
}

#[test]
fn a_colored_line_spells_the_styles_and_a_plain_one_none() {
    let record = record(&"late fill").with_thread("main");
    let mut colored = String::new();
    Formatter::terminal().format_colored_into(&record, &mut colored);
    assert_eq!(
        colored,
        "\x1b[2m2023-11-14 22:13:20,123\x1b[0m \x1b[33m! WARNING \x1b[0m \x1b[2m[main]\x1b[0m \x1b[1mtrades.feed\x1b[0m \x1b[2mfeed:42 ›\x1b[0m late fill"
    );
    let custom = Formatter::from_str("%(levelcolor)s%(levelglyph)s%(reset)s %(dim)s%(name)s%(reset)s %(bold)s%(message)s%(reset)s")
        .expect("a format");
    assert_eq!(custom.format(&record), "! trades.feed late fill");
    let mut styled = String::new();
    custom.format_colored_into(&Record::new("x", Level::CRITICAL, &"down"), &mut styled);
    assert_eq!(
        styled,
        "\x1b[1;31m‼\x1b[0m \x1b[2mx\x1b[0m \x1b[1mdown\x1b[0m"
    );
}

#[test]
fn a_multi_line_message_hangs_under_its_first_line_in_the_terminal_format() {
    let terminal = Formatter::terminal();
    let ended = Formatter::terminal().format(&record(&"crossed\n").with_thread("main"));
    assert!(
        ended.ends_with("› crossed\n"),
        "no indent owed to an empty last line: {ended:?}"
    );
    let record = record(&"crossed\nat 101.5\nby 2 lots").with_thread("main");
    let plain = terminal.format(&record);
    let mut lines = plain.lines();
    let first = lines.next().expect("a first line");
    assert_eq!(
        first,
        "2023-11-14 22:13:20,123 ! WARNING  [main] trades.feed feed:42 › crossed"
    );
    let column = first[..first.find("crossed").expect("the message")]
        .chars()
        .count();
    for (line, text) in lines.zip(["at 101.5", "by 2 lots"]) {
        assert_eq!(line, format!("{}{text}", " ".repeat(column)));
    }
    let mut colored = String::new();
    terminal.format_colored_into(&record, &mut colored);
    assert!(
        colored.contains(&format!("\n{}at 101.5", " ".repeat(column))),
        "escape sequences take no column: {colored:?}"
    );
    let python = Formatter::from_str("%(levelname)s %(message)s").expect("a format");
    assert_eq!(
        python.format(&record),
        "WARNING crossed\nat 101.5\nby 2 lots",
        "Python's own formats never hang"
    );
}

#[test]
fn ascii_escapes_past_ascii_and_repr_escapes_what_python_does_not_print() {
    let ascii = Formatter::from_str("%(message)a").expect("a format");
    let repr = Formatter::from_str("%(message)r").expect("a format");
    assert_eq!(ascii.format(&record(&"café €")), "'caf\\xe9 \\u20ac'");
    assert_eq!(repr.format(&record(&"café €")), "'café €'");
    assert_eq!(
        repr.format(&record(&"a\u{200b}b\u{a0}c\u{1f}")),
        "'a\\u200bb\\xa0c\\x1f'"
    );
    assert_eq!(ascii.format(&record(&"🦀")), "'\\U0001f980'");
}

#[test]
fn an_empty_date_format_is_none_as_in_python() {
    let formatter = Formatter::from_str("%(asctime)s")
        .and_then(|formatter| formatter.with_datefmt(""))
        .expect("a format");
    assert_eq!(formatter.datefmt(), None);
    assert_eq!(formatter.format(&record(&"")), "2023-11-14 22:13:20,123");
}
