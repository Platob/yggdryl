//! Custom medians are opt-in measurements, never an all-target test gate.

#[path = "../benchmarks/measurement.rs"]
mod measurement;

#[test]
fn custom_measurement_requires_an_optimized_criterion_benchmark_invocation() {
    let cases: &[(&[&str], bool)] = &[
        (&[], false),
        (&["--bench"], true),
        (&["--test"], false),
        (&["--list"], false),
        (&["--bench", "--test"], false),
        (&["--test", "--bench"], false),
        (&["--bench", "--list"], false),
        (&["--list", "--bench"], false),
        (&["--bench", "--test", "--list"], false),
        (&["--ignored"], false),
        (&["--bench", "--ignored"], false),
        (&["--ignored", "--bench"], false),
        (&["--profile-time", "1"], false),
        (&["--bench", "--profile-time", "1"], false),
        (&["--profile-time", "1", "--bench"], false),
        (&["--bench", "--profile-time=1"], false),
        (&["--profile-time=1", "--bench"], false),
        (&["--bench", "holder/read", "--noplot"], true),
        (&["holder/read", "--noplot"], false),
        (&["--", "--bench"], false),
        (&["--bench", "--", "--test"], true),
    ];
    for optimized in [false, true] {
        for &(arguments, benchmark_mode) in cases {
            assert_eq!(
                measurement::enabled_for(optimized, arguments),
                optimized && benchmark_mode,
                "optimized={optimized}, arguments={arguments:?}",
            );
        }
    }
}

#[test]
fn the_actual_test_invocation_does_not_enable_custom_measurement() {
    assert!(!measurement::enabled());
}
