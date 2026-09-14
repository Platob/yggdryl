//! Plans: the sections they spell, the locations they name, the sequences
//! they form, and what running one does to a stream and to a store.

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use super::{IntoPlan, Location, Ordering, Plan, Source, Target, Verb, Write};
use crate::expression::{Expression, Selector, Term};
use crate::{DataType, Field, Scalar, Url};

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
        .knob::<usize>("batch_row_size", "a row count")
        .unwrap_err()
        .to_string();
    assert!(error.contains("batch_row_size"), "{error}");
    assert!(error.contains("ten"), "{error}");
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
    let root = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("ccy"),
        DataType::Float64.nullable_field("price"),
    ])
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

#[cfg(feature = "arrow")]
mod streams {
    use super::*;
    use crate::expression::arrow::{collected, one_batch};
    use crate::holder::Holder;
    use crate::media::IORecordOptions;
    use crate::{IOBase, MediaType, MimeType};
    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};

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
        let schema = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("name"),
        ])
        .unwrap()
        .required_field("trades");
        RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema).unwrap(),
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
    fn a_sliced_reader_walks_batch_boundaries_without_copying_whole_batches() {
        let batches = [
            trades(&[(1, "a"), (2, "b")]),
            trades(&[(3, "c"), (4, "d")]),
            trades(&[(5, "e"), (6, "f")]),
        ];
        let stream = || crate::arrow::batch_reader(batches[0].schema(), batches.clone());
        let sliced: Vec<RecordBatch> = crate::arrow::sliced_reader(stream(), 1, Some(3))
            .map(|batch| batch.unwrap())
            .collect();
        assert_eq!(
            sliced.len(),
            2,
            "the run spans two batches and skips the third"
        );
        let held: Vec<i64> = sliced.iter().flat_map(ids).collect();
        assert_eq!(held, [2, 3, 4]);
        let held: Vec<i64> = crate::arrow::sliced_reader(stream(), 5, None)
            .flat_map(|batch| ids(&batch.unwrap()))
            .collect();
        assert_eq!(held, [6]);
        let held: Vec<i64> = crate::arrow::sliced_reader(stream(), 9, Some(2))
            .flat_map(|batch| ids(&batch.unwrap()))
            .collect();
        assert!(held.is_empty());
        let held: Vec<i64> = crate::arrow::sliced_reader(stream(), 0, Some(0))
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
        let holder = target.holder(None).unwrap();
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
            .holder(None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("raw.trades"), "{error}");
        let base: Url = scratch.url("").parse().unwrap();
        let held = Target::parse("plain").unwrap().holder(Some(&base)).unwrap();
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
        assert_eq!(coded.codec(), crate::Codec::Gzip);
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
                    Scalar::from_record([("id", Scalar::from(id)), ("name", Scalar::from(name))])
                        .unwrap()
                })
        };
        let filter: crate::Filter = "id > 1".parse().unwrap();
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
        let back = crate::expression::Records::from_arrow_reader(one_batch(&batch)).unwrap();
        assert_eq!(back.field().fields()[0].name(), "id");
        assert_eq!(back.collect_rows().unwrap().len(), 3);
        // Without a schema and without a record there is nothing to bind to.
        let error = selector
            .apply_records(None, std::iter::empty::<Scalar>())
            .unwrap_err()
            .to_string();
        assert!(error.contains("schema"), "{error}");
        // A declared schema binds even an empty stream.
        let schema = DataType::from_fields([DataType::Int64.required_field("id")])
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
}
