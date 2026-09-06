use std::collections::HashMap;
use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field, FixCode, FixCodeValue, Version};

/// A code set of `count` members, each carrying a description tier 3 reads.
fn vocabulary(count: usize) -> Field {
    let codes: Vec<FixCode> = (0..count)
        .map(|index| {
            FixCode::new(format!("Member{index:04}"), format!("{index:04}"))
                .with_description(format!("Member number {index} (M{index:04})"))
        })
        .collect();
    let mut field = DataType::Utf8.nullable_field("Vocabulary");
    field.as_fix_mut().set_tag(9995).expect("a static tag");
    field
        .as_fix_mut()
        .set_codes(&codes)
        .expect("a valid code set");
    field
}

pub fn benchmarks(criterion: &mut Criterion) {
    // Three sizes, because a FIX code set is usually small and occasionally
    // not: `Side` has ten members and `PartyRole` has hundreds.
    let small = vocabulary(10);
    let medium = vocabulary(60);
    let large = vocabulary(300);
    let mut group = criterion.benchmark_group("fix/codes");

    for (label, field) in [("10", &small), ("60", &medium), ("300", &large)] {
        let view = field.as_fix();
        let last = format!("{:04}", field.as_fix().codes().count() - 1);

        // Tier 1 stops at the match, so the first and last member bracket it.
        group.bench_function(format!("{label}/value_first"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box("0000")));
        });
        group.bench_function(format!("{label}/value_last"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box(last.as_str())));
        });
        group.bench_function(format!("{label}/value_miss"), |bencher| {
            bencher.iter(|| black_box(&view).code(black_box("absent")));
        });
        // Tier 2 runs the whole set whatever it finds, because two codes
        // folding to one spelling must answer nothing rather than the first.
        group.bench_function(format!("{label}/name_folded"), |bencher| {
            bencher.iter(|| black_box(&view).code_value(black_box("member_0000")));
        });

        // The baseline: the same document as a map. Built once and read many
        // times it wins, which is what the two rows are for - a caller
        // resolving one spelling pays the build, and one resolving a million
        // should build the map itself.
        let map: HashMap<&str, FixCodeValue<'_>> = view
            .codes()
            .map(|code| {
                let code = code.expect("a canonical document");
                (code.value(), code)
            })
            .collect();
        group.bench_function(format!("{label}/baseline_map_hit"), |bencher| {
            bencher.iter(|| black_box(&map).get(black_box("0000")));
        });
        group.bench_function(format!("{label}/baseline_map_build_and_hit"), |bencher| {
            bencher.iter(|| {
                let built: HashMap<&str, FixCodeValue<'_>> = black_box(&view)
                    .codes()
                    .filter_map(|code| code.ok().map(|code| (code.value(), code)))
                    .collect();
                black_box(built.get(black_box("0000")).copied())
            });
        });
    }

    // A version filter costs one comparison per code reached, not a second
    // walk.
    let view = large.as_fix();
    group.bench_function("300/value_at", |bencher| {
        bencher.iter(|| black_box(&view).code_value_at(black_box(Version::MAX), black_box("0150")));
    });
    // Writing renders the whole document once, which is what a generator pays.
    let codes: Vec<FixCode> = view
        .codes()
        .map(|code| FixCode::from(code.expect("a canonical document")))
        .collect();
    group.bench_function("300/set_codes", |bencher| {
        bencher.iter_batched_ref(
            || DataType::Utf8.nullable_field("Vocabulary"),
            |field| {
                field
                    .as_fix_mut()
                    .set_codes(black_box(&codes))
                    .expect("a valid code set");
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}
