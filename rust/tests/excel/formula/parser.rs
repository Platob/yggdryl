//! `rust/src/excel/formula/parser.rs`: the one grammar's arena and typed
//! host-relative references, inspected without evaluating a formula.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;

    fn at(text: &str) -> CellRef {
        text.parse().unwrap()
    }

    #[test]
    fn signs_bind_before_left_associative_power() {
        let signed = Formula::from_entry("-2^2", at("A1")).unwrap();
        let (root, nodes) = arena(&signed).unwrap();
        assert!(nodes[root].starts_with("Binary { op: Power"), "{nodes:?}");
        assert!(
            nodes
                .iter()
                .any(|node| node.starts_with("Unary { op: Negative")),
            "{nodes:?}"
        );
        let chained = Formula::from_entry("2^3^2", at("A1")).unwrap();
        let (root, nodes) = arena(&chained).unwrap();
        assert!(nodes[root].contains("left: 2,"), "{nodes:?}");
        assert!(nodes[2].starts_with("Binary { op: Power"), "{nodes:?}");
    }

    #[test]
    fn parentheses_survive_as_group_nodes() {
        let formula = Formula::from_entry("(1-1)", at("A1")).unwrap();
        let (root, nodes) = arena(&formula).unwrap();
        assert!(nodes[root].starts_with("Group("), "{nodes:?}");
    }

    #[test]
    fn entry_refs_are_relative_before_the_arena_is_cached() {
        let first = Formula::from_entry("A1+$B$2", at("C3")).unwrap();
        let second = Formula::from_entry("B2+$B$2", at("D4")).unwrap();
        assert_eq!(first, second);
        let (_, nodes) = arena(&first).unwrap();
        assert_eq!(nodes[0].matches("Relative(-2)").count(), 2, "{nodes:?}");
        assert!(nodes[1].contains("Absolute(1)"), "{nodes:?}");
    }
}

#[cfg(feature = "internals")]
#[test]
fn spaced_omitted_argument_is_an_actual_call_slot() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;

    let host: CellRef = "A1".parse().unwrap();
    for (text, expected) in [("IF(TRUE, )", 2usize), ("IF(FALSE,42, )", 3usize)] {
        let formula = Formula::from_entry(text, host).unwrap();
        let (root, nodes) = arena(&formula).unwrap();
        let args = nodes[root].split("args: [").nth(1).unwrap();
        let actual = args.matches("Some(").count() + args.matches("None").count();
        assert_eq!(actual, expected, "{text}: {:?}", nodes[root]);
    }
}

#[cfg(feature = "internals")]
#[test]
fn held_entry_drops_its_unused_arena_and_file_is_lazy() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::is_cached;

    let host: CellRef = "C3".parse().unwrap();
    let file = Formula::from_file("SUM(A1:A3)+2", host);
    assert!(
        !is_cached(&file),
        "file intake parses only when computation asks"
    );
    assert!(file.is_computed());
    assert!(is_cached(&file));

    let known = Formula::from_entry("SUM(A1:A3)+2", host).unwrap();
    assert!(is_cached(&known), "entry installs its validated tree");
    let unknown = Formula::from_entry("MYFUNC(1)", host).unwrap();
    assert!(!unknown.is_computed());
    assert!(
        !is_cached(&unknown),
        "held entry discards the unused parsed tree"
    );
}

#[cfg(feature = "internals")]
#[test]
fn entry_at_tree_matches_its_lowered_single_shape() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::{arena, entry_arena};

    let host: CellRef = "C3".parse().unwrap();
    for text in ["@A1 B2", "(@A1,B1)", "@A1:B2"] {
        let entry = entry_arena(text, host).unwrap();
        let formula = Formula::from_entry(text, host).unwrap();
        let held = arena(&formula).unwrap();
        assert_eq!(entry, held, "{text}: {}", formula.at(host));
        let (root, nodes) = entry;
        match text {
            "@A1 B2" => assert!(
                nodes[root].starts_with("Binary { op: Intersection"),
                "{text}: {nodes:?}"
            ),
            "(@A1,B1)" => {
                let child: usize = nodes[root]
                    .strip_prefix("Group(")
                    .and_then(|text| text.strip_suffix(')'))
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(
                    nodes[child].starts_with("Binary { op: Union"),
                    "{text}: {nodes:?}"
                );
            }
            "@A1:B2" => assert!(
                nodes[root].starts_with("Unary { op: ImplicitIntersection"),
                "{text}: {nodes:?}"
            ),
            _ => unreachable!(),
        }
    }
}

