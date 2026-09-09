//! Reading a property map written in somebody else's vocabulary.
//!
//! A caller reaching a store through this crate starts from a PyIceberg
//! catalog's properties, from PyArrow's `S3FileSystem` arguments, or from each
//! store's own environment variable names - many vocabularies for one set of
//! knobs. What these check is that each of them lands on the same knob, that a
//! name two stores share reaches both, that a value which will not parse is
//! heard here rather than at the store, and that a knob this client cannot
//! honor is refused rather than dropped.

use std::time::Duration;

use crate::Url;
use crate::holder::object::client::Client;
use crate::holder::object::{
    AssumedRole, AzureOptions, Credentials, Encryption, ObjectOptions, Provider,
};

#[test]
fn a_pyiceberg_catalogs_properties_reach_every_knob_they_name() {
    let options = ObjectOptions::from_properties([
        // Most of a catalog's properties have nothing to do with a store.
        ("type", "rest"),
        ("uri", "https://catalog.example.io"),
        ("warehouse", "s3://trades/lake"),
        ("token", "a-catalog-bearer-token-that-is-not-a-session"),
        // These do.
        ("s3.endpoint", "http://localhost:9000"),
        ("s3.access-key-id", "minioadmin"),
        ("s3.secret-access-key", "minioadmin-secret"),
        ("s3.session-token", "a-session-token"),
        ("s3.region", "eu-west-1"),
        ("s3.force-virtual-addressing", "false"),
        ("s3.connect-timeout", "2.5"),
        ("s3.request-timeout", "30"),
        ("s3.proxy-uri", "http://proxy.internal:3128"),
    ])
    .expect("readable properties");

    assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    assert_eq!(options.region(), Some("eu-west-1"));
    assert_eq!(options.path_style(), Some(true));
    assert_eq!(options.connect_timeout(), Duration::from_millis(2500));
    assert_eq!(options.timeout(), Duration::from_secs(30));
    assert_eq!(options.proxy(), Some("http://proxy.internal:3128"));
    let credentials = options.credentials().expect("a credential set");
    assert_eq!(credentials.access_key_id(), "minioadmin");
    assert_eq!(credentials.session_token(), Some("a-session-token"));
}

#[test]
fn pyarrows_arguments_reach_the_same_knobs_by_their_own_names() {
    let options = ObjectOptions::from_properties([
        ("endpoint_override", "localhost:9000"),
        ("scheme", "http"),
        ("access_key", "minioadmin"),
        ("secret_key", "minioadmin-secret"),
        ("region", "eu-west-1"),
        ("force_virtual_addressing", "true"),
        ("request_timeout", "15"),
        ("connect_timeout", "1"),
        ("allow_bucket_creation", "false"),
        ("allow_bucket_deletion", "false"),
        // Ours, and nothing PyArrow ever passes; it is simply ignored.
        ("background_writes", "true"),
    ])
    .expect("readable properties");

    // A scheme and a bare endpoint are one endpoint.
    assert_eq!(options.endpoint(), Some("http://localhost:9000"));
    assert_eq!(options.path_style(), Some(false), "virtual is not path");
    assert_eq!(options.timeout(), Duration::from_secs(15));
    assert!(!options.container_creation());
    assert!(!options.container_deletion());
}

#[test]
fn the_aws_environment_names_are_the_same_knobs_again() {
    let options = ObjectOptions::from_properties([
        ("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE"),
        ("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI"),
        ("AWS_REGION", "us-east-1"),
        ("AWS_ENDPOINT_URL", "https://s3.example.io"),
        ("AWS_S3_FORCE_PATH_STYLE", "true"),
        ("AWS_PROFILE", "trading"),
    ])
    .expect("readable properties");

    assert_eq!(options.region(), Some("us-east-1"));
    assert_eq!(options.endpoint(), Some("https://s3.example.io"));
    assert_eq!(options.path_style(), Some(true), "two prefixes are peeled");
    assert_eq!(options.aws().profile(), Some("trading"));
    assert_eq!(
        options.credentials().map(|keys| keys.access_key_id()),
        Some("AKIAIOSFODNN7EXAMPLE")
    );
}

