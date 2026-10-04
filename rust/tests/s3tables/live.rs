//! `rust/src/s3tables/`, whole, against the live service: one table's life
//! in a table bucket of the operator's own account, commit included - the
//! check the fake, written from the client's own reading, cannot make.
//! Ignored, because it needs an account, and never passing silently: unset,
//! the failure names the variable that turns it on.
//!
//! ```bash
//! YGGDRYL_S3TABLES_PROFILE=<a signed-in profile> \
//!   cargo test -p yggdryl --features s3tables --test s3tables live -- --ignored --nocapture
//! ```
//!
//! `YGGDRYL_S3TABLES_REGION` names a region the profile does not, and
//! `YGGDRYL_S3TABLES_PREFIX` the prefix of the table bucket's name, which
//! is otherwise `yggdryl-dev-trial`. Whatever a step leaves behind - the
//! table, the namespace, the table bucket - is deleted before the test
//! reports, however it ended.

use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{SystemTime, UNIX_EPOCH};

use yggdryl::aws::Session;
use yggdryl::s3::S3Options;
use yggdryl::s3tables::S3Tables;
use yggdryl::{Arn, IOBase, Url};

use crate::mod_::schema;

/// The variable that turns the run on, naming the profile it signs as.
const PROFILE: &str = "YGGDRYL_S3TABLES_PROFILE";
const NAMESPACE: &str = "trial";
const TABLE: &str = "events";
const RENAMED: &str = "events_renamed";

/// What one run holds: who signs, what it made, and every step that failed.
struct Trial {
    session: Session,
    tables: S3Tables,
    /// The table bucket's name, unique to this run.
    name: String,
    /// The table bucket's ARN, once its creation answered one.
    bucket: Option<Arn>,
    failures: Vec<String>,
}

impl Trial {
    /// Print one step's outcome, keep its failure, and hand its answer on.
    fn step<T>(
        &mut self,
        step: &str,
        subject: impl Display,
        result: yggdryl::Result<T>,
    ) -> Option<T> {
        match result {
            Ok(answer) => {
                eprintln!("{step}: {subject}: ok");
                Some(answer)
            }
            Err(error) => {
                eprintln!("{step}: {subject}: FAILED: {error}");
                self.failures.push(format!("{step}: {error}"));
                None
            }
        }
    }

    /// Record a step that answered, and answered the wrong thing.
    fn wrong(&mut self, step: &str, what: String) {
        eprintln!("{step}: FAILED: {what}");
        self.failures.push(format!("{step}: {what}"));
    }
}

