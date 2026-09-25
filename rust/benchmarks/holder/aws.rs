//! What an AWS identity costs beside the request it signs.
//!
//! A session resolves once and answers from what it holds, so the only
//! per-request cost is the cached answer; the parse of the shared files is
//! paid once per session and grows with the number of profiles the machine
//! has. Both are measured here so a regression in either shows beside the
//! request counts the accounting tests hold.

use std::hint::black_box;
use std::time::SystemTime;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::aws::{Credentials, Session};

/// The profiles the largest configuration file holds.
const PROFILES: usize = crate::bench_profile::corpus(256, 8);

/// A configuration file of `count` profiles, each with a region, an
/// endpoint and an indented `s3` table, as a real one has.
fn configuration(count: usize) -> String {
    let mut text = String::from("[default]\nregion = us-east-1\n\n");
    for index in 0..count {
        text.push_str(&format!(
            "[profile desk-{index}]\nregion = eu-west-{}\nendpoint_url = https://s3.desk-{index}.example.io\ns3 =\n  addressing_style = path\n  payload_signing_enabled = false\n\n",
            index % 3 + 1
        ));
    }
    text
}

pub(crate) fn identity_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("aws_session");

    // The per-request cost: a set in hand, answered without a walk.
    let session = Session::new()
        .with_environment(false)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
    let now = SystemTime::now();
    session.credentials(now).expect("an explicit set");
    group.bench_function("credentials_cached", |bencher| {
        bencher.iter(|| {
            black_box(
                session
                    .credentials(black_box(now))
                    .expect("the set in hand"),
            )
        });
    });

    // The per-session cost: both files parsed once and one profile found.
    for count in [1, PROFILES] {
        let text = configuration(count);
        let wanted = format!("desk-{}", count - 1);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("profile_of", count),
            &text,
            |bencher, text| {
                bencher.iter(|| {
                    let session = Session::new()
                        .with_environment(false)
                        .with_config_text(text.as_str())
                        .with_profile(wanted.as_str());
                    black_box(session.profile().expect("the profile").region().is_some())
                });
            },
        );
    }

    // The endpoint arithmetic a client does at construction.
    let session = Session::new()
        .with_environment(false)
        .with_use_fips_endpoint(true);
    group.bench_function("sts_endpoint", |bencher| {
        bencher.iter(|| black_box(session.sts_endpoint(black_box("eu-west-3"))));
    });
    group.finish();
}