#[test]
fn a_catalogs_bearer_token_is_never_read_as_a_session_token() {
    // `token` is a REST catalog's OAuth token, and reading it as an AWS
    // session token would sign every request with a credential set that is
    // half right - which fails at the store, far from the cause.
    let options = ObjectOptions::from_properties([
        ("token", "an-oauth-bearer-token"),
        ("s3.access-key-id", "minioadmin"),
        ("s3.secret-access-key", "minioadmin-secret"),
    ])
    .expect("readable properties");
    assert_eq!(
        options.credentials().and_then(|keys| keys.session_token()),
        None
    );
}

#[test]
fn a_role_is_assembled_from_the_properties_that_describe_it() {
    let options = ObjectOptions::from_properties([
        ("s3.role-arn", "arn:aws:iam::123456789012:role/lake-reader"),
        ("s3.role-session-name", "power-desk"),
        ("external_id", "desk-42"),
        ("role_duration", "7200"),
        ("sts_endpoint", "https://sts.eu-west-1.amazonaws.com"),
    ])
    .expect("readable properties");

    let role = options.aws().assumed_role().expect("a role");
    assert_eq!(
        role.role_arn(),
        "arn:aws:iam::123456789012:role/lake-reader"
    );
    assert_eq!(role.session_name(), "power-desk");
    assert_eq!(role.external_id(), Some("desk-42"));
    assert_eq!(role.duration(), Duration::from_secs(7200));
    assert_eq!(role.endpoint(), Some("https://sts.eu-west-1.amazonaws.com"));

    // A duration outside what STS issues is clamped rather than refused.
    let options = ObjectOptions::from_properties([("role_arn", "arn:x"), ("role_duration", "60")])
        .expect("readable properties");
    assert_eq!(
        options.aws().assumed_role().map(AssumedRole::duration),
        Some(Duration::from_secs(900))
    );
}