#[cfg(feature = "internals")]
#[test]
fn single_file_argument_boundary_is_not_an_authored_group() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;

    let host: CellRef = "C3".parse().unwrap();
    let outside = Formula::from_file("_xlfn.SINGLE(A1):B1", host);
    let (root, nodes) = arena(&outside).unwrap();
    assert!(nodes[root].starts_with("Binary { op: Range"), "{nodes:?}");
    assert!(
        nodes
            .iter()
            .any(|node| node.starts_with("Unary { op: ImplicitIntersection")),
        "{nodes:?}"
    );
    assert!(
        !nodes.iter().any(|node| node.starts_with("Group(")),
        "{nodes:?}"
    );

    let inside = Formula::from_file("_xlfn.SINGLE(A1:B1)", host);
    let (root, nodes) = arena(&inside).unwrap();
    assert!(
        nodes[root].starts_with("Unary { op: ImplicitIntersection"),
        "{nodes:?}"
    );
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert!(
        nodes[0].starts_with("Reference(") && nodes[0].contains("target: Area"),
        "{nodes:?}"
    );
    assert!(
        !nodes.iter().any(|node| node.starts_with("Group(")),
        "{nodes:?}"
    );

    let plain = Formula::from_file("_xlfn.SINGLE(A1+B1)", host);
    let (root, nodes) = arena(&plain).unwrap();
    assert!(
        nodes[root].starts_with("Unary { op: ImplicitIntersection"),
        "{nodes:?}"
    );
    assert!(
        !nodes.iter().any(|node| node.starts_with("Group(")),
        "{nodes:?}"
    );

    let authored = Formula::from_file("_xlfn.SINGLE((A1+B1))", host);
    let (root, nodes) = arena(&authored).unwrap();
    assert!(
        nodes[root].starts_with("Unary { op: ImplicitIntersection"),
        "{nodes:?}"
    );
    assert!(
        nodes.iter().any(|node| node.starts_with("Group(")),
        "{nodes:?}"
    );
}

#[cfg(feature = "internals")]
#[test]
fn mixed_prefixes_respect_each_operators_binding_power() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::{arena, entry_arena};

    let host: CellRef = "C3".parse().unwrap();
    for text in ["-@A1 B2", "+@A1 B2", "-@A1:B2"] {
        let entered = entry_arena(text, host).unwrap();
        let formula = Formula::from_entry(text, host).unwrap();
        let held = arena(&formula).unwrap();
        assert_eq!(entered, held, "{text}: {}", formula.at(host));
        let (root, nodes) = entered;
        assert!(
            nodes[root].starts_with(if text.starts_with('-') {
                "Unary { op: Negative"
            } else {
                "Unary { op: Positive"
            }),
            "{text}: {nodes:?}"
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.starts_with("Unary { op: ImplicitIntersection"))
        );
    }
}

#[test]
fn entered_unknown_calls_obey_excels_global_argument_limit() {
    use yggdryl::{
        Error,
        excel::{CellRef, Formula},
    };

    let host: CellRef = "A1".parse().unwrap();
    // Microsoft Excel specifications: arguments in a function <= 255.
    // Unknown/held functions share this syntax boundary with known calls.
    for name in ["MYSTERY", "_xlfn.LET"] {
        let valid = format!("{name}({})", vec!["0"; 255].join(","));
        assert!(Formula::from_entry(&valid, host).is_ok(), "{name}");
        let invalid = format!("{name}({})", vec!["0"; 256].join(","));
        assert!(
            matches!(
                Formula::from_entry(&invalid, host),
                Err(Error::Parse {
                    target: "formula",
                    ..
                })
            ),
            "{name}: 256 entered arguments",
        );
        let held = Formula::from_file(&invalid, host);
        assert_eq!(held.at(host).to_string(), invalid);
        assert!(!held.is_computed());
    }
}

