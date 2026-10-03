//! `rust/src/expression/plan.rs`: plans: the sections they spell, the
//! locations they name, the sequences they form, and what running one does
//! to a stream and to a store.

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use yggdryl::SortOptions;
use yggdryl::expression::{Expression, Selector, Term};
use yggdryl::expression::{IntoPlan, Location, Ordering, Plan, Source, Target, Verb, Write};
use yggdryl::{DataType, Field, Scalar, StructType, Url, Warehouse};

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn every_verb_and_its_aliases_print_one_way() {
    for (text, canonical, verb) in [
        ("insert into t from s", "insert into t from s", Verb::Insert),
        ("append to t from s", "insert into t from s", Verb::Insert),
        ("append into t from s", "insert into t from s", Verb::Insert),
        ("append t from s", "insert into t from s", Verb::Insert),
        (
            "insert overwrite t from s",
            "insert overwrite t from s",
            Verb::Overwrite,
        ),
        (
            "insert overwrite into t from s",
            "insert overwrite t from s",
            Verb::Overwrite,
        ),
        (
            "overwrite into t from s",
            "insert overwrite t from s",
            Verb::Overwrite,
        ),
        (
            "replace into t from s",
            "insert overwrite t from s",
            Verb::Overwrite,
        ),
        (
            "upsert into t by (id) from s",
            "upsert into t by (id) from s",
            Verb::Upsert,
        ),
        (
            "merge into t on (id, ts) from s",
            "upsert into t by (id, ts) from s",
            Verb::Upsert,
        ),
        (
            "merge t by (id) from s",
            "upsert into t by (id) from s",
            Verb::Upsert,
        ),
        (
            "delete from t where id = 1",
            "delete from t where id = 1",
            Verb::Delete,
        ),
        // A write with no target writes to the handle the plan is given to.
        (
            "insert select a from s",
            "insert select a from s",
            Verb::Insert,
        ),
        ("upsert by (id)", "upsert by (id)", Verb::Upsert),
        ("delete where id = 1", "delete where id = 1", Verb::Delete),
    ] {
        let plan: Plan = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(plan.to_string(), canonical, "{text}");
        assert_eq!(plan.write_section().map(Write::verb), Some(verb), "{text}");
        assert_eq!(canonical.parse::<Plan>().unwrap(), plan, "{text}");
        let expression: Expression = text.parse().unwrap();
        assert!(expression.is_plan(), "{text}");
        assert_eq!(expression.to_string(), canonical);
    }
}

#[test]
fn every_section_prints_in_the_order_it_runs() {
    let text = "create trades (id int64 not null, ccy utf8) with (comment = 'eu desk') \
                upsert into 'file:///lake/trades.parquet' by (id) \
                select id, upper(ccy) as ccy from lake.raw where price > 0 \
                order by id desc nulls first, ccy limit 10 offset 5";
    let plan: Plan = text.parse().unwrap();
    assert_eq!(plan.to_string(), text);
    assert_eq!(text.parse::<Plan>().unwrap(), plan);
    assert_eq!(plan.root_name(), "trades");
    assert_eq!(plan.root_metadata().get("comment"), Some("eu desk"));
    assert_eq!(plan.create_target().unwrap().to_string(), "trades");
    assert_eq!(
        plan.schema().unwrap().to_string(),
        "id int64 not null, ccy utf8"
    );
    assert_eq!(plan.merge_by().to_string(), "id");
    assert_eq!(plan.selector().to_string(), "id, upper(ccy) as ccy");
    assert_eq!(plan.source().unwrap().to_string(), "lake.raw");
    assert_eq!(plan.filter_section().to_string(), "price > 0");
    let keys: Vec<String> = plan.ordering().iter().map(ToString::to_string).collect();
    assert_eq!(keys, ["id desc nulls first", "ccy"]);
    assert_eq!(plan.row_limit(), Some(10));
    assert_eq!(plan.row_offset(), Some(5));
    assert_eq!(plan.columns(), ["price", "id", "ccy"]);
    assert_eq!(plan.read_columns().unwrap(), ["price", "id", "ccy"]);
    // The read sections are what a media is asked to push down.
    assert_eq!(
        plan.read_sections().to_string(),
        "select id, upper(ccy) as ccy where price > 0 order by id desc nulls first, ccy \
         limit 10 offset 5"
    );
    let document = plan.clone().into_json().unwrap();
    assert_eq!(Plan::from_json(&document).unwrap(), plan);
}

