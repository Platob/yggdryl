//! Statistics pruning, and what the cast rule buys when it can fire.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::expressions::{BoundColumn, ColumnStats, StatsSource};
use yggdryl::{DataType, Expr, Field, Value};

use super::schema;

/// A synthetic manifest: one file's statistics per entry.
struct File {
    lowest: i64,
    highest: i64,
}

impl StatsSource for File {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        match column.name() {
            "id" => Some(ColumnStats::range(
                Value::I64(self.lowest),
                Value::I64(self.highest),
            )),
            "venue" => Some(ColumnStats::constant(Value::from(
                if self.lowest % 2 == 0 { "XNAS" } else { "XNYS" },
            ))),
            _ => None,
        }
    }
}

/// A synthetic manifest of `count` files, each covering a thousand ids.
fn manifest(count: i64) -> Vec<File> {
    (0..count)
        .map(|index| File {
            lowest: index * 1_000,
            highest: index * 1_000 + 999,
        })
        .collect()
}

pub fn benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let files = manifest(10_000);

    let mut group = criterion.benchmark_group("expression_prune");
    group.throughput(criterion::Throughput::Elements(files.len() as u64));

    for (name, text) in [
        ("partition_equality", "venue = 'XNAS'"),
        ("range", "id BETWEEN 5000 AND 5999"),
        ("set", "id IN (10, 2000, 400000)"),
        ("conjunction", "venue = 'XNAS' AND id > 900000"),
    ] {
        let predicate = text
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                black_box(&files)
                    .iter()
                    .filter(|file| predicate.evaluate_stats(*file).is_possible())
                    .count()
            });
        });
    }

    // What the cast rule buys, as a difference rather than an assertion: a cast
    // wrapping a column destroys pruning outright, while the same comparison
    // against a converted literal prunes perfectly.
    let narrow = DataType::from_fields([DataType::Int32.nullable_field("id")])
        .expect("a one-column struct")
        .required_field("row");
    let narrow_files: Vec<File> = manifest(10_000);
    for (name, text) in [
        ("cast_moved_to_literal", "CAST(id AS int64) = 500000"),
        // The same question the rule cannot prove exact, so the cast stays and
        // nothing prunes - which is the cost the rule avoids when it can.
        ("cast_left_on_column", "CAST(id AS float64) = 500000"),
    ] {
        let predicate = text
            .parse::<Expr>()
            .expect("parses")
            .bind(&narrow)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                black_box(&narrow_files)
                    .iter()
                    .filter(|file| {
                        predicate
                            .evaluate_stats(&NarrowFile { inner: file })
                            .is_possible()
                    })
                    .count()
            });
        });
    }
    group.finish();

    // The residual: what a source settles is what the rows never see, so the
    // cost of computing it is what the whole layered read is paying for.
    let mut group = criterion.benchmark_group("expression_residual");
    group.throughput(criterion::Throughput::Elements(files.len() as u64));
    let predicate = "venue = 'XNAS' AND id BETWEEN 5000 AND 5999"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect("a predicate");
    group.bench_function("two_conjuncts", |bencher| {
        bencher.iter(|| {
            black_box(&files)
                .iter()
                .filter_map(|file| predicate.residual(file))
                .map(|residual| residual.len())
                .sum::<usize>()
        });
    });
    group.finish();
}

/// The same statistics under a one-column narrow schema.
struct NarrowFile<'files> {
    inner: &'files File,
}

impl StatsSource for NarrowFile<'_> {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        let _ = Field::new("unused", DataType::Null, true);
        (column.name() == "id").then(|| {
            ColumnStats::range(
                Value::I64(self.inner.lowest),
                Value::I64(self.inner.highest),
            )
        })
    }
}
