//! `rust/src/logging/terminal.rs`: the colour rule every handler and the
//! command line share, and the width a styled line takes.

use yggdryl::internals::logging_terminal::{colors, visible_width};

#[test]
fn the_color_rule_honours_the_shell_conventions() {
    assert!(colors(true, &[]), "a terminal is coloured");
    assert!(!colors(false, &[]), "a file or a pipe is not");
    assert!(!colors(true, &[("TERM", "dumb")]));
    assert!(colors(true, &[("TERM", "xterm-256color")]));
    assert!(colors(false, &[("FORCE_COLOR", "1")]));
    assert!(colors(false, &[("CLICOLOR_FORCE", "1")]));
    assert!(!colors(false, &[("FORCE_COLOR", "0")]));
    assert!(!colors(false, &[("FORCE_COLOR", "false")]));
    // The force reads the crate's one boolean table: a false spelling is off,
    // any other set text on.
    assert!(!colors(false, &[("FORCE_COLOR", "off")]));
    assert!(!colors(false, &[("CLICOLOR_FORCE", " NO ")]));
    assert!(colors(false, &[("FORCE_COLOR", "yes")]));
    assert!(colors(false, &[("FORCE_COLOR", "3")]));
    assert!(colors(
        false,
        &[("FORCE_COLOR", "0"), ("CLICOLOR_FORCE", "1")]
    ));
    assert!(!colors(true, &[("NO_COLOR", "1")]));
    assert!(
        !colors(true, &[("NO_COLOR", "1"), ("FORCE_COLOR", "1")]),
        "NO_COLOR wins"
    );
    assert!(
        colors(true, &[("NO_COLOR", "")]),
        "an empty NO_COLOR is unset"
    );
    assert!(
        colors(false, &[("FORCE_COLOR", "1"), ("TERM", "dumb")]),
        "forcing wins over dumb"
    );
}

#[test]
fn an_escape_sequence_takes_no_column() {
    assert_eq!(visible_width("plain"), 5);
    assert_eq!(visible_width("\x1b[1;31m‼\x1b[0m done"), 6);
    assert_eq!(visible_width("› • ✗"), 5);
}

#[test]
fn a_width_counts_columns_wide_characters_two_and_marks_none() {
    assert_eq!(visible_width("feed"), 4);
    assert_eq!(
        visible_width("\x1b[1mfeed\x1b[0m"),
        4,
        "styles take no column"
    );
    assert_eq!(visible_width("日本.feed"), 9, "an ideograph takes two");
    assert_eq!(
        visible_width("cafe\u{301}"),
        4,
        "a combining accent takes none"
    );
    assert_eq!(visible_width("🚀"), 2);
}