#[test]
fn a_location_keeps_its_parts_whatever_quotes_them() {
    let target = Target::parse(r#"catalog."my schema".[tbl.x].`odd-one`.2024"#).unwrap();
    assert_eq!(
        target.location(),
        &Location::parts(["catalog", "my schema", "tbl.x", "odd-one", "2024"])
    );
    assert_eq!(
        target.to_string(),
        r#"catalog."my schema"."tbl.x"."odd-one"."2024""#
    );
    assert_eq!(target.location().name(), Some("2024"));
    assert_eq!(Target::parse(&target.to_string()).unwrap(), target);

    // A URL is a location too, and is spelled as the text literal it is.
    let url: Url = "file:///lake/trades.parquet".parse().unwrap();
    let target = Target::parse("'file:///lake/trades.parquet'").unwrap();
    assert_eq!(target.location(), &Location::Url(url.clone()));
    assert_eq!(target.to_string(), "'file:///lake/trades.parquet'");
    assert_eq!(target.location().name(), Some("trades"));
    assert_eq!(target.location().url(None).unwrap(), url);

    // Parts resolve against the base they are given, and only against one.
    let parts = Location::parts(["raw", "trades"]);
    let base: Url = "file:///lake/".parse().unwrap();
    assert_eq!(
        parts.url(Some(&base)).unwrap().to_string(),
        "file:///lake/raw/trades"
    );
    let error = parts.url(None).unwrap_err().to_string();
    assert!(error.contains("raw.trades"), "{error}");
    assert!(error.contains("base"), "{error}");

    // A section word is never a bare location.
    let error = "select a from where"
        .parse::<Plan>()
        .unwrap_err()
        .to_string();
    assert!(error.contains("location"), "{error}");
    let error = "select a from [unclosed"
        .parse::<Plan>()
        .unwrap_err()
        .to_string();
    assert!(error.contains("]"), "{error}");
}

#[test]
fn a_target_carries_the_properties_it_is_opened_with() {
    let text = "select * from t with (media_type = 'text/csv', batch_row_size = '10')";
    let plan: Plan = text.parse().unwrap();
    assert_eq!(plan.to_string(), text);
    let Some(Source::Target(target)) = plan.source() else {
        panic!("expected a target source, got {:?}", plan.source());
    };
    assert_eq!(target.property("media_type"), Some("text/csv"));
    assert_eq!(target.property("batch_row_size"), Some("10"));
    assert_eq!(target.property("missing"), None);
    let built = Target::parse("t")
        .unwrap()
        .with_property("media_type", "text/csv")
        .with_property("batch_row_size", "10");
    assert_eq!(&built, target);
    // A knob that does not parse names itself.
    let broken = Target::parse("t with (batch_row_size = 'ten')").unwrap();
    let error = broken
        .knob_count::<usize>("batch_row_size")
        .unwrap_err()
        .to_string();
    assert!(error.contains("batch_row_size"), "{error}");
    assert!(error.contains("ten"), "{error}");
}

#[test]
fn a_flag_knob_reads_every_boolean_spelling_and_a_count_knob_every_integer_one() {
    let with = |name: &str, value: &str| Target::parse("t").unwrap().with_property(name, value);
    for (text, expected) in [
        ("true", true),
        ("yes", true),
        (" ON ", true),
        ("1", true),
        ("FALSE", false),
        ("n", false),
        ("off", false),
        ("0", false),
    ] {
        assert_eq!(
            with("safe", text).knob_bool("safe").unwrap(),
            Some(expected),
            "{text}"
        );
    }
    assert_eq!(Target::parse("t").unwrap().knob_bool("safe").unwrap(), None);
    let error = with("safe", "maybe")
        .knob_bool("safe")
        .unwrap_err()
        .to_string();
    assert!(error.contains("$.with.safe"), "{error}");
    assert!(error.contains("yes/no"), "{error}");
    assert!(error.contains("maybe"), "{error}");

    // A count is a trimmed, signed whole number at the width the option holds.
    for text in ["10", " 10 ", "+10"] {
        assert_eq!(
            with("batch_row_size", text)
                .knob_count::<usize>("batch_row_size")
                .unwrap(),
            Some(10),
            "{text:?}"
        );
    }
    assert_eq!(
        Target::parse("t")
            .unwrap()
            .knob_count::<u64>("max_row_size")
            .unwrap(),
        None
    );
    for text in ["ten", "1.5", "-1", "1e3", "512 MB"] {
        let error = with("max_row_size", text)
            .knob_count::<u64>("max_row_size")
            .unwrap_err()
            .to_string();
        assert!(error.contains("$.with.max_row_size"), "{text}: {error}");
        assert!(error.contains(text), "{text}: {error}");
    }
}

#[test]
fn a_sequence_is_plans_separated_by_semicolons() {
    let text = "create t (id int64 not null); insert into t from s; select id from t";
    let expression: Expression = text.parse().unwrap();
    assert!(expression.is_sequence());
    assert_eq!(expression.steps().len(), 3);
    assert_eq!(expression.to_string(), text);
    assert_eq!(text.parse::<Expression>().unwrap(), expression);
    let document = expression.clone().into_json().unwrap();
    assert_eq!(Expression::from_json(&document).unwrap(), expression);
    // One step is the step itself; a sequence with one step never exists.
    let one = Expression::sequence(["select a".parse::<Expression>().unwrap()]);
    assert!(one.is_selector());
    assert_eq!(one.steps().len(), 1);
    // A sequence is not one plan.
    let error = text.parse::<Plan>().unwrap_err().to_string();
    assert!(error.contains(';'), "{error}");
}

#[test]
fn a_plan_with_one_clause_collapses_into_it() {
    let selector = "select a, b as c"
        .parse::<Plan>()
        .unwrap()
        .into_expression();
    assert!(selector.is_selector());
    assert_eq!(selector.to_string(), "select a, b as c");
    let filter = "where a > 1".parse::<Plan>().unwrap().into_expression();
    assert!(filter.is_filter());
    assert_eq!(filter.to_string(), "where a > 1");
    let plan = "select a from t".parse::<Plan>().unwrap().into_expression();
    assert!(plan.is_plan());
    // And a clause is a plan: the one section it is.
    let from_selector = Plan::from("a, b".parse::<Selector>().unwrap());
    assert_eq!(from_selector.to_string(), "select a, b");
    assert_eq!(
        "select a, b".into_plan().unwrap().to_string(),
        "select a, b"
    );
    // Text has to say which clause it is, as an expression does.
    let error = "a, b".into_plan().unwrap_err().to_string();
    assert!(error.contains("select"), "{error}");
    assert_eq!(
        "where a > 1"
            .parse::<Expression>()
            .unwrap()
            .into_plan()
            .unwrap()
            .to_string(),
        "where a > 1"
    );
    // A whole sequence is not one plan.
    let error = "select a; select b"
        .parse::<Expression>()
        .unwrap()
        .into_plan()
        .unwrap_err();
    assert!(error.to_string().contains("sequence"), "{error}");
}

#[test]
fn a_plan_in_parentheses_is_a_source() {
    let text = "select a from (select a, b from t where b > 1) where a > 0";
    let plan: Plan = text.parse().unwrap();
    assert_eq!(plan.to_string(), text);
    let Some(Source::Plan(inner)) = plan.source() else {
        panic!("expected a nested plan, got {:?}", plan.source());
    };
    assert_eq!(inner.to_string(), "select a, b from t where b > 1");
    assert_eq!(inner.source().unwrap().to_string(), "t");
    // The nesting budget holds for plans as it does for terms.
    let deep = format!(
        "{}select a from t{}",
        "select a from (".repeat(200),
        ")".repeat(200)
    );
    let error = deep.parse::<Plan>().unwrap_err().to_string();
    assert!(error.contains("nest"), "{error}");
}

#[test]
fn a_create_section_is_the_field_it_declares() {
    let text = "create trades (id int64 not null, ccy utf8) with (comment = 'eu desk')";
    let plan: Plan = text.parse().unwrap();
    let field = plan.field().unwrap().unwrap();
    assert_eq!(field.name(), "trades");
    assert!(!field.is_nullable());
    assert_eq!(field.fields().len(), 2);
    assert_eq!(field.fields()[0].name(), "id");
    assert_eq!(field.fields()[0].dtype(), &DataType::Int64);
    assert!(!field.fields()[0].is_nullable());
    assert_eq!(field.get_metadata("comment"), Some("eu desk"));
    // A field is a plan that spells every column out, nullability included.
    let spelled = "create trades (id int64 not null, ccy utf8 null) with (comment = 'eu desk')";
    assert_eq!(Plan::from_field(&field).to_string(), spelled);
    assert_eq!(Plan::from(&field).field().unwrap(), Some(field.clone()));
    assert_eq!((&field).into_plan().unwrap().to_string(), spelled);
    assert_eq!(Expression::plan(&field).unwrap().to_string(), spelled);
    assert_eq!(
        spelled.parse::<Plan>().unwrap().field().unwrap(),
        Some(field.clone())
    );
    // A plan with no `create` declares nothing.
    assert_eq!(
        "select a from t".parse::<Plan>().unwrap().field().unwrap(),
        None
    );
    // Applied to a root, the read sections say what comes out.
    let root = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("ccy"),
        DataType::Float64.nullable_field("price"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("raw");
    let read: Plan = "select id, price * 2 as doubled where price > 0"
        .parse()
        .unwrap();
    let out = read.field_from(&root).unwrap();
    let names: Vec<&str> = out.fields().iter().map(Field::name).collect();
    assert_eq!(names, ["id", "doubled"]);
    assert_eq!(out.fields()[1].dtype(), &DataType::Float64);
}

#[test]
fn a_plan_simplifies_and_knows_when_it_does_nothing() {
    let plan: Plan = "select a where a = 1 or a = 2".parse().unwrap();
    assert_eq!(plan.simplify().to_string(), "select a where a in (1, 2)");
    assert!("select *".parse::<Plan>().unwrap().is_identity());
    assert!("select * where true".parse::<Plan>().unwrap().is_identity());
    assert!(Plan::new().is_empty());
    assert!(Plan::new().is_identity());
    assert!(!"select a".parse::<Plan>().unwrap().is_identity());
    assert!(!"select * from t".parse::<Plan>().unwrap().is_empty());
    assert!(!"insert into t".parse::<Plan>().unwrap().is_identity());
    let mut plan: Plan = "select a from t where b > 1 limit 3".parse().unwrap();
    plan.clear_read_sections();
    assert_eq!(plan.to_string(), "select * from t");
}

#[test]
fn a_join_is_a_read_section_every_accessor_accounts_for() {
    use yggdryl::JoinKind;

    let plan = Plan::new()
        .read_from(Target::parse("trades").unwrap())
        .join(
            JoinKind::Left,
            Target::parse("venues").unwrap(),
            "lower(venue) = mic",
        )
        .unwrap()
        .join(
            JoinKind::Semi,
            "select id from flags where kind = :kind"
                .parse::<Plan>()
                .unwrap(),
            "id = coalesce(:pin, id)",
        )
        .unwrap()
        .filter("city is not null")
        .unwrap();
    assert_eq!(
        plan.to_string(),
        "from trades left join venues on lower(venue) = mic \
         semi join (select id from flags where kind = :kind) on id = coalesce(:pin, id) \
         where city is not null"
    );
    assert_eq!(plan.to_string().parse::<Plan>().unwrap(), plan);
    let [left, semi] = plan.joins() else {
        panic!("expected two joins, got {:?}", plan.joins());
    };
    assert_eq!(left.how(), JoinKind::Left);
    assert_eq!(left.keys().to_string(), "lower(venue) = mic");
    assert_eq!(
        semi.source().to_string(),
        "(select id from flags where kind = :kind)"
    );
    // The keys' columns and parameters are the plan's; a nested source's
    // parameters are its own.
    assert_eq!(plan.columns(), ["city", "venue", "mic", "id"]);
    assert_eq!(plan.parameters(), ["pin"]);
    // A join is never the identity, and it is a read section.
    assert!(!plan.is_empty());
    assert!(!plan.is_identity());
    assert_eq!(
        plan.read_sections().to_string(),
        "left join venues on lower(venue) = mic \
         semi join (select id from flags where kind = :kind) on id = coalesce(:pin, id) \
         where city is not null"
    );
    let mut cleared = plan.clone();
    cleared.clear_read_sections();
    assert_eq!(cleared.to_string(), "select * from trades");
    // A join with no `from` joins whatever stream the plan is applied to,
    // and reads back as it prints.
    let streamed = Plan::new()
        .join(JoinKind::Inner, Target::parse("v").unwrap(), "id")
        .unwrap();
    assert!(!streamed.is_identity());
    assert_eq!(streamed.to_string(), "select * inner join v using (id)");
    assert_eq!(streamed.to_string().parse::<Plan>().unwrap(), streamed);
    // Simplifying reaches the keys and a nested source.
    let plan: Plan =
        "select * from t join (select * from v where a = 1 or a = 2) on (not not flag) = flag"
            .parse()
            .unwrap();
    assert_eq!(
        plan.simplify().to_string(),
        "select * from t inner join (from v where a in (1, 2)) using (flag)"
    );
    // The hash is the canonical text's: the kind is part of it.
    let inner: Plan = "select * from t join v using (id)".parse().unwrap();
    let outer: Plan = "select * from t left join v using (id)".parse().unwrap();
    assert_ne!(inner.stable_hash(), outer.stable_hash());
    assert_eq!(
        inner.stable_hash(),
        "select * from t inner join v using (id)"
            .parse::<Plan>()
            .unwrap()
            .stable_hash()
    );
    // An empty key list is refused by the builder, before any row.
    let error = Plan::new()
        .join(
            JoinKind::Inner,
            Target::parse("v").unwrap(),
            Vec::<&str>::new(),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("at least one key"), "{error}");
}

#[test]
fn a_join_types_its_output_where_both_roots_are_stated() {
    let trades = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
        DataType::utf8().nullable_field("city"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("trades");
    let names = |field: &Field| -> Vec<String> {
        field
            .fields()
            .iter()
            .map(|child| child.name().to_owned())
            .collect()
    };
    // The engine's layout: the left columns, then the right ones, a `using`
    // key once, a colliding right name suffixed, the optional side nullable.
    let plan: Plan = "select * from trades \
                      full join (create (venue utf8 not null, city utf8 not null, mic utf8)) using (venue)"
        .parse()
        .unwrap();
    let out = plan.field_from(&trades).unwrap();
    assert_eq!(out.name(), "trades");
    assert_eq!(names(&out), ["id", "venue", "city", "city_right", "mic"]);
    // A full join makes both sides optional; the coalesced key is present
    // whichever side holds the row.
    let nullable: Vec<bool> = out.fields().iter().map(Field::is_nullable).collect();
    assert_eq!(nullable, [true, false, true, true, true]);
    // The sections after it type against the joined root, and the datatype
    // door answers the same.
    let plan: Plan = "select id, upper(mic) as mic from trades \
                      join (create (venue utf8 not null, mic utf8 not null)) using (venue) \
                      where mic is not null"
        .parse()
        .unwrap();
    let out = plan.field_from(&trades).unwrap();
    assert_eq!(names(&out), ["id", "mic"]);
    assert!(!out.fields()[1].is_nullable());
    assert_eq!(
        plan.apply_datatype(trades.dtype()).unwrap(),
        out.dtype().clone()
    );
    // A semi join keeps the left columns alone.
    let plan: Plan = "select * from trades semi join (create (id int32 not null)) using (id)"
        .parse()
        .unwrap();
    assert_eq!(
        names(&plan.field_from(&trades).unwrap()),
        ["id", "venue", "city"]
    );
    // A target's columns are known only once it is read: refused naming it.
    let plan: Plan = "select * from trades join lake.venues using (venue)"
        .parse()
        .unwrap();
    let error = plan.field_from(&trades).unwrap_err().to_string();
    assert!(error.contains("lake.venues"), "{error}");
    assert!(error.contains("$.join"), "{error}");
    // A key that binds on no side is refused naming it.
    let plan: Plan = "select * from trades join (create (venue utf8)) on nope = venue"
        .parse()
        .unwrap();
    let error = plan.field_from(&trades).unwrap_err().to_string();
    assert!(error.contains("nope"), "{error}");
}

#[test]
fn an_ordering_key_spells_its_direction_and_where_nulls_go() {
    let key = Ordering::desc(Term::column("a")).nulls_first(true);
    assert_eq!(key.to_string(), "a desc nulls first");
    assert!(key.is_descending());
    assert!(key.is_nulls_first());
    assert_eq!(Ordering::asc(Term::column("a")).to_string(), "a");
    let plan: Plan = "select * from t order by a asc nulls last, lower(b) desc"
        .parse()
        .unwrap();
    assert_eq!(
        plan.to_string(),
        "select * from t order by a, lower(b) desc"
    );
    let built = Plan::new()
        .read_from(Target::parse("t").unwrap())
        .order_by([
            Ordering::asc(Term::column("a")),
            Ordering::desc(Term::column("b")),
        ])
        .limit(Some(3));
    assert_eq!(
        built.to_string(),
        "select * from t order by a, b desc limit 3"
    );
}

#[test]
fn an_ordering_key_is_read_from_text_and_carries_its_sort_options() {
    let key: Ordering = "price DESC nulls first".parse().unwrap();
    assert_eq!(key.term(), &Term::column("price"));
    assert_eq!(
        key.options(),
        SortOptions::descending().with_nulls_first(true)
    );
    assert_eq!(key.to_string(), "price desc nulls first");
    assert_eq!(key, Ordering::desc(Term::column("price")).nulls_first(true));
    assert_eq!(
        "lower(b) asc nulls last".parse::<Ordering>().unwrap(),
        Ordering::new(
            Term::call(yggdryl::expression::Function::Lower, [Term::column("b")]),
            SortOptions::default()
        )
    );
    for text in ["", "price desc nulls", "price, size", "price desc desc"] {
        assert!(text.parse::<Ordering>().is_err(), "{text:?}");
    }
    // The document keeps the two facts flat beside the term.
    let document = serde_json::to_value(&key).unwrap();
    assert_eq!(document["descending"], serde_json::json!(true));
    assert_eq!(document["nulls_first"], serde_json::json!(true));
    assert_eq!(serde_json::from_value::<Ordering>(document).unwrap(), key);
    let plain = serde_json::to_value(Ordering::asc(Term::column("a"))).unwrap();
    assert_eq!(plain.get("descending"), None);
    assert_eq!(plain.get("nulls_first"), None);
}

#[test]
fn a_plan_is_built_section_by_section() {
    let plan = Plan::new()
        .create(
            Some(Target::parse("trades").unwrap()),
            "id int64 not null".parse().unwrap(),
        )
        .write(
            Write::new(Verb::Upsert)
                .into(Target::parse("'file:///lake/trades.parquet'").unwrap())
                .by("id".parse().unwrap()),
        )
        .select("id, ccy")
        .unwrap()
        .read_from(Target::parse("raw").unwrap())
        .filter("price > 0")
        .unwrap()
        .offset(Some(1));
    assert_eq!(
        plan.to_string(),
        "create trades (id int64 not null) upsert into 'file:///lake/trades.parquet' by (id) \
         select id, ccy from raw where price > 0 offset 1"
    );
    assert_eq!(plan.clauses().len(), 2);
    // The merge keys live on the write section; setting them makes one.
    let mut plan = Plan::new();
    plan.set_merge_by("id".parse().unwrap());
    assert_eq!(plan.to_string(), "upsert by (id)");
    assert_eq!(plan.write_section().map(Write::verb), Some(Verb::Upsert));
}

// ---------------------------------------------------------------------------
// Streams and stores
// ---------------------------------------------------------------------------

mod streams {
    use super::*;
    use yggdryl::arrow::BatchReader;

    /// One batch as the stream it is.
    fn one_batch(batch: &RecordBatch) -> BatchReader {
        yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()])
    }

    /// Everything one stream yields, as the batch it amounts to.
    fn collected(reader: BatchReader) -> yggdryl::Result<RecordBatch> {
        let schema = reader.schema();
        let mut batches = Vec::new();
        for batch in reader {
            batches.push(
                batch.map_err(|error| yggdryl::Error::from(yggdryl::arrow::Error::from(error)))?,
            );
        }
        match batches.len() {
            0 => Ok(RecordBatch::new_empty(schema)),
            1 => Ok(batches.swap_remove(0)),
            _ => Ok(arrow_select::concat::concat_batches(&schema, &batches)
                .map_err(yggdryl::arrow::Error::from)?),
        }
    }
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::Holder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{
        IOBase, IOMedia, MediaTable, MediaType, MimeType, ObjectValue, Properties, SystemWarehouse,
    };

    #[test]
    fn a_path_resolves_through_the_warehouse_it_is_given() {
        let scratch = Scratch::new("warehouse");
        std::fs::write(scratch.0.join("trades.csv"), b"id,name\n1,a\n2,b\n").unwrap();
        let url: Url = scratch.url("trades.csv").parse().unwrap();
        let mut warehouse = Warehouse::new();
        warehouse
            .register(
                MediaTable::new("lake.eu.trades", url.clone())
                    .unwrap()
                    .with_properties(Properties::new().with_property("tier", "hot")),
            )
            .unwrap();
        // A read pushes its sections into the registered table.
        let read: Plan = "select id from lake.eu.trades where id = 2"
            .parse()
            .unwrap();
        assert_eq!(
            ids(&collected(read.execute_in(&warehouse).unwrap()).unwrap()),
            [2]
        );
        // The target's own properties are stated on the table it names,
        // over the table's.
        let target =
            Target::parse("lake.eu.trades with (tier = 'cold', codec = 'identity')").unwrap();
        let Holder::Table(table) = target.holder(&warehouse, None).unwrap() else {
            panic!("a path holds the table");
        };
        let properties = table.properties().unwrap();
        assert_eq!(properties.get("tier"), Some("cold"));
        assert_eq!(properties.get("codec"), Some("identity"));
        // A URL inherits what the warehouse states for it, its own winning.
        let typed = Target::parse(&format!("'{url}' with (tier = 'warm')")).unwrap();
        assert_eq!(
            typed
                .properties()
                .inherit(&warehouse.properties_for(&url))
                .get("tier"),
            Some("warm")
        );
        assert_eq!(warehouse.properties_for(&url).get("tier"), Some("hot"));
        // A namespace or a catalog in a `from` is refused naming its kind.
        for (path, kind) in [("lake.eu", "namespace"), ("lake", "catalog")] {
            let error = Target::parse(path)
                .unwrap()
                .holder(&warehouse, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains(kind) && error.contains(path), "{error}");
        }
        // Absence names the path and says what to do.
        let error = Target::parse("lake.eu.fills")
            .unwrap()
            .holder(&warehouse, None)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected a table at \"lake.eu.fills\", got nothing"),
            "{error}"
        );
        assert!(
            error.contains("register the table or name a URL"),
            "{error}"
        );
        // A write to an absent path asks the namespace above it to create the
        // table; a memory namespace creates none, and says so by name.
        let write: Plan = "insert into lake.eu.fills select * from lake.eu.trades"
            .parse()
            .unwrap();
        let error = write
            .execute_in(&warehouse)
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains("creating a table"), "{error}");
        assert!(error.contains("MemoryNamespace"), "{error}");
        // A write to a registered table lands where the table is, creating
        // its file as a write to a URL does.
        let copy: Url = scratch.url("copy.csv").parse().unwrap();
        warehouse
            .register(MediaTable::new("lake.eu.copy", copy).unwrap())
            .unwrap();
        let write: Plan = "insert into lake.eu.copy select * from lake.eu.trades"
            .parse()
            .unwrap();
        collected(write.execute_in(&warehouse).unwrap()).unwrap();
        assert_eq!(
            warehouse.table("lake.eu.copy").unwrap().row_size().unwrap(),
            2
        );
        let back: Plan = "select id from lake.eu.copy order by id desc"
            .parse()
            .unwrap();
        assert_eq!(
            ids(&collected(back.execute_in(&warehouse).unwrap()).unwrap()),
            [2, 1]
        );
    }

    #[test]
    fn a_join_source_resolves_through_the_warehouse_like_a_from() {
        let scratch = Scratch::new("joins");
        std::fs::write(scratch.0.join("trades.csv"), b"id,name\n1,a\n2,b\n3,c\n").unwrap();
        std::fs::write(scratch.0.join("venues.csv"), b"id,venue\n1,XNAS\n3,XLON\n").unwrap();
        let mut warehouse = Warehouse::new();
        for name in ["trades", "venues"] {
            let url: Url = scratch.url(&format!("{name}.csv")).parse().unwrap();
            warehouse
                .register(MediaTable::new(format!("lake.eu.{name}").as_str(), url).unwrap())
                .unwrap();
        }
        // The probe and the build side are both registered tables, read
        // through the warehouse the plan runs in.
        let joined: Plan =
            "select id, venue from lake.eu.trades join lake.eu.venues using (id) order by id"
                .parse()
                .unwrap();
        assert_eq!(
            ids(&collected(joined.execute_in(&warehouse).unwrap()).unwrap()),
            [1, 3]
        );
        // A join source with no table registered at its path is reported at
        // that location, as a `from` is.
        let absent: Plan = "select * from lake.eu.trades join lake.eu.nowhere using (id)"
            .parse()
            .unwrap();
        let error = absent
            .execute_in(&warehouse)
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains("$.lake.eu.nowhere"), "{error}");
        assert!(
            error.contains("expected a table at \"lake.eu.nowhere\", got nothing"),
            "{error}"
        );
    }

    #[test]
    fn execute_resolves_against_the_system_warehouse() {
        let scratch = Scratch::new("system");
        std::fs::write(scratch.0.join("trades.csv"), b"id,name\n1,a\n2,b\n3,c\n").unwrap();
        let url: Url = scratch.url("trades.csv").parse().unwrap();
        let catalog = format!("plan_test_{}", std::process::id());
        let path = format!("{catalog}.eu.trades");
        SystemWarehouse::register(MediaTable::new(path.as_str(), url).unwrap()).unwrap();
        let read: Plan = format!("select id from {path} where id > 1")
            .parse()
            .unwrap();
        let outcome = read.execute().and_then(collected);
        SystemWarehouse::unregister([catalog.as_str()]).unwrap();
        assert_eq!(ids(&outcome.unwrap()), [2, 3]);
        // Unregistered, the path is nothing again.
        let error = read.execute().err().expect("refused").to_string();
        assert!(
            error.contains("register the table or name a URL"),
            "{error}"
        );
    }

    /// A directory of this process alone, removed when the test is done.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "yggdryl-plan-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn url(&self, name: &str) -> String {
            Url::from_path(self.0.join(name)).unwrap().to_string()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn trades(rows: &[(i64, &str)]) -> RecordBatch {
        let schema = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("trades");
        RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![
                std::sync::Arc::new(Int64Array::from_iter_values(rows.iter().map(|(id, _)| *id))),
                std::sync::Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|(_, name)| *name),
                )),
            ],
        )
        .unwrap()
    }

    fn ids(batch: &RecordBatch) -> Vec<i64> {
        batch
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values()
            .to_vec()
    }

    fn names(batch: &RecordBatch) -> Vec<String> {
        batch
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .map(|name| name.unwrap_or("<null>").to_owned())
            .collect()
    }

    #[test]
    fn a_plan_shapes_a_stream_in_section_order() {
        let batch = trades(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        let plan: Plan =
            "select id, upper(name) as name where id > 1 order by id desc offset 1 limit 2"
                .parse()
                .unwrap();
        assert_eq!(
            plan.to_string(),
            "select id, upper(name) as name where id > 1 order by id desc limit 2 offset 1"
        );
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        assert_eq!(ids(&out), [3, 2]);
        assert_eq!(names(&out), ["C", "B"]);
        // A key orders by a column the projection drops, or by an alias it
        // publishes.
        let plan: Plan = "select upper(name) as name where id > 1 order by id desc"
            .parse()
            .unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        assert_eq!(names(&out), ["D", "C", "B"]);
        let plan: Plan = "select id * -1 as negated order by negated"
            .parse()
            .unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        let held: Vec<i64> = out
            .column_by_name("negated")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values()
            .to_vec();
        assert_eq!(held, [-4, -3, -2, -1]);
        // The same through the expression, and through a sequence of steps.
        let expression: Expression = "where id > 1; select id order by id desc; limit 1"
            .parse()
            .unwrap();
        let out = expression.apply_arrow_batch(&batch).unwrap();
        assert_eq!(ids(&out), [4]);
        assert_eq!(out.num_columns(), 1);
        // A plan whose `from` names a store still shapes the stream it is
        // given; the source is what `execute` reads.
        let plan: Plan = "select id from 'file:///nowhere/at/all.parquet' where id = 2"
            .parse()
            .unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        assert_eq!(ids(&out), [2]);
    }

    #[test]
    fn an_ordering_keeps_the_rows_it_cannot_tell_apart_in_arrival_order() {
        // Sixty-four rows over three keys, scattered: every key is tied many
        // times over, and each tie keeps the order its rows arrived in,
        // whichever way the key runs.
        let mut state = 7_u64;
        let keys: Vec<&str> = (0..64)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ["a", "b", "c"][usize::try_from((state >> 33) % 3).unwrap()]
            })
            .collect();
        let rows: Vec<(i64, &str)> = keys
            .iter()
            .enumerate()
            .map(|(row, key)| (i64::try_from(row).unwrap(), *key))
            .collect();
        let batch = trades(&rows);
        for (text, order) in [
            ("select * order by name", ["a", "b", "c"]),
            ("select * order by name desc", ["c", "b", "a"]),
        ] {
            let plan: Plan = text.parse().unwrap();
            let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
            let expected: Vec<i64> = order
                .iter()
                .flat_map(|key| {
                    rows.iter()
                        .filter(move |(_, name)| name == key)
                        .map(|(id, _)| *id)
                })
                .collect();
            assert_eq!(ids(&out), expected, "{text}");
        }
    }

    #[test]
    fn a_plan_over_a_struct_array_orders_a_null_row_as_one_whose_keys_are_null() {
        use arrow_array::{ArrayRef, StructArray};
        use arrow_buffer::NullBuffer;

        // The second row is a null struct over `(9, "z")`: nothing reads it,
        // every key is null for it, and it stays a null row.
        let rows = trades(&[(3, "c"), (9, "z"), (1, "a"), (2, "b")]);
        let (fields, columns, _) = StructArray::from(rows).into_parts();
        let array: ArrayRef = std::sync::Arc::new(
            StructArray::try_new(
                fields,
                columns,
                Some(NullBuffer::from(vec![true, false, true, true])),
            )
            .unwrap(),
        );
        // The first column of every row, null where the row is.
        let ids = |array: &ArrayRef| -> Vec<Option<i64>> {
            let held = array.as_any().downcast_ref::<StructArray>().unwrap();
            let ids = held
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap();
            (0..held.len())
                .map(|row| held.is_valid(row).then(|| ids.value(row)))
                .collect()
        };
        for (text, expected) in [
            (
                "select id order by id",
                vec![Some(1), Some(2), Some(3), None],
            ),
            (
                "select id order by id nulls first limit 2",
                vec![None, Some(1)],
            ),
            (
                "select id, name order by id desc offset 1 limit 2",
                vec![Some(2), Some(1)],
            ),
            // An ordering over an alias the select publishes orders after it.
            (
                "select id * -1 as negated order by negated",
                vec![Some(-3), Some(-2), Some(-1), None],
            ),
            (
                "select id where name <> 'z' order by id desc",
                vec![Some(3), Some(2), Some(1)],
            ),
            ("select id limit 2", vec![Some(3), None]),
        ] {
            let expression: Expression = text.parse().unwrap();
            let out = expression
                .apply_arrow_array(&array)
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(ids(&out), expected, "{text}");
        }
        // A null is not a record: a plan that would write one is refused
        // before it opens its target.
        let plan: Expression = "insert into 'file:///nowhere/rows.arrows' select *"
            .parse()
            .unwrap();
        let error = plan.apply_arrow_array(&array).unwrap_err().to_string();
        assert!(error.contains("null struct at row 1"), "{error}");
    }

    #[test]
    fn a_star_plan_drops_what_it_excludes_and_appends_what_it_computes() {
        let batch = trades(&[(1, "a"), (2, "b"), (3, "c")]);
        let plan: Plan = "select * exclude (name), upper(name) as shout where id > 1"
            .parse()
            .unwrap();
        assert_eq!(
            plan.to_string(),
            "select * exclude (name), upper(name) as shout where id > 1"
        );
        assert_eq!(plan.to_string().parse::<Plan>().unwrap(), plan);
        assert_eq!(plan.read_columns(), None);
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        let published: Vec<&str> = out
            .schema_ref()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(published, ["id", "shout"]);
        assert_eq!(ids(&out), [2, 3]);
        let shouted: Vec<Option<&str>> = out
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(shouted, [Some("B"), Some("C")]);
        // With nothing to filter, a kept column is the batch's own array.
        let plan: Plan = "select * exclude (name), id * 2 as twice".parse().unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        assert!(std::sync::Arc::ptr_eq(out.column(0), batch.column(0)));
        assert_eq!(out.schema_ref().field(1).name(), "twice");
    }

    #[test]
    fn a_sliced_reader_walks_batch_boundaries_without_copying_whole_batches() {
        let batches = [
            trades(&[(1, "a"), (2, "b")]),
            trades(&[(3, "c"), (4, "d")]),
            trades(&[(5, "e"), (6, "f")]),
        ];
        let stream = || yggdryl::arrow::batch_reader(batches[0].schema(), batches.clone());
        let sliced: Vec<RecordBatch> = yggdryl::arrow::sliced_reader(stream(), 1, Some(3))
            .map(|batch| batch.unwrap())
            .collect();
        assert_eq!(
            sliced.len(),
            2,
            "the run spans two batches and skips the third"
        );
        let held: Vec<i64> = sliced.iter().flat_map(ids).collect();
        assert_eq!(held, [2, 3, 4]);
        let held: Vec<i64> = yggdryl::arrow::sliced_reader(stream(), 5, None)
            .flat_map(|batch| ids(&batch.unwrap()))
            .collect();
        assert_eq!(held, [6]);
        let held: Vec<i64> = yggdryl::arrow::sliced_reader(stream(), 9, Some(2))
            .flat_map(|batch| ids(&batch.unwrap()))
            .collect();
        assert!(held.is_empty());
        let held: Vec<i64> = yggdryl::arrow::sliced_reader(stream(), 0, Some(0))
            .flat_map(|batch| ids(&batch.unwrap()))
            .collect();
        assert!(held.is_empty());
    }

    #[test]
    fn a_plan_creates_inserts_upserts_deletes_and_reads_a_store() {
        let scratch = Scratch::new("store");
        let url = scratch.url("trades.arrow");
        let read = |text: &str| -> RecordBatch {
            let plan: Plan = text.replace("{url}", &url).parse().unwrap();
            collected(plan.execute().unwrap()).unwrap()
        };
        let run = |text: &str, batch: &RecordBatch| {
            let plan: Plan = text.replace("{url}", &url).parse().unwrap();
            let out = collected(plan.apply_arrow_reader(one_batch(batch)).unwrap()).unwrap();
            assert_eq!(out.num_rows(), 0, "a write yields the empty stream");
            out
        };

        // `create` alone writes the declared schema and no rows.
        let plan: Plan = format!("create '{url}' (id int64 not null, name utf8)")
            .parse()
            .unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(out.num_rows(), 0);
        assert_eq!(out.schema().fields().len(), 2);
        let stored = read("select * from '{url}'");
        assert_eq!(stored.num_rows(), 0);
        assert_eq!(stored.schema().field(0).name(), "id");

        // `insert into` appends.
        run(
            "insert into '{url}'",
            &trades(&[(1, "a"), (2, "b"), (3, "c")]),
        );
        run("append to '{url}'", &trades(&[(4, "d")]));
        assert_eq!(ids(&read("select * from '{url}'")), [1, 2, 3, 4]);

        // A read pushes its sections down, and orders what comes back.
        let out = read("select id, name from '{url}' where id > 1 order by id desc limit 2");
        assert_eq!(ids(&out), [4, 3]);
        assert_eq!(names(&out), ["d", "c"]);
        let out = read("select name from '{url}' where id = 2");
        assert_eq!(out.num_columns(), 1);
        assert_eq!(names(&out), ["b"]);
        // With nothing to order, the offset is pushed down with the limit:
        // the media skips, then bounds.
        assert_eq!(ids(&read("select * from '{url}' limit 2 offset 1")), [2, 3]);
        assert_eq!(ids(&read("select * from '{url}' offset 3")), [4]);

        // `upsert ... by` replaces matching keys and adds the rest.
        run(
            "upsert into '{url}' by (id)",
            &trades(&[(2, "B"), (5, "e")]),
        );
        let out = read("select * from '{url}' order by id");
        assert_eq!(ids(&out), [1, 2, 3, 4, 5]);
        assert_eq!(names(&out), ["a", "B", "c", "d", "e"]);

        // `delete ... where` removes what the predicate keeps.
        let plan: Plan = format!("delete from '{url}' where id in (1, 3)")
            .parse()
            .unwrap();
        assert_eq!(collected(plan.execute().unwrap()).unwrap().num_rows(), 0);
        assert_eq!(ids(&read("select * from '{url}' order by id")), [2, 4, 5]);

        // `insert overwrite` replaces everything.
        run("insert overwrite '{url}'", &trades(&[(9, "z")]));
        assert_eq!(ids(&read("select * from '{url}'")), [9]);

        // A plan reads one store and writes another, with a nested plan as
        // its source.
        let copy = scratch.url("copy.arrow");
        let plan: Plan = format!(
            "insert into '{copy}' select id * 2 as id, name from (select * from '{url}' where id = 9)"
        )
        .parse()
        .unwrap();
        assert_eq!(collected(plan.execute().unwrap()).unwrap().num_rows(), 0);
        let out = read(&format!("select * from '{copy}'"));
        assert_eq!(ids(&out), [18]);
        assert_eq!(names(&out), ["z"]);

        // A store that is not there yet reads as the empty stream, which is
        // what lets the first `insert into` it be the same plan as the rest.
        let missing = scratch.url("missing.arrow");
        let plan: Plan = format!("select * from '{missing}'").parse().unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(out.num_rows(), 0);
        run("insert into '{url}'", &trades(&[(7, "g")]));
        assert_eq!(ids(&read("select * from '{url}' order by id")), [7, 9]);
    }

    /// Orders: an id and the venue it traded on.
    fn orders(rows: &[(i64, &str)]) -> RecordBatch {
        RecordBatch::try_from_iter([
            (
                "id",
                std::sync::Arc::new(Int64Array::from_iter_values(rows.iter().map(|(id, _)| *id)))
                    as arrow_array::ArrayRef,
            ),
            (
                "venue",
                std::sync::Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|(_, venue)| *venue),
                )),
            ),
        ])
        .unwrap()
    }

    /// Venues: a venue and its city, some of them unknown.
    fn venues(rows: &[(&str, Option<&str>)]) -> RecordBatch {
        RecordBatch::try_from_iter([
            (
                "venue",
                std::sync::Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|(venue, _)| *venue),
                )) as arrow_array::ArrayRef,
            ),
            (
                "city",
                std::sync::Arc::new(StringArray::from_iter(rows.iter().map(|(_, city)| *city))),
            ),
        ])
        .unwrap()
    }

    /// Write `batch` where `url` names, through a plan.
    fn stored(url: &str, batch: &RecordBatch) {
        let plan: Plan = format!("insert overwrite '{url}'").parse().unwrap();
        collected(plan.apply_arrow_reader(one_batch(batch)).unwrap()).unwrap();
    }

    /// A batch as the record column it is.
    fn serie_of(batch: &RecordBatch) -> yggdryl::Serie {
        yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::new()).unwrap()
    }

    /// The names of a batch's columns.
    fn columns(batch: &RecordBatch) -> Vec<String> {
        batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect()
    }

    fn cities(batch: &RecordBatch) -> Vec<String> {
        batch
            .column_by_name("city")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .map(|city| city.unwrap_or("<null>").to_owned())
            .collect()
    }

    #[test]
    fn a_plan_joins_two_stores_as_the_serie_join_does() {
        use yggdryl::{JoinKind, JoinOptions, JoinSide};

        let scratch = Scratch::new("join");
        let (left_url, right_url) = (scratch.url("orders.arrow"), scratch.url("venues.arrows"));
        let left = orders(&[
            (1, "XNAS"),
            (2, "XPAR"),
            (3, "XNYS"),
            (4, "XNAS"),
            (5, "XLON"),
        ]);
        let right = venues(&[
            ("XNAS", Some("New York")),
            ("XPAR", Some("Paris")),
            ("XNYS", None),
        ]);
        stored(&left_url, &left);
        stored(&right_url, &right);
        // The left streams and probes; the right is held and hashed.
        let options = JoinOptions::new().with_build(Some(JoinSide::Right));
        for (word, how) in [
            ("join", JoinKind::Inner),
            ("left join", JoinKind::Left),
            ("semi join", JoinKind::Semi),
            ("anti join", JoinKind::Anti),
            ("right join", JoinKind::Right),
            ("full join", JoinKind::Full),
        ] {
            let plan: Plan =
                format!("select * from '{left_url}' {word} '{right_url}' using (venue)")
                    .parse()
                    .unwrap();
            let out = collected(plan.execute().unwrap()).unwrap();
            let expected = serie_of(&left)
                .join_with(&serie_of(&right), "venue", how, &options)
                .unwrap();
            assert_eq!(serie_of(&out), expected, "{word}");
            assert_eq!(
                columns(&out),
                columns(&expected.into_arrow_batch().unwrap()),
                "{word}"
            );
        }
        // An `on` pair keeps both key columns.
        let plan: Plan =
            format!("select * from '{left_url}' join '{right_url}' on lower(venue) = lower(venue)")
                .parse()
                .unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(columns(&out), ["id", "venue", "venue_right", "city"]);
        assert_eq!(ids(&out), [1, 2, 3, 4]);

        // A `where` naming a right column runs over the joined rows, and so
        // does an `order by` on one; the projection publishes from them too.
        let plan: Plan = format!(
            "select id, city from '{left_url}' left join '{right_url}' using (venue) \
             where city is not null or id = 5 order by city desc nulls first, id"
        )
        .parse()
        .unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(columns(&out), ["id", "city"]);
        assert_eq!(ids(&out), [5, 2, 1, 4]);
        assert_eq!(cities(&out), ["<null>", "Paris", "New York", "New York"]);
        let plan: Plan = format!(
            "select id from '{left_url}' join '{right_url}' using (venue) \
             where city = 'New York' order by id desc limit 1"
        )
        .parse()
        .unwrap();
        assert_eq!(ids(&collected(plan.execute().unwrap()).unwrap()), [4]);

        // Joins chain left to right, each over the rows so far, and a nested
        // plan is a source like a target.
        let plan: Plan = format!(
            "select id, city from '{left_url}' join '{right_url}' using (venue) \
             anti join (select id from '{left_url}' where id > 3) using (id) order by id"
        )
        .parse()
        .unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(ids(&out), [1, 2, 3]);
        assert_eq!(cities(&out), ["New York", "Paris", "<null>"]);
        // The same plan applied to a stream joins the stream.
        let plan: Plan = format!(
            "select id, city from 'file:///nowhere' join '{right_url}' using (venue) order by id"
        )
        .parse()
        .unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&left)).unwrap()).unwrap();
        assert_eq!(ids(&out), [1, 2, 3, 4]);

        // A key that binds on no side is refused naming it, before a row is
        // joined.
        let plan: Plan = format!("select * from '{left_url}' join '{right_url}' on nope = venue")
            .parse()
            .unwrap();
        let error = plan.execute().map(|_| ()).unwrap_err().to_string();
        assert!(error.contains("nope"), "{error}");
    }

    #[test]
    fn the_first_join_prunes_the_left_read_by_its_held_keys() {
        // The left is a partitioned folder with a leaf no reader can decode:
        // a read that reaches it fails, so a plan that succeeds is one whose
        // read never listed it. One distinct right key is the equality a
        // partition path answers.
        let scratch = Scratch::new("pushdown");
        let lake = scratch.0.join("orders");
        for (index, venue) in ["XNAS", "XNYS", "XPAR"].into_iter().enumerate() {
            let batch = orders(
                &(0..1000)
                    .filter(|id| id % 3 == index as i64)
                    .map(|id| (id, venue))
                    .collect::<Vec<_>>(),
            );
            let ids = batch.project(&[0]).unwrap();
            let folder = lake.join(format!("venue={venue}"));
            std::fs::create_dir_all(&folder).unwrap();
            stored(
                &Url::from_path(folder.join("part-0.arrows"))
                    .unwrap()
                    .to_string(),
                &ids,
            );
        }
        let broken = lake.join("venue=ZZZZ");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("part-0.arrows"), b"not an arrow stream").unwrap();
        let lake_url = Url::from_path(&lake).unwrap().to_string();
        let left =
            format!("'{lake_url}' with (media_type = 'application/vnd.apache.arrow.stream')");
        let right_url = scratch.url("venues.arrows");
        stored(
            &right_url,
            &venues(&[("XNYS", Some("New York")), ("XNYS", Some("again"))]),
        );

        // `inner`, `right` and `semi` emit no unmatched left row: the left
        // read is filtered by `venue in ('XNYS')` and never reaches the leaf
        // that cannot be read.
        for word in ["join", "right join", "semi join"] {
            let plan: Plan =
                format!("select id from {left} {word} '{right_url}' using (venue) order by id")
                    .parse()
                    .unwrap();
            let out = collected(plan.execute().unwrap())
                .unwrap_or_else(|error| panic!("{word}: {error}"));
            let expected: Vec<i64> = (0..1000)
                .filter(|id| id % 3 == 1)
                .flat_map(|id| {
                    if word == "semi join" {
                        vec![id]
                    } else {
                        vec![id, id]
                    }
                })
                .collect();
            assert_eq!(ids(&out), expected, "{word}");
            let explained = Expression::Plan(Box::new(plan)).explain();
            assert!(
                explained.contains("pushdown venue in (distinct venue, at most 10000)"),
                "{explained}"
            );
        }
        // A `left` join emits every left row: nothing is pushed, and the read
        // reaches the leaf and fails decoding it.
        let plan: Plan = format!("select id from {left} left join '{right_url}' using (venue)")
            .parse()
            .unwrap();
        let error = plan
            .execute()
            .and_then(collected)
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert!(error.contains("codec I/O error"), "{error}");
        // A `where` conjunct over left columns alone prunes the left read too,
        // where no join nulls a left column...
        let plan: Plan = format!(
            "select id, city from {left} left join '{right_url}' using (venue) \
             where venue = 'XPAR' and id < 10 order by id"
        )
        .parse()
        .unwrap();
        let out = collected(plan.execute().unwrap()).unwrap();
        assert_eq!(ids(&out), [2, 5, 8]);
        assert_eq!(cities(&out), ["<null>", "<null>", "<null>"]);
        // ...and not where one does: a `full` join reads the left whole.
        let plan: Plan = format!(
            "select id from {left} full join '{right_url}' using (venue) where venue = 'XPAR'"
        )
        .parse()
        .unwrap();
        let error = plan
            .execute()
            .and_then(collected)
            .map(|_| ())
            .unwrap_err()
            .to_string();
        assert!(error.contains("codec I/O error"), "{error}");
    }

    #[test]
    fn a_create_with_no_target_declares_what_the_stream_becomes() {
        let batch = trades(&[(1, "a"), (2, "b")]);
        let plan: Plan = "create (id int32 not null, name utf8) select id, name where id = 2"
            .parse()
            .unwrap();
        let out = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        assert_eq!(out.num_rows(), 1);
        assert_eq!(
            out.schema().field(0).data_type(),
            &arrow_schema::DataType::Int32
        );
        assert!(!out.schema().field(0).is_nullable());
        // A declared schema a stream cannot meet says which column.
        let plan: Plan = "create (id int64 not null, name int64 not null)"
            .parse()
            .unwrap();
        let error = collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("name"), "{error}");
    }

    #[test]
    fn a_target_holds_its_location_with_its_properties() {
        let scratch = Scratch::new("holder");
        let plain = scratch.url("plain");
        let typed = format!("'{plain}' with (media_type = 'application/vnd.apache.arrow.stream')");
        let target = Target::parse(&typed).unwrap();
        let holder = target.holder(&Warehouse::new(), None).unwrap();
        assert_eq!(holder.media_type(), &MediaType::new(MimeType::ARROW_STREAM));
        // The property types an extensionless store for a write and a read.
        let plan: Plan = format!("insert into {typed}").parse().unwrap();
        let batch = trades(&[(1, "a"), (2, "b")]);
        collected(plan.apply_arrow_reader(one_batch(&batch)).unwrap()).unwrap();
        let read: Plan = format!("select id from {typed} where id = 2")
            .parse()
            .unwrap();
        assert_eq!(ids(&collected(read.execute().unwrap()).unwrap()), [2]);
        // Parts resolve against a base; without one they cannot be held.
        let error = Target::parse("raw.trades")
            .unwrap()
            .holder(&Warehouse::new(), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("raw.trades"), "{error}");
        let base: Url = scratch.url("").parse().unwrap();
        let held = Target::parse("plain")
            .unwrap()
            .holder(&Warehouse::new(), Some(&base))
            .unwrap();
        assert_eq!(held.url().unwrap().to_string(), plain);
        // The knobs that shape a read or write are read from the properties.
        let target = Target::parse(&format!(
            "'{plain}' with (batch_row_size = '1', safe = 'false')"
        ))
        .unwrap();
        let options = target.record_options(&holder).unwrap();
        assert_eq!(options.batch_row_size(), Some(1));
        assert!(!options.safe());
    }

    #[test]
    fn a_target_reads_its_commit_cadence_as_a_batch_count() {
        let scratch = Scratch::new("cadence");
        let plain = scratch.url("trades.arrows");
        let holder = Target::parse(&format!("'{plain}'"))
            .unwrap()
            .holder(&Warehouse::new(), None)
            .unwrap();
        // Unstated, the destination keeps its own cadence.
        let options = Target::parse(&format!("'{plain}'"))
            .unwrap()
            .record_options(&holder)
            .unwrap();
        assert_eq!(options.commit_batch_num(), None);
        let target = Target::parse(&format!("'{plain}' with (commit_batch_num = '3')")).unwrap();
        let options = target.record_options(&holder).unwrap();
        assert_eq!(options.commit_batch_num(), Some(3));
        // A cadence that is not a count is refused naming the knob.
        let broken =
            Target::parse(&format!("'{plain}' with (commit_batch_num = 'three')")).unwrap();
        let error = broken.record_options(&holder).unwrap_err().to_string();
        assert!(error.contains("$.with.commit_batch_num"), "{error}");
        assert!(error.contains("a whole number"), "{error}");
        assert!(error.contains("three"), "{error}");
    }

    #[test]
    fn a_holder_is_built_from_a_url_and_properties() {
        let scratch = Scratch::new("url");
        std::fs::write(scratch.0.join("t.csv"), b"id\n1\n").unwrap();
        let url: Url = scratch.url("t.csv").parse().unwrap();
        let none: [(&str, &str); 0] = [];
        let plain = Holder::from_url(&url, none).unwrap();
        assert_eq!(plain.media_type(), &MediaType::new(MimeType::CSV));
        assert_eq!(plain.size(), 5);
        let typed = Holder::from_url(&url, [("content-type", "application/json")]).unwrap();
        assert_eq!(typed.media_type(), &MediaType::new(MimeType::JSON));
        let Holder::Coded(coded) = Holder::from_url(&url, [("codec", "gzip")]).unwrap() else {
            panic!("expected the codec property to wrap the handle");
        };
        assert_eq!(coded.codec(), yggdryl::Codec::Gzip);
        assert_eq!(
            Holder::try_from(&url).unwrap().media_type(),
            &MediaType::new(MimeType::CSV)
        );
        // A scheme no backend holds is refused by name.
        let foreign: Url = "ftp://example.com/t.csv".parse().unwrap();
        let error = Holder::from_url(&foreign, none).unwrap_err().to_string();
        assert!(error.contains("ftp"), "{error}");
        // A property that does not parse names its value.
        let error = Holder::from_url(&url, [("codec", "not a codec")])
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a codec"), "{error}");
    }

    #[test]
    fn records_run_through_every_expression() {
        let rows = || {
            [(1_i64, "a"), (2, "b"), (3, "c")]
                .into_iter()
                .map(|(id, name)| {
                    Scalar::from_struct([("id", Scalar::from(id)), ("name", Scalar::from(name))])
                        .unwrap()
                })
        };
        let filter: yggdryl::Filter = "id > 1".parse().unwrap();
        let kept = filter.apply_records(None, rows()).unwrap();
        assert_eq!(kept.field().fields().len(), 2);
        let kept = kept.collect_rows().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(
            kept[0],
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("b")])
        );
        // A plan goes through the vectorized tier and comes back as rows.
        let plan: Expression = "select upper(name) as name where id > 1 order by id desc limit 1"
            .parse()
            .unwrap();
        let out = plan.apply_records(None, rows()).unwrap();
        assert_eq!(out.field().fields()[0].name(), "name");
        assert_eq!(
            out.collect_rows().unwrap(),
            [Scalar::from_sequence([Scalar::from("C")])]
        );
        // Records stream into a batch reader and back.
        let selector: Selector = "id * 10 as id".parse().unwrap();
        let reader = selector
            .apply_records(None, rows())
            .unwrap()
            .into_arrow_reader()
            .unwrap();
        let batch = collected(reader).unwrap();
        assert_eq!(ids(&batch), [10, 20, 30]);
        let back = yggdryl::expression::Records::from_arrow_reader(one_batch(&batch)).unwrap();
        assert_eq!(back.field().fields()[0].name(), "id");
        assert_eq!(back.collect_rows().unwrap().len(), 3);
        // Without a schema and without a record there is nothing to bind to.
        let error = selector
            .apply_records(None, std::iter::empty::<Scalar>())
            .unwrap_err()
            .to_string();
        assert!(error.contains("schema"), "{error}");
        // A declared schema binds even an empty stream.
        let schema = StructType::from_fields([DataType::Int64.required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("rows");
        let empty = selector
            .apply_records(Some(&schema), std::iter::empty::<Scalar>())
            .unwrap();
        assert!(empty.collect_rows().unwrap().is_empty());
    }
}

