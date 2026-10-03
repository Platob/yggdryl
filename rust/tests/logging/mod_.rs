//! `rust/src/logging/mod.rs`: `basic_config`, `disable`, `shutdown` and the
//! last resort.

use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{self, BasicConfig, FileHandler, Formatter, Handler, Level, get_logger};

use crate::support::{Collect, serial};

#[test]
fn basic_config_gives_a_bare_root_its_handler_format_and_level() {
    let _tree = serial();
    let collect = Collect::bare();
    logging::basic_config(
        BasicConfig::new()
            .with_level(Level::INFO)
            .with_handler(collect.clone()),
    )
    .expect("a configuration");
    assert!(logging::is_installed());
    let root = get_logger("");
    assert_eq!(root.level(), Level::INFO);
    assert_eq!(
        collect.formatter(),
        Formatter::terminal(),
        "the modern terminal by default"
    );
    get_logger("configured.feed").info("opened");
    let [line] = collect.lines().try_into().expect("one line");
    let line_number = line!() - 2;
    assert!(
        line.ends_with(&format!(
            " • INFO     [{}] configured.feed mod_:{line_number} › opened",
            std::thread::current()
                .name()
                .expect("libtest names its threads")
        )),
        "{line:?}"
    );
    assert!(!line.contains('\x1b'), "a collector is no colour terminal");

    let ignored = Collect::shared();
    logging::basic_config(
        BasicConfig::new()
            .with_level(Level::ERROR)
            .with_handler(ignored.clone()),
    )
    .expect("a second configuration");
    assert_eq!(
        root.level(),
        Level::INFO,
        "a root with handlers is left alone"
    );
    assert_eq!(root.handlers().len(), 1);
}

#[test]
fn force_replaces_the_root_s_handlers_and_keeps_a_handler_s_own_format() {
    let _tree = serial();
    let first = Collect::bare();
    logging::basic_config(BasicConfig::new().with_handler(first.clone())).expect("a configuration");
    let second = Collect::bare();
    second.set_formatter(Formatter::from_str("[%(levelname)s] %(message)s").expect("a format"));
    logging::basic_config(
        BasicConfig::new()
            .with_force(true)
            .with_level(Level::DEBUG)
            .with_formatter(Formatter::from_str("%(name)s %(message)s").expect("a format"))
            .with_handler(second.clone()),
    )
    .expect("a forced configuration");
    let root = get_logger("");
    assert_eq!(root.handlers().len(), 1);
    get_logger("configured.forced").debug("tick");
    assert!(first.lines().is_empty());
    assert_eq!(second.lines(), ["[DEBUG] tick"]);
}

#[test]
fn disable_drops_every_record_at_or_below_its_level_until_lifted() {
    let _tree = serial();
    let logger = get_logger("disabled.feed");
    logger.set_level(Level::DEBUG);
    let collect = Collect::shared();
    logger.add_handler(collect.clone());
    logging::disable(Level::WARNING);
    logger.warning("dropped");
    logger.error("kept");
    assert!(!logger.is_enabled_for(Level::WARNING));
    logging::disable(Level::NOTSET);
    logger.warning("kept again");
    assert_eq!(collect.lines(), ["kept", "kept again"]);
}

#[test]
fn shutdown_publishes_what_a_handler_held() {
    let _tree = serial();
    let file = Arc::new(FileHandler::new(Buffer::new()).with_capacity(1 << 20));
    file.set_formatter(Formatter::default());
    let logger = get_logger("shutdown.feed");
    logger.add_handler(file.clone());
    logger.warning("held");
    assert_eq!(file.io().read_all_bytes().expect("the bytes"), b"");
    logging::shutdown();
    assert_eq!(file.io().read_all_bytes().expect("the bytes"), b"held\n");
    logger.remove_handler(&(file.clone() as Arc<dyn Handler>));
}

#[test]
fn concurrent_configurations_attach_one_handler() {
    let _tree = serial();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                logging::basic_config(BasicConfig::new().with_handler(Collect::shared()))
                    .expect("a configuration");
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a configuring thread");
    }
    assert_eq!(get_logger("").handlers().len(), 1);
}
