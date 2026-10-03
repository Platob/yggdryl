//! `rust/src/logging/facade.rs`: the tree as the `log` facade's backend -
//! targets read as dotted logger names, the ceiling following the tree,
//! the foreign floor.

use yggdryl::logging::{self, Level, get_logger};

use crate::support::{Collect, serial};

#[test]
fn installing_twice_is_one_installation() {
    let _tree = serial();
    logging::install().expect("the tree");
    logging::install().expect("the same tree");
    assert!(logging::is_installed());
}

#[test]
fn a_facade_record_reaches_the_logger_its_target_names() {
    let _tree = serial();
    logging::install().expect("the tree");
    let feed = get_logger("facade.feed");
    feed.set_level(Level::DEBUG);
    let collect = Collect::shared();
    feed.add_handler(collect.clone());

    let line = line!() + 1;
    log::debug!(target: "facade::feed::orders", "{} orders", 3);
    log::trace!(target: "facade::feed::orders", "below the logger's level");
    let [kept] = collect.take().try_into().expect("one record");
    assert_eq!(kept.name, "facade.feed.orders");
    assert_eq!(kept.target, "facade::feed::orders");
    assert_eq!(kept.level, Level::DEBUG);
    assert_eq!(kept.line, "3 orders");
    assert_eq!(kept.file.as_deref(), Some(file!()));
    assert_eq!(kept.at, Some(line));
    assert!(log::log_enabled!(target: "facade::feed", log::Level::Debug));
    assert!(!log::log_enabled!(target: "facade::feed", log::Level::Trace));
}

#[test]
fn the_ceiling_is_the_most_verbose_level_any_logger_handles() {
    // Other tests leave their own loggers at their own levels, so each
    // assertion here holds whatever the rest of the tree states.
    let _tree = serial();
    logging::install().expect("the tree");
    let verbose = get_logger("facade.ceiling");
    verbose.set_level(Level::TRACE);
    assert_eq!(
        log::max_level(),
        log::LevelFilter::Trace,
        "nothing is below TRACE"
    );
    logging::disable(Level::INFO);
    assert_eq!(
        log::max_level(),
        log::LevelFilter::Warn,
        "disable clamps every logger"
    );
    logging::disable(Level::CRITICAL);
    assert_eq!(log::max_level(), log::LevelFilter::Off);
    logging::disable(Level::NOTSET);
    verbose.set_disabled(true);
    verbose.set_level(Level::NOTSET);
    assert!(
        log::max_level() >= log::LevelFilter::Warn,
        "the root's WARNING at least"
    );
    verbose.set_disabled(false);
}

#[test]
fn the_foreign_floor_holds_back_what_lies_outside_this_crate() {
    let _tree = serial();
    logging::install().expect("the tree");
    get_logger("").set_level(Level::DEBUG);
    let collect = Collect::shared();
    get_logger("").add_handler(collect.clone());
    logging::set_foreign_level(Level::WARNING);
    assert_eq!(logging::foreign_level(), Level::WARNING);

    log::debug!(target: "dependency::client", "a request");
    log::warn!(target: "dependency::client", "a retry");
    log::debug!(target: "yggdryl::iceberg::table", "a scan");
    log::debug!(target: "yggdryl", "the crate root");
    log::debug!(target: "yggdryl_cli::shell", "a neighbour's crate");
    let names: Vec<String> = collect.take().into_iter().map(|kept| kept.name).collect();
    assert_eq!(
        names,
        ["dependency.client", "yggdryl.iceberg.table", "yggdryl"]
    );
    assert!(!log::log_enabled!(target: "dependency::client", log::Level::Info));
}

#[test]
fn a_disabled_logger_s_children_still_count_toward_the_ceiling() {
    let _tree = serial();
    logging::install().expect("the tree");
    let parent = get_logger("facade.disabled");
    parent.set_level(Level::TRACE);
    parent.set_disabled(true);
    assert_eq!(
        log::max_level(),
        log::LevelFilter::Trace,
        "a child not asked for yet inherits TRACE"
    );
    let collect = Collect::shared();
    parent.add_handler(collect.clone());
    log::trace!(target: "facade::disabled::child", "reached");
    assert_eq!(collect.lines(), ["reached"]);
    parent.set_disabled(false);
    parent.set_level(Level::NOTSET);
}