#[cfg(feature = "internals")]
#[test]
fn literals_match_native_decimal_intake_without_changing_file_spelling() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;

    let native: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/literal_precision_excel.json")).unwrap();
    let host: CellRef = "A1".parse().unwrap();
    for case in native["literal_cases"].as_array().unwrap() {
        let text = case["wire_formula"].as_str().unwrap();
        let bits = u64::from_str_radix(case["bits"].as_str().unwrap(), 16).unwrap();
        let expected = f64::from_bits(bits);
        let entered = Formula::from_entry(text, host).unwrap();
        let file = Formula::from_file(text, host);
        for formula in [&entered, &file] {
            let (root, nodes) = arena(formula).unwrap();
            assert_eq!(
                nodes[0],
                format!("Literal(Number({:?}))", expected.abs()),
                "{text}"
            );
            assert_eq!(root, usize::from(expected.is_sign_negative()), "{text}");
            if expected.is_sign_negative() {
                assert_eq!(nodes[1], "Unary { op: Negative, value: 0 }", "{text}");
            }
        }
        assert_eq!(file.at(host).to_string(), text);
    }
    for case in native["refused_literal_cases"].as_array().unwrap() {
        let text = case["wire_formula"].as_str().unwrap();
        let expected_at = usize::from(text.starts_with('-'));
        assert!(
            matches!(Formula::from_entry(text, host), Err(yggdryl::Error::Parse {
            target: "formula", position, ..
        }) if position == expected_at),
            "{text}"
        );
        let file = Formula::from_file(text, host);
        assert_eq!(file.at(host).to_string(), text);
        assert!(!file.is_computed(), "{text}");
    }
}

#[cfg(feature = "internals")]
#[test]
fn literal_decimal_intake_keeps_leading_zero_point_and_exponent_meaning() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;

    let host: CellRef = "A1".parse().unwrap();
    for (text, expected) in [
        (".5", "0.5"),
        ("5.e-1", "0.5"),
        ("000000.50000000000000000000", "0.5"),
        ("0001.234567890123459E+2", "123.456789012345"),
        ("0.000001234567890123459e2", "0.000123456789012345"),
        ("1e-999999999999999999999999", "0"),
    ] {
        let actual = Formula::from_entry(text, host).unwrap();
        let expected = Formula::from_entry(expected, host).unwrap();
        assert_eq!(arena(&actual), arena(&expected), "{text}");
    }
    assert!(Formula::from_entry("1e999999999999999999999999", host).is_err());
}

#[test]
fn call_depth_matches_native_eligibility_and_keeps_rejected_file_text() {
    use yggdryl::{
        Error,
        excel::{CellRef, Formula},
    };

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/formula_nesting_excel.json")).unwrap();
    let host: CellRef = "A1".parse().unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let text = case["wire_formula"].as_str().unwrap();
        let accepted = case["accepted"].as_bool().unwrap();
        let file = Formula::from_file(text, host);
        assert_eq!(file.at(host).to_string(), text, "{}", case["id"]);
        assert_eq!(file.is_computed(), accepted, "{}", case["id"]);
        if accepted {
            let entered = Formula::from_entry(text, host).unwrap();
            assert!(entered.is_computed(), "{}", case["id"]);
        } else {
            let expected = case["refusal_position"].as_u64().unwrap() as usize;
            assert!(
                matches!(Formula::from_entry(text, host), Err(Error::Parse {
                target: "formula", position, ..
            }) if position == expected),
                "{}",
                case["id"]
            );
        }
    }
}