/// A number no other run shares: the process, and the clock.
fn unique() -> u64 {
    let clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    // The low 64 bits are the ones that move.
    #[allow(clippy::cast_possible_truncation)]
    let clock = clock as u64;
    // The process is spread over every bit, so any eight hex digits of the
    // result carry both.
    clock ^ u64::from(std::process::id()).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The steps, each stopping the run at its first failure: the cleanup that
/// follows is the same wherever it stopped.
fn run(trial: &mut Trial) {
    let tables = trial.tables.clone();
    let name = trial.name.clone();

    let Some(region) = trial.step("resolve region", &name, tables.region_of(None)) else {
        return;
    };
    let Some(endpoint) = trial.step("resolve endpoint", &region, tables.endpoint_url(&region))
    else {
        return;
    };
    eprintln!("endpoint: {endpoint}");

    let created = tables.create_table_bucket(&name);
    let Some(bucket) = trial.step("create table bucket", &name, created) else {
        return;
    };
    trial.bucket = Some(bucket.clone());
    eprintln!("table bucket: {bucket}");

    let described = tables.get_table_bucket(&bucket);
    let Some(described) = trial.step("get table bucket", &bucket, described) else {
        return;
    };
    if described.name() != name || described.arn() != &bucket {
        trial.wrong(
            "get table bucket",
            format!("expected {name} at {bucket}, got {described:?}"),
        );
        return;
    }

    let listed = tables.table_buckets().collect::<yggdryl::Result<Vec<_>>>();
    let Some(listed) = trial.step("list table buckets", &region, listed) else {
        return;
    };
    if !listed.iter().any(|held| held.arn() == &bucket) {
        trial.wrong(
            "list table buckets",
            format!("expected {bucket} among {} buckets, got none", listed.len()),
        );
        return;
    }

    let namespace = tables.create_namespace(&bucket, NAMESPACE);
    if trial
        .step("create namespace", NAMESPACE, namespace)
        .is_none()
    {
        return;
    }

    let described = tables.get_namespace(&bucket, NAMESPACE);
    let Some(described) = trial.step("get namespace", NAMESPACE, described) else {
        return;
    };
    if described.name() != NAMESPACE {
        trial.wrong(
            "get namespace",
            format!("expected {NAMESPACE}, got {described:?}"),
        );
        return;
    }
    let listed = tables
        .namespaces(&bucket)
        .collect::<yggdryl::Result<Vec<_>>>();
    let Some(listed) = trial.step("list namespaces", &bucket, listed) else {
        return;
    };
    if listed != [described] {
        trial.wrong(
            "list namespaces",
            format!("expected the one namespace {NAMESPACE}, got {listed:?}"),
        );
        return;
    }

    let table = tables.create_table(&bucket, NAMESPACE, TABLE, Some(&schema()));
    let Some(created) = trial.step("create table", TABLE, table) else {
        return;
    };
    eprintln!("table: {}", created.arn());

    let table = tables.get_table(&bucket, NAMESPACE, TABLE);
    let Some(table) = trial.step("get table", created.arn(), table) else {
        return;
    };
    for namespace in [None, Some(NAMESPACE)] {
        let listed = tables
            .tables(&bucket, namespace)
            .collect::<yggdryl::Result<Vec<_>>>();
        let Some(listed) = trial.step("list tables", &bucket, listed) else {
            return;
        };
        let names: Vec<(&str, &str)> = listed
            .iter()
            .map(|summary| (summary.namespace(), summary.name()))
            .collect();
        if names != [(NAMESPACE, TABLE)] || listed[0].arn() != table.arn() {
            trial.wrong(
                "list tables",
                format!(
                    "expected {NAMESPACE}.{TABLE} at {}, got {listed:?}",
                    table.arn()
                ),
            );
            return;
        }
    }
    eprintln!("warehouse location: {}", table.warehouse_location());
    eprintln!(
        "metadata location: {}",
        table
            .metadata_location()
            .map_or_else(|| "none".to_owned(), ToString::to_string)
    );

    let renamed = tables.rename_table(
        &bucket,
        NAMESPACE,
        TABLE,
        None,
        Some(RENAMED),
        Some(table.version_token()),
    );
    if trial.step("rename table", RENAMED, renamed).is_none() {
        return;
    }

    // The commit: read the metadata file the service names, write the next
    // one beside it, and name that one under the token just read.
    let before = tables.get_table_metadata_location(&bucket, NAMESPACE, RENAMED);
    let Some(before) = trial.step("get metadata location", RENAMED, before) else {
        return;
    };
    let Some(current) = before.metadata_location().cloned() else {
        trial.wrong(
            "get metadata location",
            "expected a metadata file for a table created with a schema, got none".to_owned(),
        );
        return;
    };
    let options = S3Options::default()
        .with_session(trial.session.clone())
        .with_region(region);
    let read = yggdryl::s3::file_with(&current.to_string(), options.clone())
        .and_then(|file| file.read_all_bytes())
        .and_then(|bytes| Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?));
    let Some(mut document) = trial.step("read metadata", &current, read) else {
        return;
    };
    document["properties"]["yggdryl.trial"] = "1".into();

    let stamp = unique();
    let next = format!(
        "{}/metadata/00001-{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}.metadata.json",
        before
            .warehouse_location()
            .to_string()
            .trim_end_matches('/'),
        stamp >> 32,
        (stamp >> 16) & 0xffff,
        stamp & 0xfff,
        (stamp >> 12) & 0xfff,
        stamp & 0xffff_ffff_ffff
    );
    let written = serde_json::to_vec(&document)
        .map_err(yggdryl::Error::from)
        .and_then(|bytes| {
            let mut file = yggdryl::s3::file_with(&next, options)?;
            file.write_all_bytes(&bytes)
        });
    if trial.step("write metadata", &next, written).is_none() {
        return;
    }

    let committed = Url::from_str(&next).and_then(|location| {
        tables.update_table_metadata_location(
            &bucket,
            NAMESPACE,
            RENAMED,
            before.version_token(),
            &location,
        )
    });
    let Some(committed) = trial.step("update metadata location", &next, committed) else {
        return;
    };

    let after = tables.get_table_metadata_location(&bucket, NAMESPACE, RENAMED);
    let Some(after) = trial.step("get metadata location again", RENAMED, after) else {
        return;
    };
    if after
        .metadata_location()
        .map(ToString::to_string)
        .as_deref()
        != Some(next.as_str())
    {
        trial.wrong(
            "get metadata location again",
            format!(
                "expected the metadata location {next}, got {:?}",
                after.metadata_location().map(ToString::to_string)
            ),
        );
    }
    if after.version_token() == before.version_token()
        || after.version_token() != committed.version_token()
    {
        trial.wrong(
            "get metadata location again",
            "expected the version token the commit answered, and another than the one it was \
             made under"
                .to_owned(),
        );
    }
}

