//! `rust/src/logging/logger.rs`: the dotted tree, levels inherited and
//! cached, records propagated up to the last resort.

use std::sync::Arc;

use yggdryl::logging::{self, Filter, Handler, Level, NullHandler, Record, get_logger};

use crate::support::{Collect, serial};

#[test]
fn one_name_is_one_logger_and_the_root_is_named_root() {
    let _tree = serial();
    assert_eq!(get_logger("tree.one"), get_logger("tree.one"));
    assert_ne!(get_logger("tree.one"), get_logger("tree.two"));
    let root = get_logger("");
    assert_eq!(root, get_logger("root"));
    assert_eq!(root.name(), "root");
    assert_eq!(root.parent(), None);
    assert_eq!(root.level(), Level::WARNING);
}

#[test]
fn a_logger_hangs_from_its_nearest_ancestor_whenever_that_was_made() {
    let _tree = serial();
    let leaf = get_logger("tree.fixup.a.b.c");
    assert_eq!(leaf.parent(), Some(get_logger("")));
    let middle = get_logger("tree.fixup.a");
    assert_eq!(leaf.parent(), Some(middle.clone()));
    let nearer = get_logger("tree.fixup.a.b");
    assert_eq!(leaf.parent(), Some(nearer.clone()));
    assert_eq!(nearer.parent(), Some(middle.clone()));
    assert_eq!(get_logger("tree.fixup.ab").parent(), Some(get_logger("")));
    assert_eq!(middle.child("b"), nearer);
    assert_eq!(get_logger("").child("tree.fixup.a"), middle);
}

#[test]
fn a_level_is_inherited_and_a_change_applies_after_records_flowed() {
    let _tree = serial();
    let parent = get_logger("tree.inherit");
    let child = get_logger("tree.inherit.child");
    assert_eq!(child.level(), Level::NOTSET);
    assert_eq!(child.effective_level(), Level::WARNING);
    let collect = Collect::shared();
    parent.add_handler(collect.clone());

    child.info("before");
    assert!(!child.is_enabled_for(Level::INFO));
    parent.set_level(Level::DEBUG);
    assert!(
        child.is_enabled_for(Level::DEBUG),
        "the cache follows the change"
    );
    child.info("after");
    child.set_level(Level::ERROR);
    child.warning("under its own level");
    child.set_level(Level::NOTSET);
    child.debug("inherited again");
    assert_eq!(collect.lines(), ["after", "inherited again"]);
}

#[test]
fn a_record_climbs_the_propagating_chain_past_each_handler_s_level() {
    let _tree = serial();
    let top = get_logger("tree.climb");
    let leaf = get_logger("tree.climb.leaf");
    top.set_level(Level::DEBUG);
    let at_top = Collect::shared();
    let at_leaf = Collect::shared();
    at_top.set_level(Level::WARNING);
    top.add_handler(at_top.clone());
    leaf.add_handler(at_leaf.clone());

    leaf.info("fill");
    leaf.error("reject");
    assert_eq!(at_leaf.lines(), ["fill", "reject"]);
    assert_eq!(at_top.lines(), ["reject"]);

    leaf.set_propagating(false);
    assert!(!leaf.is_propagating());
    leaf.error("kept below");
    assert_eq!(at_leaf.lines(), ["kept below"]);
    assert!(at_top.lines().is_empty());
}

#[test]
fn a_handler_is_attached_once_and_removed_by_identity() {
    let _tree = serial();
    let logger = get_logger("tree.attach");
    let collect: Arc<dyn Handler> = Collect::shared();
    logger.add_handler(Arc::clone(&collect));
    logger.add_handler(Arc::clone(&collect));
    assert_eq!(logger.handlers().len(), 1);
    assert!(logger.has_handlers());
    assert!(get_logger("tree.attach.below").has_handlers());
    let other: Arc<dyn Handler> = Arc::new(NullHandler::new());
    assert!(!logger.remove_handler(&other));
    assert!(logger.remove_handler(&collect));
    assert!(logger.handlers().is_empty());
    assert!(!get_logger("tree.attach.below").has_handlers());
}

