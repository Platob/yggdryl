//! `rust/src/logging/level.rs`: Python's numeric levels, their names and
//! the `log` levels they read.

use yggdryl::logging::Level;

#[test]
fn a_level_is_python_s_number_and_name() {
    let named = [
        (Level::NOTSET, 0, "NOTSET"),
        (Level::TRACE, 5, "TRACE"),
        (Level::DEBUG, 10, "DEBUG"),
        (Level::INFO, 20, "INFO"),
        (Level::WARNING, 30, "WARNING"),
        (Level::ERROR, 40, "ERROR"),
        (Level::CRITICAL, 50, "CRITICAL"),
    ];
    assert_eq!(Level::ALL.len(), named.len());
    for ((level, number, name), listed) in named.into_iter().zip(Level::ALL) {
        assert_eq!(level, listed);
        assert_eq!(level.get(), number);
        assert_eq!(level.name(), Some(name));
        assert_eq!(level.to_string(), name);
        assert_eq!(Level::new(number), level);
    }
    assert_eq!(Level::new(15).name(), None);
    assert_eq!(Level::new(15).to_string(), "Level 15");
    assert!(Level::DEBUG < Level::new(15) && Level::new(15) < Level::INFO);
    assert_eq!(Level::default(), Level::NOTSET);
}

#[test]
fn a_level_reads_from_any_case_python_s_aliases_and_its_number() {
    for (spelling, level) in [
        ("debug", Level::DEBUG),
        ("Info", Level::INFO),
        ("WARN", Level::WARNING),
        (" warning ", Level::WARNING),
        ("fatal", Level::CRITICAL),
        ("CRITICAL", Level::CRITICAL),
        ("trace", Level::TRACE),
        ("notset", Level::NOTSET),
        ("15", Level::new(15)),
        ("255", Level::new(255)),
    ] {
        assert_eq!(
            Level::from_str(spelling).expect(spelling),
            level,
            "{spelling}"
        );
        assert_eq!(spelling.parse::<Level>().expect(spelling), level);
    }
}

#[test]
fn anything_else_is_refused_naming_what_is_read() {
    for spelling in ["verbose", "", "256", "-1", "1.5"] {
        let error = Level::from_str(spelling).expect_err(spelling).to_string();
        assert!(error.starts_with("invalid log level expression"), "{error}");
        assert!(error.contains(&format!("{spelling:?}")), "{error}");
    }
}

#[test]
fn a_log_level_arrives_at_python_s_number() {
    assert_eq!(Level::from(log::Level::Error), Level::ERROR);
    assert_eq!(Level::from(log::Level::Warn), Level::WARNING);
    assert_eq!(Level::from(log::Level::Info), Level::INFO);
    assert_eq!(Level::from(log::Level::Debug), Level::DEBUG);
    assert_eq!(Level::from(log::Level::Trace), Level::TRACE);
}
