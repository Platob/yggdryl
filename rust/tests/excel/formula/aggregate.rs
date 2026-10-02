//! `rust/src/excel/formula/aggregate.rs`: ordered numeric accumulation.

#[cfg(feature = "internals")]
mod internal {
    use serde_json::Value;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;

    fn number(bits: &str) -> f64 {
        f64::from_bits(u64::from_str_radix(bits, 16).unwrap())
    }

    #[test]
    fn native_ordered_three_input_sum_matches_all_54_saved_results() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/sum_order_native.json")).unwrap();
        assert_eq!(fixture["native_cases"].as_u64(), Some(288));
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 54);
        for case in cases {
            let mut accumulator = Accumulator::default();
            let inputs = case["source_bits"].as_array().unwrap();
            assert_eq!(inputs.len(), 3);
            for input in inputs {
                accumulator
                    .push_number(number(input.as_str().unwrap()))
                    .unwrap();
            }
            let actual = accumulator.finish_sum().unwrap().unwrap().to_bits();
            let expected = number(case["expected_bits"].as_str().unwrap()).to_bits();
            assert_eq!(actual, expected, "{}", case["formula"].as_str().unwrap());
        }
    }

    #[test]
    fn empty_singleton_and_order_are_explicit() {
        assert_eq!(
            Accumulator::default()
                .finish_sum()
                .unwrap()
                .unwrap()
                .to_bits(),
            0
        );
        let mut one = Accumulator::default();
        one.push_number(7.25).unwrap();
        assert_eq!(
            one.finish_sum().unwrap().unwrap().to_bits(),
            7.25_f64.to_bits()
        );

        let n = -f64::from_bits(0x3fef_ffff_ffff_fffe);
        let tiny = f64::EPSILON;
        let mut natural = Accumulator::default();
        for value in [1.0, n, tiny] {
            natural.push_number(value).unwrap();
        }
        let mut permuted = Accumulator::default();
        for value in [1.0, tiny, n] {
            permuted.push_number(value).unwrap();
        }
        assert_eq!(
            natural.finish_sum().unwrap().unwrap().to_bits(),
            (2.0 * tiny).to_bits()
        );
        assert_eq!(permuted.finish_sum().unwrap().unwrap().to_bits(), 0);
    }

    #[test]
    fn a_refused_number_does_not_publish_a_partial_push() {
        use yggdryl::excel::ExcelError;

        let mut accumulator = Accumulator::default();
        accumulator.push_number(2.0).unwrap();
        assert_eq!(accumulator.push_number(f64::INFINITY), Err(ExcelError::Num));
        accumulator.push_number(3.0).unwrap();
        assert_eq!(
            accumulator.finish_sum().unwrap().unwrap().to_bits(),
            5.0_f64.to_bits()
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn native_subnormal_transport_requires_an_uncomputed_sum_boundary() {
    use yggdryl::excel::ExcelError;
    use yggdryl::internals::excel_formula_aggregate::Accumulator;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/sum_edges_native.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 40);
    assert_eq!(
        fixture["input_sha256"],
        "e8cde983212d3129fc47d6464dc4fecf4bf02cfc7ab80e4b8551a281178b5b83"
    );
    let native = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["formula"] == "SUM(C2,D2,E2)")
        .unwrap();
    assert_eq!(native["answer"], "0008000000000000");
    assert_eq!(native["cache"], "0");
    let mut sum = Accumulator::default();
    for bits in [
        0x0010_0000_0000_0000,
        0x8018_0000_0000_0000,
        0x0010_0000_0000_0000,
    ] {
        sum.push_number(f64::from_bits(bits)).unwrap();
    }
    assert_eq!(sum.finish_sum(), Ok(None));

    let mut ordinary = Accumulator::default();
    for value in [1.0, -(1.0 - f64::EPSILON), f64::EPSILON, 0.0] {
        ordinary.push_number(value).unwrap();
    }
    assert_eq!(ordinary.finish_sum(), Ok(Some(2.0 * f64::EPSILON)));

    let mut overflow = Accumulator::default();
    overflow.push_number(9e307).unwrap();
    overflow.push_number(9e307).unwrap();
    assert_eq!(overflow.finish_sum(), Err(ExcelError::Num));
}