// ---------------------------------------------------------------------------
// Explain
// ---------------------------------------------------------------------------

#[test]
fn explain_draws_every_section_as_a_branch() {
    let plan: Expression = "upsert into t with (safe = 'false') by (id) \
                            select id, price * 2 as doubled from s where price > 0 \
                            order by id desc limit 3"
        .parse()
        .unwrap();
    assert_eq!(
        plan.explain(),
        [
            "plan",
            "├─ upsert into t",
            "│  ├─ with",
            "│  │  └─ safe = \"false\"",
            "│  └─ by",
            "│     └─ column id",
            "├─ from",
            "│  └─ s",
            "├─ where",
            "│  └─ >",
            "│     ├─ column price",
            "│     └─ literal 0",
            "├─ select",
            "│  ├─ column id",
            "│  └─ doubled",
            "│     └─ *",
            "│        ├─ column price",
            "│        └─ literal 2",
            "├─ order by",
            "│  └─ desc",
            "│     └─ column id",
            "└─ limit 3",
        ]
        .join("\n")
    );
    let sequence: Expression = "where a > 1; select a".parse().unwrap();
    assert_eq!(
        sequence.explain(),
        [
            "sequence",
            "├─ where",
            "│  └─ >",
            "│     ├─ column a",
            "│     └─ literal 1",
            "└─ select",
            "   └─ column a",
        ]
        .join("\n")
    );
    // A predicate segment is drawn under the path that keeps elements by it,
    // in a `where` and in a `select` alike.
    let kept: Expression = "select legs[ccy = 'EUR'] as eur where size(legs[active]) > 0"
        .parse()
        .unwrap();
    assert_eq!(
        kept.explain(),
        [
            "plan",
            "├─ where",
            "│  └─ >",
            "│     ├─ call size",
            "│     │  └─ path legs[active]",
            "│     │     └─ where",
            "│     │        └─ column active",
            "│     └─ literal 0",
            "└─ select",
            "   └─ eur",
            "      └─ path legs[ccy = 'EUR']",
            "         └─ where",
            "            └─ =",
            "               ├─ column ccy",
            "               └─ literal 'EUR'",
        ]
        .join("\n")
    );
}

