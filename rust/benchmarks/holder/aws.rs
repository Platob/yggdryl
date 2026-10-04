//! What an AWS identity costs beside the request it signs.
//!
//! A session resolves once and answers from what it holds, so the only
//! per-request cost is the cached answer; the parse of the shared files is
//! paid once per session and grows with the number of profiles the machine
//! has. A walk - the first ask, the one after a store's refusal, the one a
//! set nearing its end causes - reads the files from disk when they moved,
//! and passes over a set a store refused: each is measured on files a
//! benchmark writes, so a regression in any shows beside the request counts
//! the accounting tests hold. Signing is the one cost every attempt of a
//! request signed through `Request::with_sigv4` pays: the cached signer, the
//! SHA-256 of the body and the canonical request, measured on a 1 KiB JSON
//! `POST` to a catalog path (built with `internals`, which reaches one
//! attempt's signing at a stated instant). Under `s3tables`, what holding a
//! table bucket's location costs sits beside them: its catalog and a
//! namespace are descriptions, built from the properties with no request.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::aws::{Credentials, Session};
use yggdryl::{Arn, ArnPartition};

/// The profiles the largest configuration file holds.
const PROFILES: usize = crate::bench_profile::corpus(256, 8);

/// A directory of this benchmark's own under the platform's temporary one,
/// holding `files` as `(name, text)`.
fn directory(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("yggdryl-bench-aws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a scratch directory");
    for (file, text) in files {
        let target = path.join(file);
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("a parent directory");
        std::fs::write(target, text).expect("a written file");
    }
    path
}

/// A session reading the files under `directory` and nothing else: its own
/// variables, no metadata service.
fn reading(directory: &Path) -> Session {
    Session::new()
        .with_variables([(
            "BOTO_CONFIG",
            directory.join("no-such-boto.cfg").display().to_string(),
        )])
        .with_directory(directory)
        .with_metadata_disabled(true)
}

