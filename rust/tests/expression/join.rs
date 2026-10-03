//! `rust/src/expression/join.rs`: the keys two sides are joined on - one
//! pair of terms per key, the spellings every join verb reads its `by`
//! through, and the text each prints back as.

use yggdryl::Scalar;
use yggdryl::expression::{IntoJoinKeys, JoinKey, JoinKeys, Selector, Term};

fn term(text: &str) -> Term {
    text.parse().unwrap()
}

// ---------------------------------------------------------------------------
// One key
// ---------------------------------------------------------------------------

#[test]
fn a_key_is_a_left_term_against_a_right_term_or_one_term_over_both() {
    let pair = JoinKey::new(term("venue"), term("mic"));
    assert_eq!(pair.left(), &term("venue"));
    assert_eq!(pair.right(), &term("mic"));
    assert!(!pair.is_using());
    assert_eq!(pair.using_column(), None);

    let using = JoinKey::using(Term::column("id"));
    assert!(using.is_using());
    assert_eq!(using.left(), using.right());
    assert_eq!(using.using_column(), Some("id"));
    // A pair of one bare column is the same key `using` spells.
    assert_eq!(JoinKey::new(term("id"), term("id")), using);

    // A shared term that computes is a `using` key with no column to
    // coalesce.
    let computed = JoinKey::using(term("lower(venue)"));
    assert!(computed.is_using());
    assert_eq!(computed.using_column(), None);
    // A path is not a bare column either.
    assert_eq!(JoinKey::using(term("trade.id")).using_column(), None);
}

#[test]
fn an_equality_splits_into_its_two_sides_and_any_other_term_is_shared() {
    let pair: JoinKey = "lower(venue) = mic".parse().unwrap();
    assert_eq!(pair, JoinKey::new(term("lower(venue)"), term("mic")));
    let shared: JoinKey = "id".parse().unwrap();
    assert_eq!(shared, JoinKey::using(term("id")));
    // An `or` is one term over both sides, not two keys.
    let shared: JoinKey = "a or b".parse().unwrap();
    assert_eq!(shared, JoinKey::using(term("a or b")));
    // Only an equality splits: another comparison is refused naming it.
    for (text, named) in [("a > b", "`>`"), ("a <> b", "`<>`"), ("a <= b", "`<=`")] {
        let error = text.parse::<JoinKey>().unwrap_err().to_string();
        assert!(error.contains(named), "{text}: {error}");
        assert!(error.contains("equality"), "{text}: {error}");
    }
    // One key is one key.
    let error = "a, b".parse::<JoinKey>().unwrap_err().to_string();
    assert!(error.contains("expected one join key, got 2"), "{error}");
    assert!("a =".parse::<JoinKey>().is_err());
}

