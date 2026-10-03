//! `rust/src/logging/host.rs`: a runtime's own logging mirrored by the
//! tree - its thresholds asked once per change, its records handed over.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use yggdryl::logging::{self, Host, Level, Record, get_logger};

use crate::support::{Collect, serial};

/// A host whose levels the test states, counting what the tree asks.
#[derive(Default)]
struct Runtime {
    levels: Mutex<HashMap<String, Level>>,
    asked: AtomicUsize,
    handled: Mutex<Vec<(String, Level, String)>>,
}

impl Runtime {
    fn set(&self, name: &str, level: Level) {
        self.levels
            .lock()
            .expect("the levels")
            .insert(name.to_owned(), level);
        logging::invalidate();
    }

    fn handled(&self) -> Vec<(String, Level, String)> {
        std::mem::take(&mut *self.handled.lock().expect("the records"))
    }
}

impl Host for Runtime {
    fn threshold(&self, name: &str) -> Option<Level> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        let levels = self.levels.lock().expect("the levels");
        let mut name = name;
        loop {
            if let Some(level) = levels.get(name) {
                return Some(*level);
            }
            match name.rsplit_once('.') {
                Some((parent, _)) => name = parent,
                None => return levels.get("root").copied(),
            }
        }
    }

    fn lowest_threshold(&self) -> Option<Level> {
        self.levels
            .lock()
            .expect("the levels")
            .values()
            .min()
            .copied()
    }

    fn handle(&self, record: &Record<'_>) {
        self.handled.lock().expect("the records").push((
            record.name().to_owned(),
            record.level(),
            record.message().to_string(),
        ));
    }
}

#[test]
fn a_host_logger_of_the_same_name_takes_what_its_level_admits() {
    let _tree = serial();
    let runtime = Arc::new(Runtime::default());
    runtime.set("root", Level::WARNING);
    runtime.set("host.feed", Level::DEBUG);
    logging::set_host(Some(runtime.clone()));
    assert!(logging::has_host());

    let feed = get_logger("host.feed.orders");
    assert_eq!(
        feed.effective_level(),
        Level::WARNING,
        "the tree's own level"
    );
    assert!(feed.is_enabled_for(Level::DEBUG), "the host's level");
    feed.debug("sent");
    get_logger("host.other").info("not the host's level");
    assert_eq!(
        runtime.handled(),
        [(
            "host.feed.orders".to_owned(),
            Level::DEBUG,
            "sent".to_owned()
        )]
    );
}

#[test]
fn a_threshold_is_asked_once_per_change_of_the_host_s_levels() {
    let _tree = serial();
    let runtime = Arc::new(Runtime::default());
    runtime.set("root", Level::INFO);
    logging::set_host(Some(runtime.clone()));
    let logger = get_logger("host.cached");
    let asked = runtime.asked.load(Ordering::Relaxed);
    for _ in 0..100 {
        logger.debug("below");
        logger.info("at");
    }
    assert_eq!(
        runtime.asked.load(Ordering::Relaxed),
        asked + 1,
        "one question"
    );
    assert_eq!(runtime.handled().len(), 100);

    runtime.set("host.cached", Level::DEBUG);
    logger.debug("now handled");
    assert_eq!(
        runtime.asked.load(Ordering::Relaxed),
        asked + 2,
        "asked again once"
    );
    assert_eq!(
        runtime.handled(),
        [(
            "host.cached".to_owned(),
            Level::DEBUG,
            "now handled".to_owned()
        )]
    );
}

#[test]
fn under_a_host_the_tree_s_handlers_take_records_too_and_the_last_resort_is_silent() {
    let _tree = serial();
    let last = Collect::shared();
    logging::set_last_resort(Some(last.clone()));
    let runtime = Arc::new(Runtime::default());
    runtime.set("root", Level::WARNING);
    logging::set_host(Some(runtime.clone()));

    let logger = get_logger("host.both");
    logger.error("to the host alone");
    assert!(last.lines().is_empty(), "the host has its own last resort");

    let collect = Collect::shared();
    logger.add_handler(collect.clone());
    logger.set_level(Level::DEBUG);
    logger.debug("to the tree alone");
    logger.error("to both");
    assert_eq!(collect.lines(), ["to the tree alone", "to both"]);
    let handled: Vec<String> = runtime
        .handled()
        .into_iter()
        .map(|(_, _, message)| message)
        .collect();
    assert_eq!(handled, ["to the host alone", "to both"]);

    logging::set_host(None);
    assert!(!logging::has_host());
    get_logger("host.detached").error("to the last resort");
    assert_eq!(last.lines(), ["to the last resort"]);
}

#[test]
fn a_deduplicating_logger_hands_the_host_no_repeat() {
    let _tree = serial();
    let runtime = Arc::new(Runtime::default());
    runtime.set("root", Level::WARNING);
    logging::set_host(Some(runtime.clone()));
    let logger = get_logger("host.dedup");
    logger.set_deduplicating(Some(true));
    for _ in 0..10 {
        logger.warning("stalled");
    }
    let handled: Vec<String> = runtime
        .handled()
        .into_iter()
        .map(|(_, _, message)| message)
        .collect();
    assert_eq!(handled, ["stalled", "stalled (seen 10 times)"]);
    logger.set_deduplicating(None);
}

#[test]
fn a_disabled_logger_and_the_disabled_levels_hold_on_the_hosts_side_too() {
    let _tree = serial();
    let runtime = Arc::new(Runtime::default());
    runtime.set("root", Level::DEBUG);
    logging::set_host(Some(runtime.clone()));
    let logger = get_logger("host.switched");

    logger.set_disabled(true);
    assert!(!logger.is_enabled_for(Level::CRITICAL));
    logger.critical("dropped");
    logger.handle(&Record::new(
        "host.switched",
        Level::CRITICAL,
        &"handed over",
    ));
    assert_eq!(runtime.handled(), [], "a disabled logger reaches no host");

    logger.set_disabled(false);
    logging::disable(Level::WARNING);
    assert!(!logger.is_enabled_for(Level::WARNING));
    assert!(logger.is_enabled_for(Level::ERROR));
    logger.warning("at the disabled level");
    logger.error("above it");
    logging::disable(Level::NOTSET);
    assert_eq!(
        runtime.handled(),
        [(
            "host.switched".to_owned(),
            Level::ERROR,
            "above it".to_owned()
        )]
    );
}