#[test]
fn a_verb_reads_every_spelling_the_grammar_reads_and_prints_its_canonical_one() {
    for (text, verb) in [
        ("insert", Verb::Insert),
        ("INSERT INTO", Verb::Insert),
        (" append ", Verb::Insert),
        ("append to", Verb::Insert),
        ("append into", Verb::Insert),
        ("insert overwrite", Verb::Overwrite),
        ("insert overwrite into", Verb::Overwrite),
        ("overwrite", Verb::Overwrite),
        ("overwrite into", Verb::Overwrite),
        ("replace", Verb::Overwrite),
        ("Replace Into", Verb::Overwrite),
        ("upsert", Verb::Upsert),
        ("upsert into", Verb::Upsert),
        ("merge", Verb::Upsert),
        ("merge into", Verb::Upsert),
        ("delete", Verb::Delete),
        ("delete from", Verb::Delete),
    ] {
        assert_eq!(text.parse::<Verb>().expect(text), verb, "{text:?}");
    }
    for verb in [Verb::Insert, Verb::Overwrite, Verb::Upsert, Verb::Delete] {
        assert_eq!(verb.as_str().parse::<Verb>().unwrap(), verb);
        assert_eq!(verb.word().parse::<Verb>().unwrap(), verb);
    }
    for text in [
        "sideways",
        "",
        "insert into t",
        "upsert by (id)",
        "delete into",
        "insert overwrite from",
        "select",
    ] {
        let refused = text.parse::<Verb>().expect_err(text).to_string();
        assert!(
            refused.contains("write verb") || refused.contains("end of the expression"),
            "{text:?}: {refused}"
        );
    }
}