#[test]
fn a_key_is_read_from_the_scalar_that_spells_it() {
    assert_eq!(
        JoinKey::from_scalar(&Scalar::from("venue = mic")).unwrap(),
        JoinKey::new(term("venue"), term("mic"))
    );
    let pair = Scalar::from_sequence([Scalar::from("lower(venue)"), Scalar::from("mic")]);
    assert_eq!(
        JoinKey::from_scalar(&pair).unwrap(),
        JoinKey::new(term("lower(venue)"), term("mic"))
    );
    for refused in [
        Scalar::from(1_i64),
        Scalar::from_sequence([Scalar::from("a")]),
        Scalar::from_sequence([Scalar::from("a"), Scalar::from("b"), Scalar::from("c")]),
    ] {
        let error = JoinKey::from_scalar(&refused).unwrap_err().to_string();
        assert!(error.contains("[left, right]"), "{refused:?}: {error}");
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn a_key_list_prints_the_text_it_reads_back_as() {
    for text in [
        "id",
        "id, venue = mic",
        "lower(venue) = mic, (a or b) = flag",
        "trade.legs[0].ccy = ccy",
    ] {
        let keys: JoinKeys = text.parse().unwrap();
        assert_eq!(keys.to_string(), text);
        assert_eq!(keys.to_string().parse::<JoinKeys>().unwrap(), keys);
    }
    // A pair of one column prints as the shared column it is.
    let keys: JoinKeys = "id = id, venue = venue".parse().unwrap();
    assert_eq!(keys.to_string(), "id, venue");
    // A shared term that is itself an equality prints as the pair, since
    // bare it would read back as two terms.
    let shared = JoinKey::using(term("a = b"));
    assert_eq!(shared.to_string(), "(a = b) = (a = b)");
    let keys = JoinKeys::new([shared.clone()]);
    assert_eq!(keys.to_string(), "(a = b) = (a = b)");
    assert_eq!(keys.to_string().parse::<JoinKeys>().unwrap(), keys);
    assert_eq!(shared.to_string().parse::<JoinKey>().unwrap(), shared);
    // Malformed lists are parse errors with a position.
    for refused in ["", "id,", ", id", "id venue"] {
        let error = refused.parse::<JoinKeys>().unwrap_err().to_string();
        assert!(error.contains("at byte "), "{refused:?}: {error}");
    }
}

// ---------------------------------------------------------------------------
// Many keys
// ---------------------------------------------------------------------------

#[test]
fn a_key_list_lends_its_keys_in_order() {
    let keys = JoinKeys::new([JoinKey::using(term("id")), "venue = mic".parse().unwrap()]);
    assert_eq!(keys.len(), 2);
    assert!(!keys.is_empty());
    assert!(JoinKeys::default().is_empty());
    assert_eq!(keys.keys()[1].right(), &term("mic"));
    let lent: Vec<String> = keys.iter().map(ToString::to_string).collect();
    assert_eq!(lent, ["id", "venue = mic"]);
    let borrowed: Vec<&JoinKey> = (&keys).into_iter().collect();
    assert_eq!(borrowed.len(), 2);
    let owned: Vec<JoinKey> = keys.clone().into_iter().collect();
    assert_eq!(JoinKeys::new(owned), keys);
    // The document is the list of `{left, right}` pairs.
    let document = serde_json::to_value(&keys).unwrap();
    assert_eq!(document.as_array().unwrap().len(), 2);
    assert!(document[1]["left"].is_object(), "{document}");
    assert_eq!(serde_json::from_value::<JoinKeys>(document).unwrap(), keys);
}

#[test]
fn a_key_list_is_read_from_every_scalar_shape_that_spells_one() {
    let expected: JoinKeys = "id, venue = mic".parse().unwrap();
    assert_eq!(
        JoinKeys::from_scalar(&Scalar::from("id, venue = mic")).unwrap(),
        expected
    );
    let texts = Scalar::from_sequence([Scalar::from("id"), Scalar::from("venue = mic")]);
    assert_eq!(JoinKeys::from_scalar(&texts).unwrap(), expected);
    let pairs = Scalar::from_sequence([
        Scalar::from("id"),
        Scalar::from_sequence([Scalar::from("venue"), Scalar::from("mic")]),
    ]);
    assert_eq!(JoinKeys::from_scalar(&pairs).unwrap(), expected);
    // A mapping keeps its order; a record is sorted by its left terms.
    let mapping = Scalar::from_mapping([
        (Scalar::from("venue"), Scalar::from("mic")),
        (Scalar::from("id"), Scalar::from("id")),
    ])
    .unwrap();
    assert_eq!(
        JoinKeys::from_scalar(&mapping).unwrap().to_string(),
        "venue = mic, id"
    );
    let record =
        Scalar::from_struct([("venue", Scalar::from("mic")), ("id", Scalar::from("id"))]).unwrap();
    assert_eq!(JoinKeys::from_scalar(&record).unwrap(), expected);
    let error = JoinKeys::from_scalar(&Scalar::from(1_i64))
        .unwrap_err()
        .to_string();
    assert!(error.contains("join key list"), "{error}");
}

#[test]
fn every_into_join_keys_form_resolves_to_the_same_keys() {
    let expected: JoinKeys = "id, venue = mic".parse().unwrap();
    let keys = || expected.keys().to_vec();
    let text = String::from("id, venue = mic");
    let pairs = || vec![(term("id"), term("id")), (term("venue"), term("mic"))];
    let resolved: Vec<JoinKeys> = vec![
        expected.clone().into_join_keys().unwrap(),
        (&expected).into_join_keys().unwrap(),
        keys().into_join_keys().unwrap(),
        keys().as_slice().into_join_keys().unwrap(),
        [keys()[0].clone(), keys()[1].clone()]
            .into_join_keys()
            .unwrap(),
        "id, venue = mic".into_join_keys().unwrap(),
        text.clone().into_join_keys().unwrap(),
        (&text).into_join_keys().unwrap(),
        vec!["id", "venue = mic"].into_join_keys().unwrap(),
        ["id", "venue = mic"].as_slice().into_join_keys().unwrap(),
        ["id", "venue = mic"].into_join_keys().unwrap(),
        vec![String::from("id"), String::from("venue = mic")]
            .into_join_keys()
            .unwrap(),
        [String::from("id"), String::from("venue = mic")]
            .into_join_keys()
            .unwrap(),
        pairs().into_join_keys().unwrap(),
        <[(Term, Term); 2]>::try_from(pairs())
            .unwrap()
            .into_join_keys()
            .unwrap(),
        (
            "id, venue".parse::<Selector>().unwrap(),
            "id, mic".parse::<Selector>().unwrap(),
        )
            .into_join_keys()
            .unwrap(),
        (&Scalar::from("id, venue = mic")).into_join_keys().unwrap(),
        Scalar::from("id, venue = mic").into_join_keys().unwrap(),
    ];
    for (index, keys) in resolved.iter().enumerate() {
        assert_eq!(keys, &expected, "form {index}");
    }
    // One key is a list of one.
    assert_eq!(
        JoinKey::using(term("id")).into_join_keys().unwrap(),
        "id".parse::<JoinKeys>().unwrap()
    );
}

#[test]
fn a_selector_pair_is_refused_where_it_cannot_be_one_key_per_row() {
    let left: Selector = "id, venue".parse().unwrap();
    let right: Selector = "id".parse().unwrap();
    let error = (&left, &right).into_join_keys().unwrap_err().to_string();
    assert!(
        error.contains("expected as many left keys as right keys, got 2 and 1"),
        "{error}"
    );
    let unnested: Selector = "unnest(xs)".parse().unwrap();
    let error = (&unnested, &unnested)
        .into_join_keys()
        .unwrap_err()
        .to_string();
    assert!(error.contains("join key"), "{error}");
    // Text that is not a key list is a parse error at every text form.
    assert!("a >".into_join_keys().is_err());
    assert!(vec!["id", "a >"].into_join_keys().is_err());
}