/// A temporary set as `aws login`, a portal or a tool dumps one.
const DUMPED: &str = "[default]\naws_access_key_id = ASIABENCHDUMPED\naws_secret_access_key = dumped-secret\n\
aws_session_token = dumped-token\nx_security_token_expires = 2099-01-01T00:00:00Z\n";

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

    // A first walk: both files read from disk and parsed, the dumped set
    // read with its expiry and admitted.
    let dumped = directory("walk", &[("credentials", DUMPED)]);
    group.bench_function("walk_shared_credentials_file", |bencher| {
        bencher.iter(|| {
            let session = reading(&dumped);
            black_box(
                session
                    .credentials(now)
                    .expect("a walk")
                    .expect("the dumped set"),
            )
        });
    });

    // The walk a store's refusal causes: the files read again whatever their
    // versions say, the chain walked again.
    let session = reading(&dumped);
    session.credentials(now).expect("a first walk");
    group.bench_function("walk_after_invalidate", |bencher| {
        bencher.iter(|| {
            session.invalidate();
            black_box(
                session
                    .credentials(now)
                    .expect("a walk")
                    .expect("the dumped set"),
            )
        });
    });

    // A refused key passed over by name to the configuration file's set.
    let refused = directory(
        "refused",
        &[
            ("credentials", DUMPED),
            (
                "config",
                "[default]\naws_access_key_id = AKIABENCHCONFIG\naws_secret_access_key = config-secret\n",
            ),
        ],
    );
    let session = reading(&refused);
    session.credentials(now).expect("a first walk");
    session.invalidate_if("ASIABENCHDUMPED");
    group.bench_function("walk_past_refused_key", |bencher| {
        bencher.iter(|| {
            session.invalidate();
            black_box(
                session
                    .credentials(now)
                    .expect("a walk")
                    .expect("the config set"),
            )
        });
    });

    // A console sign-in `aws login` filed, answered from its cache with no
    // request: the cache read, its document parsed, its set admitted.
    let signed_in = directory(
        "login",
        &[
            (
                "config",
                "[profile console]\nlogin_session = arn:aws:iam::0123456789012:user/Admin\nregion = eu-west-3\n",
            ),
            (
                "login/cache/36db1d138ff460920374e4c3d8e01f53f9f73537e89c88d639f68393df0e2726.json",
                r#"{"accessToken":{"accessKeyId":"ASIABENCHLOGIN","secretAccessKey":"login-secret","sessionToken":"login-token","accountId":"012345678901","expiresAt":"2099-01-01T00:00:00Z"},"tokenType":"aws_sigv4","refreshToken":"login-refresh","clientId":"arn:aws:signin:::devtools/same-device","dpopKey":"-----BEGIN EC PRIVATE KEY-----\nnot read while the set lasts\n-----END EC PRIVATE KEY-----\n"}"#,
            ),
        ],
    );
    let session = reading(&signed_in).with_profile("console");
    session.credentials(now).expect("a first walk");
    group.bench_function("walk_console_sign_in_cached", |bencher| {
        bencher.iter(|| {
            session.invalidate();
            black_box(
                session
                    .credentials(now)
                    .expect("a walk")
                    .expect("the cached set"),
            )
        });
    });

    // The partition a role's ARN names and the host it is traded at.
    group.bench_function("arn_partition_host", |bencher| {
        bencher.iter(|| {
            let arn = Arn::from_str(black_box("arn:aws-cn:iam::123456789012:role/lake-reader"))
                .expect("an ARN");
            let partition = ArnPartition::from_arn(&arn).expect("a partition");
            black_box(partition.service_host("sts", partition.global_region(), false, true))
        });
    });
    // What holding a table bucket's location costs: its catalog and a
    // namespace below it are descriptions, built from the properties - who
    // signs among them - with no request and no file read.
    #[cfg(feature = "s3tables")]
    {
        let properties = yggdryl::Properties::new()
            .with_property("access_key_id", "AKIAIOSFODNN7EXAMPLE")
            .with_property("secret_access_key", "wJalrXUtnFEMI");
        let namespace = yggdryl::Url::from_str("s3tables://lake/desk").expect("a location");
        let bucket = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")
            .expect("a table bucket's ARN");
        group.bench_function("s3tables_namespace_by_location", |bencher| {
            bencher.iter(|| {
                black_box(
                    yggdryl::holder::Holder::from_url(black_box(&namespace), &properties)
                        .expect("a namespace"),
                )
            });
        });
        group.bench_function("s3tables_catalog_by_arn", |bencher| {
            bencher.iter(|| {
                black_box(
                    yggdryl::holder::Holder::from_url(black_box(&bucket), &properties)
                        .expect("a catalog"),
                )
            });
        });
    }
    // What signing one attempt costs: a 1 KiB JSON `POST` whose path carries
    // an encoded bucket ARN, so the canonical URI is made by the rule every
    // service outside the S3 family signs by.
    #[cfg(feature = "internals")]
    {
        let signing = Session::new()
            .with_environment(false)
            .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"));
        let url = yggdryl::Url::from_str(
            "https://s3tables.eu-west-3.amazonaws.com/iceberg/v1/\
             arn%3Aaws%3As3tables%3Aeu-west-3%3A123456789012%3Abucket%2Flake/namespaces/a%1Fb/tables",
        )
        .expect("a URL");
        let mut headers = yggdryl::http::Headers::new();
        headers
            .insert("content-type", "application/json")
            .expect("a header");
        let body = format!(r#"{{"name":"trades","padding":"{}"}}"#, "x".repeat(990));
        group.throughput(Throughput::Bytes(body.len() as u64));
        group.bench_function("sign_post_1kib", |bencher| {
            bencher.iter(|| {
                black_box(
                    yggdryl::internals::aws_request::signed_headers(
                        &signing,
                        "s3tables",
                        "eu-west-3",
                        yggdryl::http::Method::Post,
                        &url,
                        &headers,
                        Some(black_box(body.as_bytes())),
                        now,
                    )
                    .expect("signed headers"),
                )
            });
        });
    }
    group.finish();
    for scratch in [dumped, refused, signed_in] {
        let _ = std::fs::remove_dir_all(scratch);
    }
}