/// Delete everything the run may have made, whatever it reached: a delete
/// of what is not there is success, so each level is asked once.
fn clean_up(trial: &mut Trial) {
    let tables = trial.tables.clone();
    // A creation whose answer was lost left a bucket this run never learnt
    // the ARN of: it is found by its name, and a listing that cannot say
    // whether it is there is a failure - it may be left behind.
    let mut bucket = trial.bucket.clone();
    if bucket.is_none() {
        for held in tables.table_buckets() {
            match held {
                Ok(held) if held.name() == trial.name => {
                    bucket = Some(held.arn().clone());
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    let name = trial.name.clone();
                    trial.wrong(
                        "clean up",
                        format!("could not list the table buckets to find {name}: {error}"),
                    );
                    return;
                }
            }
        }
    }
    let Some(bucket) = bucket else {
        eprintln!("clean up: {}: no table bucket to remove", trial.name);
        return;
    };
    for table in [RENAMED, TABLE] {
        let removed = tables.remove_table(&bucket, NAMESPACE, table, None);
        trial.step("remove table", table, removed);
    }
    let removed = tables.remove_namespace(&bucket, NAMESPACE);
    trial.step("remove namespace", NAMESPACE, removed);
    let removed = tables.remove_table_bucket(&bucket);
    trial.step("remove table bucket", &bucket, removed);
}

#[test]
#[ignore = "contacts AWS: set YGGDRYL_S3TABLES_PROFILE to a signed-in profile"]
fn a_table_is_created_updated_and_deleted_on_the_live_service() {
    let Ok(profile) = std::env::var(PROFILE) else {
        panic!(
            "{PROFILE} is not set: name a signed-in AWS profile to run against the live service"
        );
    };
    let session = Session::new().with_profile(profile);
    let mut tables = S3Tables::new(session.clone());
    if let Ok(region) = std::env::var("YGGDRYL_S3TABLES_REGION") {
        tables = tables.with_region(region);
    }
    let prefix =
        std::env::var("YGGDRYL_S3TABLES_PREFIX").unwrap_or_else(|_| "yggdryl-dev-trial".to_owned());
    // Eight hex digits of the process and the clock.
    let name = format!("{prefix}-{:08x}", unique() & 0xffff_ffff);
    let mut trial = Trial {
        session,
        tables,
        name,
        bucket: None,
        failures: Vec::new(),
    };

    // A panic below is a failure like any other: the cleanup still runs.
    if catch_unwind(AssertUnwindSafe(|| run(&mut trial))).is_err() {
        trial.wrong("run", "a step panicked".to_owned());
    }
    if catch_unwind(AssertUnwindSafe(|| clean_up(&mut trial))).is_err() {
        trial.wrong("clean up", "a deletion panicked".to_owned());
    }
    assert!(
        trial.failures.is_empty(),
        "the live run failed:\n{}",
        trial.failures.join("\n")
    );
}