#[test]
fn the_sse_properties_name_each_of_the_three_kinds() {
    let managed = ObjectOptions::from_properties([("s3.sse.type", "AES256")]).expect("properties");
    assert!(matches!(managed.encryption(), Encryption::Managed));

    let kms = ObjectOptions::from_properties([
        ("s3.sse.type", "aws:kms"),
        ("s3.sse.key", "arn:aws:kms:eu-west-1:1234:key/abcd"),
        ("sse.context", r#"{"desk":"power"}"#),
        ("sse.bucket-key-enabled", "true"),
    ])
    .expect("properties");
    let Encryption::Kms(key) = kms.encryption() else {
        panic!("expected SSE-KMS, got {:?}", kms.encryption());
    };
    assert_eq!(key.key_id(), Some("arn:aws:kms:eu-west-1:1234:key/abcd"));
    assert_eq!(key.context(), Some(r#"{"desk":"power"}"#));
    assert_eq!(key.bucket_key(), Some(true));

    // `AES256` is what SSE-S3 is called and what a customer key's algorithm
    // is called; a key alongside it is what tells the two apart.
    let customer = ObjectOptions::from_properties([
        ("s3.sse.type", "AES256"),
        ("s3.sse.key", "AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw="),
        ("s3.sse.md5", "N+iD0sgzzGlyEXJzCNck5w=="),
    ])
    .expect("properties");
    assert!(matches!(customer.encryption(), Encryption::Customer(_)));

    // And nothing said is nothing done.
    assert!(
        ObjectOptions::from_properties([("s3.sse.type", "none")])
            .expect("properties")
            .encryption()
            .is_default()
    );
}

#[test]
fn a_value_that_will_not_parse_is_heard_here_rather_than_at_the_store() {
    for (name, value, expected) in [
        ("s3.request-timeout", "soon", "seconds"),
        ("anonymous", "perhaps", "boolean"),
        ("max_attempts", "many", "whole number"),
        ("part_size", "big", "byte count"),
        ("s3.sse.type", "rot13", "sse type"),
    ] {
        let refused = ObjectOptions::from_properties([(name, value)]).expect_err("a refusal");
        assert!(
            refused.to_string().contains(expected),
            "{name}={value}: {refused}"
        );
    }

    // A customer key whose MD5 does not match it is the same story.
    let refused = ObjectOptions::from_properties([
        ("s3.sse.type", "sse-c"),
        ("s3.sse.key", "AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw="),
        ("s3.sse.md5", "AAAAAAAAAAAAAAAAAAAAAA=="),
    ])
    .expect_err("a refusal");
    assert!(refused.to_string().contains("MD5"), "{refused}");
}

#[test]
fn a_knob_this_client_cannot_honor_is_refused_rather_than_dropped() {
    let refused = ObjectOptions::from_properties([("s3.signer.uri", "https://signer.example.io")])
        .expect_err("a refusal");
    assert!(refused.to_string().contains("does not do"), "{refused}");

    // Whereas a knob about something that is not this store is simply not
    // this store's business.
    let options = ObjectOptions::from_properties([
        ("adls.account-name", "trades"),
        ("gcs.project-id", "trades"),
        ("py-io-impl", "pyiceberg.io.pyarrow.PyArrowFileIO"),
    ])
    .expect("readable properties");
    assert_eq!(options.endpoint(), None);
    assert_eq!(options.region(), None);
}

#[test]
fn sizes_may_carry_the_unit_a_configuration_file_writes_them_with() {
    let options = ObjectOptions::from_properties([
        ("part_size", "8MiB"),
        ("multipart_threshold", "32 MB"),
        ("list_page_size", "500"),
        ("num_retries", "4"),
    ])
    .expect("readable properties");

    assert_eq!(options.part_size(), 8 * 1024 * 1024);
    assert_eq!(options.multipart_threshold(), 32 * 1024 * 1024);
    assert_eq!(options.list_page_size(), 500);
    assert_eq!(options.max_attempts(), 5, "four retries is five attempts");
}

#[test]
fn the_environment_is_swept_rather_than_looked_up_by_name() {
    // `PATH` exists everywhere, so a prefix of `PAT` proves the sweep runs
    // without this test setting a variable of its own - which it could not do
    // in a crate that denies unsafe code, and should not do to a process it
    // shares with every other test.
    let found = ObjectOptions::default()
        .with_environment_prefixes(["PAT"])
        .environment_properties();
    assert!(
        found
            .iter()
            .any(|(name, value)| name == "H" && !value.is_empty()),
        "expected the sweep to reach PATH: {found:?}"
    );

    // The longest matching prefix wins, so a specific spelling is not eaten by
    // a general one.
    let found = ObjectOptions::default()
        .with_environment_prefixes(["PA", "PAT"])
        .environment_properties();
    assert!(found.iter().any(|(name, _)| name == "H"), "{found:?}");

    // And nothing at all when the environment is shut off, whatever the
    // prefixes say.
    assert!(
        ObjectOptions::default()
            .with_environment(false)
            .with_environment_prefix("PAT")
            .environment_properties()
            .is_empty()
    );
    assert_eq!(
        ObjectOptions::default().environment_prefixes(),
        [
            "AWS_".to_owned(),
            "GOOGLE_".to_owned(),
            "AZURE_".to_owned(),
            "YGGDRYL_".to_owned()
        ]
    );
    assert_eq!(
        ObjectOptions::default()
            .with_environment_prefix("TRADING_S3_")
            .environment_prefixes()
            .len(),
        5
    );
}

#[test]
fn what_a_caller_set_wins_over_what_the_environment_says() {
    let ambient = ObjectOptions::from_properties([
        ("endpoint", "https://ambient.example.io"),
        ("region", "us-east-1"),
        ("access_key_id", "AMBIENT"),
        ("secret_access_key", "ambient-secret"),
        ("max_attempts", "9"),
        ("part_size", "32MiB"),
        ("sse_type", "AES256"),
        ("allow_bucket_creation", "false"),
    ])
    .expect("readable properties");

    let explicit = ObjectOptions::default()
        .with_region("eu-west-1")
        .with_max_attempts(2)
        .under(&ambient);

    // What the caller said stands.
    assert_eq!(explicit.region(), Some("eu-west-1"));
    assert_eq!(explicit.max_attempts(), 2);
    // What they left alone takes the ambient answer.
    assert_eq!(explicit.endpoint(), Some("https://ambient.example.io"));
    assert_eq!(
        explicit.credentials().map(|keys| keys.access_key_id()),
        Some("AMBIENT")
    );
    assert_eq!(explicit.part_size(), 32 * 1024 * 1024);
    assert!(matches!(explicit.encryption(), Encryption::Managed));
    assert!(!explicit.container_creation());

    // An anonymous client stays anonymous, whatever keys are lying about.
    let anonymous = ObjectOptions::default()
        .with_anonymous(true)
        .under(&ambient);
    assert!(anonymous.anonymous());
    assert!(anonymous.credentials().is_none());
}

#[test]
fn a_pair_the_environment_answered_never_names_an_azure_account() {
    // One credential pair serves all three stores, so on Azure a pair a caller
    // handed over is an account name and a shared key. A pair the environment
    // answered is not: it arrived under an `AWS_` name, so letting it name an
    // account would address `<that access key id>.blob.core.windows.net` and
    // sign with a secret meant for somewhere else, purely because the process
    // happened to hold keys for another store.
    let handed = Credentials::new("devstoreaccount1", "a2V5");
    let url = Url::from_str("az://trades/lake/part.bin").expect("a location");
    assert_eq!(
        Client::azure_account(
            Provider::Azure,
            &url,
            &ObjectOptions::default(),
            Some(&handed)
        ),
        Some("devstoreaccount1".to_owned())
    );

    // The same pair sitting in the options, with nothing handed over, is what
    // the environment sweep leaves behind - and it names nothing.
    let ambient = ObjectOptions::default().with_credentials(handed.clone());
    assert_eq!(
        Client::azure_account(Provider::Azure, &url, &ambient, None),
        None
    );

    // The options and the location both outrank a pair either way.
    let named =
        ObjectOptions::default().with_azure(AzureOptions::default().with_account("trading"));
    assert_eq!(
        Client::azure_account(Provider::Azure, &url, &named, Some(&handed)),
        Some("trading".to_owned())
    );
    let attached =
        Url::from_str("az://trades@lake.blob.core.windows.net/part.bin").expect("a location");
    assert_eq!(
        Client::azure_account(
            Provider::Azure,
            &attached,
            &ObjectOptions::default(),
            Some(&handed)
        ),
        Some("lake".to_owned())
    );

    // Only Azure writes an account into its host, so the other two never read
    // one out of a pair that is theirs to begin with.
    for provider in [Provider::Aws, Provider::Google] {
        let url = Url::from_str(&format!(
            "{}://trades/lake/part.bin",
            provider.scheme().as_str()
        ))
        .expect("a location");
        assert_eq!(
            Client::azure_account(provider, &url, &ObjectOptions::default(), Some(&handed)),
            None,
            "{provider}"
        );
    }

    // And with no account anywhere - which is what a process holding only
    // another store's keys now has - the refusal names where to put one rather
    // than sending the request to whichever host a stray key spelled.
    let Err(refusal) = Client::new(&url, ObjectOptions::default().with_environment(false)) else {
        panic!("an unaddressable location is refused");
    };
    assert!(
        refusal
            .to_string()
            .contains("expected an Azure storage account"),
        "{refusal}"
    );
}

#[test]
fn the_service_specific_endpoint_and_region_names_win() {
    let options = ObjectOptions::from_properties([
        ("AWS_ENDPOINT_URL", "https://generic.example.io"),
        ("AWS_ENDPOINT_URL_S3", "https://s3.example.io"),
        ("AWS_DEFAULT_REGION", "us-east-1"),
        ("AWS_REGION", "eu-west-1"),
    ])
    .expect("readable properties");

    assert_eq!(options.endpoint(), Some("https://s3.example.io"));
    assert_eq!(options.region(), Some("eu-west-1"));
}

#[test]
fn a_catalogs_google_and_azure_properties_reach_the_store_they_name() {
    // One catalog can hold all three stores' properties at once, because a
    // warehouse on one cloud and a backup on another is an ordinary thing to
    // configure. Each name reaches its own store's options and no other's.
    let options = ObjectOptions::from_properties([
        ("type", "rest"),
        ("warehouse", "gs://trades/lake"),
        // PyIceberg's Google names.
        ("gcs.project-id", "trading-analytics"),
        ("gcs.oauth2.token", "ya29.a0AfH6"),
        ("gcs.default-bucket-location", "EUROPE-WEST4"),
        // PyIceberg's Azure names.
        ("adls.account-name", "trades"),
        ("adls.account-key", "a2V5"),
        ("adls.tenant-id", "00000000-0000-0000-0000-000000000000"),
        ("adls.client-id", "a-client"),
        ("adls.client-secret", "a-secret"),
    ])
    .expect("readable properties");

    assert_eq!(options.google().project(), Some("trading-analytics"));
    assert_eq!(options.google().access_token(), Some("ya29.a0AfH6"));
    assert_eq!(options.azure().account(), Some("trades"));
    assert!(options.azure().has_account_key());
    assert_eq!(
        options.azure().tenant_id(),
        Some("00000000-0000-0000-0000-000000000000")
    );
    assert_eq!(options.azure().client_id(), Some("a-client"));
    // An Azure account name and key are a credential pair like any other, and
    // the pair is what a shared-key signature is built from.
    let credentials = options.credentials().expect("a credential pair");
    assert_eq!(credentials.access_key_id(), "trades");
}

#[test]
fn an_azure_connection_string_is_read_whole_rather_than_taken_apart() {
    let options = ObjectOptions::from_properties([(
        "azure_storage_connection_string",
        "DefaultEndpointsProtocol=https;AccountName=trades;AccountKey=a2V5;\
         EndpointSuffix=core.windows.net",
    )])
    .expect("readable properties");
    assert_eq!(options.azure().account(), Some("trades"));
    assert!(options.azure().has_account_key());
    assert_eq!(
        options.azure().endpoint(),
        Some("https://trades.blob.core.windows.net")
    );

    // The emulator's switch names the published development account, so a test
    // needs no secret of its own.
    let development =
        ObjectOptions::from_properties([("connection_string", "UseDevelopmentStorage=true")])
            .expect("readable properties");
    assert_eq!(development.azure().account(), Some("devstoreaccount1"));
    assert_eq!(
        development.azure().endpoint(),
        Some("http://127.0.0.1:10000/devstoreaccount1")
    );
}

#[test]
fn a_name_two_stores_both_have_is_applied_to_both() {
    // Only the store that answers ever reads its own options, so nothing is
    // ambiguous by the time it matters.
    let options = ObjectOptions::from_properties([("storage_class", "NEARLINE")])
        .expect("readable properties");
    assert_eq!(options.aws().storage_class(), Some("NEARLINE"));
    assert_eq!(options.google().storage_class(), Some("NEARLINE"));
}

#[test]
fn a_google_credentials_file_and_an_impersonation_are_read_by_their_own_names() {
    let options = ObjectOptions::from_properties([
        (
            "google_application_credentials",
            "/etc/keys/service-account.json",
        ),
        (
            "gcs.impersonate-service-account",
            "lake-reader@trading.iam.gserviceaccount.com",
        ),
        ("gcs.user-project", "billing-project"),
    ])
    .expect("readable properties");
    assert_eq!(
        options.google().credentials_file(),
        Some(std::path::Path::new("/etc/keys/service-account.json"))
    );
    assert_eq!(
        options.google().impersonation(),
        Some("lake-reader@trading.iam.gserviceaccount.com")
    );
    assert_eq!(options.google().user_project(), Some("billing-project"));
}

#[test]
fn an_azure_blob_type_and_tier_are_read_and_a_name_neither_has_is_refused() {
    let options = ObjectOptions::from_properties([
        ("adls.blob-type", "append"),
        ("adls.access-tier", "Cool"),
        ("adls.encryption-scope", "desk-power"),
    ])
    .expect("readable properties");
    assert_eq!(
        options.azure().blob_type(),
        crate::holder::object::BlobType::Append
    );
    assert_eq!(options.azure().access_tier(), Some("Cool"));
    assert!(matches!(
        options.encryption(),
        Encryption::Scope(scope) if scope == "desk-power"
    ));

    let refused =
        ObjectOptions::from_properties([("adls.blob-type", "ledger")]).expect_err("a refusal");
    assert!(
        refused.to_string().contains("block, append, or page"),
        "{refused}"
    );
}

#[test]
fn an_encryption_shape_a_store_does_not_have_is_refused_when_the_client_is_built() {
    // Azure spells a managed key as an account encryption scope, so naming a
    // KMS key for it is a mistake worth hearing before the first write.
    let refused = crate::holder::object::file_with(
        "az://lake/part.bin",
        ObjectOptions::default()
            .with_environment(false)
            .with_azure(crate::holder::object::AzureOptions::default().with_account("trades"))
            .with_encryption(Encryption::kms("a-key")),
    )
    .expect_err("a refusal");
    assert!(
        refused.to_string().contains("encryption scope"),
        "{refused}"
    );

    // And the other way round: a scope is Azure's shape, not Google's.
    let refused = crate::holder::object::file_with(
        "gs://lake/part.bin",
        ObjectOptions::default()
            .with_environment(false)
            .with_encryption(Encryption::scope("desk-power")),
    )
    .expect_err("a refusal");
    assert!(refused.to_string().contains("Encryption::kms"), "{refused}");
}