#[test]
fn an_ordering_key_reads_as_the_grammar_spells_one() {
    let a = || Term::column("a");
    for (text, key) in [
        ("a", Ordering::asc(a())),
        ("a asc", Ordering::asc(a())),
        ("a ASC NULLS LAST", Ordering::asc(a())),
        ("a desc", Ordering::desc(a())),
        ("a desc nulls first", Ordering::desc(a()).nulls_first(true)),
        ("a nulls first", Ordering::asc(a()).nulls_first(true)),
    ] {
        assert_eq!(text.parse::<Ordering>().expect(text), key, "{text:?}");
    }
    // Any term the grammar reads is a key, printed back as it was read.
    let computed: Ordering = "price * 2 desc nulls first".parse().unwrap();
    assert!(computed.is_descending() && computed.is_nulls_first());
    assert_eq!(computed.to_string(), "price * 2 desc nulls first");
    for key in [
        Ordering::asc(a()),
        Ordering::desc(a()),
        Ordering::desc(a()).nulls_first(true),
        Ordering::asc(a()).nulls_first(true),
    ] {
        assert_eq!(key.to_string().parse::<Ordering>().unwrap(), key);
    }
    for text in [
        "",
        "a, b",
        "a descending",
        "a nulls",
        "a desc first",
        "a asc desc",
    ] {
        assert!(text.parse::<Ordering>().is_err(), "{text:?} read as a key");
    }
    // Neither direction word is reserved: alone, it names a column.
    assert_eq!(
        "desc".parse::<Ordering>().unwrap(),
        Ordering::asc(Term::column("desc"))
    );
}

