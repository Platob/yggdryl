//! `rust/src/logging/record.rs`: one logged event, borrowed, its message
//! rendered only by whoever writes it.

use std::cell::Cell;
use std::fmt;

use yggdryl::logging::{Level, Record};

/// A message counting how many times it was rendered.
struct Counted<'a>(&'a Cell<usize>);

impl fmt::Display for Counted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.set(self.0.get() + 1);
        formatter.write_str("rendered")
    }
}

#[test]
fn a_record_borrows_its_message_and_renders_nothing_itself() {
    let renders = Cell::new(0);
    let message = Counted(&renders);
    let record = Record::new("trades.feed", Level::INFO, &message);
    assert_eq!(renders.get(), 0);
    assert_eq!(record.name(), "trades.feed");
    assert_eq!(record.level(), Level::INFO);
    assert_eq!(record.message().to_string(), "rendered");
    assert_eq!(renders.get(), 1);
}

#[test]
fn a_new_record_is_dated_now_and_located_nowhere() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_nanos();
    let record = Record::new("trades", Level::WARNING, &"late");
    let created = u128::try_from(record.created()).expect("a positive instant");
    assert!(created >= before, "{created} >= {before}");
    assert_eq!(record.target(), "trades");
    assert_eq!(record.module_path(), None);
    assert_eq!((record.file(), record.line()), (None, None));
}

#[test]
fn a_record_takes_its_location_target_and_date() {
    let record = Record::new("yggdryl.fix.build", Level::DEBUG, &"merged")
        .with_location("rust/fix/src/build.rs", 1098)
        .with_target("yggdryl_fix::build", Some("yggdryl_fix::build"))
        .with_created(1_700_000_000_000_000_000);
    assert_eq!(record.file(), Some("rust/fix/src/build.rs"));
    assert_eq!(record.line(), Some(1098));
    assert_eq!(record.target(), "yggdryl_fix::build");
    assert_eq!(record.module_path(), Some("yggdryl_fix::build"));
    assert_eq!(record.created(), 1_700_000_000_000_000_000);
    let debugged = format!("{record:?}");
    assert!(
        debugged.contains("\"merged\"") || debugged.contains("merged"),
        "{debugged}"
    );
}