#[test]
fn call_depth_counts_entered_implicit_intersection_before_lowering() {
    use yggdryl::{
        Error,
        excel::{CellRef, Formula},
    };

    let host: CellRef = "A1".parse().unwrap();
    let abs64 = "ABS(".repeat(64) + "1" + &")".repeat(64);
    let abs65 = "ABS(".repeat(65) + "1" + &")".repeat(65);
    // An entered @ becomes one file SINGLE, including at the native maximum.
    // The independent existing group bound survives that lowering.
    for text in [
        format!("@{abs64}"),
        "ABS(".repeat(64) + "@1" + &")".repeat(64),
        "@".repeat(65) + "1",
        "@(".repeat(64) + "@1" + &")".repeat(64),
        format!("@{abs64}+@{abs64}"),
        "ABS(".repeat(64) + "(@1)" + &")".repeat(64),
    ] {
        let entered = Formula::from_entry(&text, host).unwrap();
        assert!(entered.is_computed(), "{text}");
        let wire = entered.at(host).to_string();
        let reopened = Formula::from_file(&wire, host);
        assert!(reopened.is_computed(), "{text}: {wire}");
        assert_eq!(reopened, entered, "{text}");
    }
    for (text, expected) in [
        (format!("@{abs65}"), 260),
        ("ABS(".repeat(65) + "@1" + &")".repeat(65), 260),
        ("@".repeat(66) + "1", 65),
        ("@(".repeat(64) + "@@1" + &")".repeat(64), 129),
    ] {
        assert!(
            matches!(Formula::from_entry(&text, host), Err(Error::Parse {
            target: "formula", position, ..
        }) if position == expected),
            "{text}: expected byte {expected}"
        );
    }
}

#[test]
fn call_depth_does_not_change_the_group_and_array_resource_bound() {
    use yggdryl::{
        Error,
        excel::{CellRef, Formula},
    };

    let host: CellRef = "A1".parse().unwrap();
    for inner in ["1", "{1}"] {
        let groups = if inner == "1" { 64 } else { 63 };
        let accepted = "(".repeat(groups) + inner + &")".repeat(groups);
        assert!(Formula::from_entry(&accepted, host).unwrap().is_computed());
        let rejected = "(".repeat(groups + 1) + inner + &")".repeat(groups + 1);
        assert!(matches!(
            Formula::from_entry(&rejected, host),
            Err(Error::Parse {
                target: "formula",
                position: 64,
                ..
            })
        ));
    }
}

#[cfg(feature = "internals")]
#[test]
fn strict_evaluation_policy_excludes_unsupported_and_malformed_edges() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::{strict_references, strict_root_child_count};

    let host: CellRef = "C3".parse().unwrap();
    for (text, count) in [
        ("ABS(A1)", 1),
        ("SQRT(A1)", 1),
        ("ROUND(A1,2)", 2),
        ("MOD(A1,B1)", 2),
        ("SUM(A1,B1)", 2),
        ("A1+B1", 2),
        ("A1=B1", 2),
        ("1^A1", 2),
    ] {
        let formula = Formula::from_file(text, host);
        assert_eq!(strict_root_child_count(&formula), Some(count), "{text}");
        assert_eq!(
            strict_references(&formula).unwrap().len(),
            if matches!(text, "ROUND(A1,2)" | "1^A1") {
                1
            } else {
                count
            },
            "{text}"
        );
    }
    for text in [
        "ABS(A1,B1)",
        "ROUND(A1)",
        "MOD(A1)",
        "SUM(A1,,B1)",
        "IF(FALSE,A1,0)",
        "CHOOSE(2,A1,0)",
        "IF(FALSE,1^A1,0)",
    ] {
        let formula = Formula::from_file(text, host);
        assert_eq!(strict_root_child_count(&formula), None, "{text}");
        assert!(
            strict_references(&formula).unwrap_or_default().is_empty(),
            "{text}"
        );
    }
    let nested = Formula::from_file("SUM(A1,IF(FALSE,B1,0),C1)", host);
    // SUM is strict, but the constant IF predicate contributes no B1 edge.
    assert_eq!(strict_references(&nested).unwrap().len(), 2);
}

