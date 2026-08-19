//! What the `{{ }}` guard costs a document that has none, and what
//! substitution costs the documents that do.
//!
//! The first pair is the one that matters. Substitution is a feature almost no
//! document uses, so `none/off` against `none/on` is the whole question: the
//! delta between them is the cost of the linear `{{` scan and nothing else, and
//! if it is not near zero the guard is not doing its job.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Value;
use yggdryl::text::{self, Format, Loading, Placeholders};

/// Entries per benchmarked document, so all three are the same size.
const ENTRIES: usize = 256;

pub fn placeholder_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/placeholder");

    let placeholders = (0..ENTRIES).fold(Placeholders::new(), |held, index| {
        held.with_variable(format!("VAR_{index}"), Value::from("resolved"))
    });
    let off = Loading::new();
    let on = Loading::new().with_placeholders(placeholders);

    for (label, document) in [
        // No placeholder anywhere: the case the guard exists for.
        ("none", document(0)),
        // A few scalars carry one.
        ("few", document(ENTRIES / 32)),
        // Most of them do.
        ("most", document(ENTRIES)),
    ] {
        for (state, loading) in [("off", &off), ("on", &on)] {
            group.bench_function(format!("{label}/{state}"), |bencher| {
                bencher.iter(|| {
                    black_box(text::from_str_with(&document, Format::Json, loading).unwrap())
                });
            });
        }
    }
    group.finish();
}

/// A JSON document of [`ENTRIES`] scalars, `filled` of them placeholders.
fn document(filled: usize) -> String {
    let mut text = String::from("{");
    for index in 0..ENTRIES {
        if index > 0 {
            text.push(',');
        }
        if index < filled {
            text.push_str(&format!("\"key_{index}\":\"{{{{ VAR_{index} }}}}\""));
        } else {
            text.push_str(&format!("\"key_{index}\":\"plain value {index}\""));
        }
    }
    text.push('}');
    text
}