#[test]
fn a_disabled_logger_and_a_refusing_filter_drop_records() {
    let _tree = serial();
    let logger = get_logger("tree.drop");
    let collect = Collect::shared();
    logger.add_handler(collect.clone());
    logger.set_disabled(true);
    assert!(logger.is_disabled());
    assert!(!logger.is_enabled_for(Level::CRITICAL));
    logger.critical("dropped");
    logger.set_disabled(false);
    let refuse: Filter =
        Arc::new(|record: &Record<'_>| !record.message().to_string().contains("secret"));
    logger.add_filter(Arc::clone(&refuse));
    logger.error("a secret");
    logger.error("public");
    assert!(logger.remove_filter(&refuse));
    logger.error("a secret now");
    assert_eq!(collect.lines(), ["public", "a secret now"]);
}

#[test]
fn a_record_is_located_at_the_line_that_logged_it() {
    let _tree = serial();
    let logger = get_logger("tree.located");
    let collect = Collect::shared();
    logger.add_handler(collect.clone());
    let line = line!() + 1;
    logger.warning("here");
    let [kept] = collect.take().try_into().expect("one record");
    assert_eq!(kept.file.as_deref(), Some(file!()));
    assert_eq!(kept.at, Some(line));
    assert_eq!(kept.name, "tree.located");
    assert_eq!(kept.target, "tree.located");
}

#[test]
fn handle_takes_a_built_record_whatever_its_level() {
    let _tree = serial();
    let logger = get_logger("tree.handle");
    let collect = Collect::shared();
    logger.add_handler(collect.clone());
    logger.handle(&Record::new(logger.name(), Level::DEBUG, &"built"));
    assert_eq!(collect.lines(), ["built"]);
}

#[test]
fn a_record_no_handler_takes_reaches_the_last_resort_at_its_level() {
    let _tree = serial();
    let last = Collect::shared();
    last.set_level(Level::WARNING);
    logging::set_last_resort(Some(last.clone()));
    let logger = get_logger("tree.resort");
    logger.set_level(Level::DEBUG);
    logger.info("under the last resort's level");
    logger.error("said");
    assert_eq!(last.lines(), ["said"]);

    logger.add_handler(Arc::new(NullHandler::new()));
    logger.error("taken by a handler");
    assert!(last.lines().is_empty());

    logging::set_last_resort(None);
    assert!(logging::last_resort().is_none());
    get_logger("tree.resort.none").error("dropped");
}

#[test]
fn a_logger_reads_as_its_name_and_effective_level() {
    let _tree = serial();
    let logger = get_logger("tree.display");
    assert_eq!(logger.to_string(), "tree.display (WARNING)");
    assert!(format!("{logger:?}").contains("tree.display"));
}

#[test]
fn a_deduplicating_logger_says_a_record_once_then_its_tenfold_counts() {
    let _tree = serial();
    let top = get_logger("dedup.feed");
    let leaf = get_logger("dedup.feed.orders");
    top.set_level(Level::DEBUG);
    let collect = Collect::shared();
    top.add_handler(collect.clone());
    assert_eq!((top.deduplicating(), top.is_deduplicating()), (None, false));

    top.set_deduplicating(Some(true));
    assert_eq!(top.deduplicating(), Some(true));
    assert!(leaf.is_deduplicating(), "inherited");
    assert_eq!(leaf.deduplicating(), None);
    for _ in 0..100 {
        leaf.warning("late fill");
    }
    leaf.warning("late fills");
    leaf.error("late fill");
    assert_eq!(
        collect.lines(),
        [
            "late fill",
            "late fill (seen 10 times)",
            "late fill (seen 100 times)",
            "late fills",
            "late fill",
        ]
    );

    leaf.set_deduplicating(Some(false));
    assert!(!leaf.is_deduplicating(), "a nearer statement wins");
    leaf.warning("late fill");
    leaf.warning("late fill");
    assert_eq!(collect.lines().len(), 2);
    leaf.set_deduplicating(None);
    top.set_deduplicating(None);
    assert!(!leaf.is_deduplicating());
}

#[test]
fn a_repeat_reaches_neither_the_tree_s_handlers_nor_the_last_resort() {
    let _tree = serial();
    let last = Collect::shared();
    logging::set_last_resort(Some(last.clone()));
    let logger = get_logger("dedup.resort");
    logger.set_deduplicating(Some(true));
    for _ in 0..12 {
        logger.error("stalled");
    }
    assert_eq!(last.lines(), ["stalled", "stalled (seen 10 times)"]);
    logger.set_deduplicating(None);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::logging_logger::nearest_logger;
    use yggdryl::logging::get_logger;

    #[test]
    fn past_the_facade_bound_a_target_finds_the_nearest_logger_and_makes_none() {
        let _tree = crate::support::serial();
        let _ = get_logger("bounded.feed");
        assert_eq!(nearest_logger("bounded.feed"), "bounded.feed");
        assert_eq!(
            nearest_logger("bounded.feed.tenant-7.orders"),
            "bounded.feed"
        );
        assert_eq!(nearest_logger("bounded.other"), "root");
        assert_eq!(
            nearest_logger("bounded.feed.tenant-7"),
            "bounded.feed",
            "the lookup above made no logger for the tenant"
        );
    }
}