#[cfg(feature = "internals")]
#[test]
fn lazy_selectors_policy_registers_only_the_initial_selector_input() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::{strict_references, strict_root_child_count};

    let host: CellRef = "H8".parse().unwrap();
    let references = |text: &str| strict_references(&Formula::from_file(text, host)).unwrap();
    for text in [
        "IF(A1,B1,C1)",
        "IFERROR(A1,B1)",
        "IFNA(A1,B1)",
        "CHOOSE(A1,B1,C1)",
    ] {
        assert_eq!(references(text), references("A1"), "{text}");
        assert_eq!(
            strict_root_child_count(&Formula::from_file(text, host)),
            None,
            "{text}"
        );
    }
    for text in ["IF(,B1,C1)", "IFERROR(,B1)", "IFNA(,B1)", "CHOOSE(,B1,C1)"] {
        assert!(references(text).is_empty(), "{text}");
    }
    let nested = Formula::from_file("SUM(A1,IF(B1,C1,D1),E1)", host);
    assert_eq!(strict_root_child_count(&nested), Some(3));
    assert_eq!(
        strict_references(&nested).unwrap(),
        references("SUM(A1,B1,E1)")
    );
    // An omitted result is valid; wrong arity remains held before any input read.
    for text in ["IF(A1)", "IFERROR(A1)", "IFNA(A1,B1,C1)"] {
        assert!(
            strict_references(&Formula::from_file(text, host))
                .unwrap_or_default()
                .is_empty(),
            "{text}"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn multi_selectors_policy_keeps_all_tests_and_keys_out_of_result_branches() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::strict_references;
    let host = CellRef::new(7, 7);
    let refs = |text: &str| strict_references(&Formula::from_file(text, host)).unwrap();
    for (formula, inputs) in [
        ("IFS(A1,B1,C1,D1,E1,F1)", "SUM(A1,C1,E1)"),
        ("SWITCH(A1,B1,C1,D1,E1,F1)", "SUM(A1,B1,D1)"),
        ("IFS(,B1,C1,)", "C1"),
        ("SWITCH(,B1,C1,,E1,F1)", "B1"),
    ] {
        assert_eq!(refs(formula), refs(inputs), "{formula}");
    }
    for formula in ["IFS(A1,B1,C1)", "SWITCH(A1,B1)"] {
        assert!(
            strict_references(&Formula::from_file(formula, host))
                .unwrap_or_default()
                .is_empty(),
            "{formula}"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn geometry_functions_reuse_the_typed_call_and_array_grammar() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::strict_root_child_count;
    for (text, children) in [
        ("ROW()", 0),
        ("COLUMN()", 0),
        ("ROWS(A1:A5)", 1),
        ("COLUMNS({1,2;3,4})", 1),
    ] {
        let formula = Formula::from_entry(text, CellRef::new(0, 0)).unwrap();
        assert_eq!(strict_root_child_count(&formula), Some(children), "{text}");
    }
    for text in [
        "ROW(A1,A2)",
        "COLUMN(A1,A2)",
        "ROWS()",
        "COLUMNS()",
        "ROWS({1,2;3})",
    ] {
        assert!(
            Formula::from_entry(text, CellRef::new(0, 0)).is_err(),
            "{text}"
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn reference_operator_binding_powers_preserve_typed_geometry() {
    use yggdryl::excel::{CellRef, Formula};
    use yggdryl::internals::excel_formula::arena;
    let host = CellRef::new(0, 0);
    let children = |node: &str| -> (usize, usize) {
        let left = node
            .split("left: ")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let right = node
            .split("right: ")
            .nth(1)
            .unwrap()
            .split(|ch: char| !ch.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap();
        (left, right)
    };
    let (root, nodes) = arena(&Formula::from_file("A1:B2 C2:D3", host)).unwrap();
    assert!(nodes[root].starts_with("Binary { op: Intersection"));
    let (left, right) = children(&nodes[root]);
    // The lexer owns ordinary A1:B2 area syntax as one typed Reference;
    // the parser owns the space intersection between those references.
    assert!(nodes[left].starts_with("Reference("));
    assert!(nodes[right].starts_with("Reference("));

    let (root, nodes) = arena(&Formula::from_file("A1:B2:C3:D4:E5", host)).unwrap();
    assert!(nodes[root].starts_with("Binary { op: Range"));
    let (left, _) = children(&nodes[root]);
    assert!(
        nodes[left].starts_with("Binary { op: Range"),
        "colon associates left"
    );

    for (text, intersect_left) in [("(A1 B1,C1)", true), ("(A1,B1 C1)", false)] {
        let (root, nodes) = arena(&Formula::from_file(text, host)).unwrap();
        let group = nodes[root]
            .strip_prefix("Group(")
            .unwrap()
            .strip_suffix(')')
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(nodes[group].starts_with("Binary { op: Union"), "{text}");
        let (left, right) = children(&nodes[group]);
        let intersection = if intersect_left { left } else { right };
        assert!(
            nodes[intersection].starts_with("Binary { op: Intersection"),
            "{text}"
        );
    }
}