#[test]
fn an_ordering_record_reads_its_nulls_flag_under_either_spelling() {
    use yggdryl::expression::Ordering;
    use yggdryl::{Scalar, SortOptions};

    let snake = Scalar::from_struct([
        ("term", Scalar::from("price")),
        ("descending", Scalar::from(true)),
        ("nulls_first", Scalar::from(true)),
    ])
    .expect("a record");
    let camel = Scalar::from_struct([
        ("term", Scalar::from("price")),
        ("descending", Scalar::from(true)),
        ("nullsFirst", Scalar::from(true)),
    ])
    .expect("a record");
    let expected = Ordering::new(
        "price".parse().expect("a term"),
        SortOptions::descending().with_nulls_first(true),
    );
    assert_eq!(Ordering::from_scalar(&snake).expect("snake case"), expected);
    assert_eq!(Ordering::from_scalar(&camel).expect("camel case"), expected);
    let refused = Scalar::from_struct([
        ("term", Scalar::from("price")),
        ("nulls", Scalar::from(true)),
    ])
    .expect("a record");
    assert!(
        Ordering::from_scalar(&refused)
            .unwrap_err()
            .to_string()
            .contains("$.nulls")
    );
    // A flag is a boolean or the text the crate's one boolean table reads.
    let spelled = Scalar::from_struct([
        ("term", Scalar::from("price")),
        ("descending", Scalar::from("yes")),
        ("nulls_first", Scalar::from("0")),
    ])
    .expect("a record");
    assert_eq!(
        Ordering::from_scalar(&spelled).expect("spelled flags"),
        Ordering::new("price".parse().expect("a term"), SortOptions::descending())
    );
    for flag in [Scalar::from("maybe"), Scalar::from(1_i64)] {
        let record = Scalar::from_struct([("term", Scalar::from("price")), ("descending", flag)])
            .expect("a record");
        let refused = Ordering::from_scalar(&record).unwrap_err().to_string();
        assert!(refused.contains("$.descending"), "{refused}");
    }
}
