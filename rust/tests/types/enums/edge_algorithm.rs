//! How a geography's edges are read between two coordinates.

use yggdryl::EdgeAlgorithm;

#[test]
fn names_round_trip_case_insensitively() {
    for algorithm in EdgeAlgorithm::ALL {
        assert_eq!(
            EdgeAlgorithm::from_str(algorithm.as_str()).unwrap(),
            algorithm
        );
        assert_eq!(
            EdgeAlgorithm::from_str(&algorithm.as_str().to_uppercase()).unwrap(),
            algorithm
        );
    }
}

#[test]
fn spherical_is_the_default_both_formats_fill() {
    assert_eq!(EdgeAlgorithm::default(), EdgeAlgorithm::Spherical);
}

#[test]
fn unknown_name_reports_the_input_and_vocabulary() {
    let error = EdgeAlgorithm::from_str("euclidean").unwrap_err();
    let message = error.to_string();
    assert!(message.contains("\"euclidean\""), "{message}");
    assert!(message.contains("spherical"), "{message}");
}
