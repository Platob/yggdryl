//! `rust/src/aws/properties.rs`: the one reader of AWS identity properties,
//! in the names this crate, the AWS tools and PyIceberg each spell them by.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use yggdryl::Error;
use yggdryl::aws::{CredentialSource, DeviceAuthorization, Session, SsoLogin};

use crate::identity::{Identity, SSO_TOKEN};
use crate::mod_::scratch;

/// A session that reads nothing but what a test states on it.
fn sealed() -> Session {
    Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(scratch("properties"))
        .with_metadata_disabled(true)
}

fn refusal(error: &Error) -> String {
    assert!(
        matches!(error, Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput),
        "{error:?}"
    );
    error.to_string()
}

#[test]
fn a_pyiceberg_catalogs_client_properties_state_the_region_and_the_set() {
    let session = sealed()
        .with_properties([
            ("uri", "https://s3tables.eu-west-3.amazonaws.com/iceberg"),
            (
                "warehouse",
                "arn:aws:s3tables:eu-west-3:123456789012:bucket/lake",
            ),
            ("rest.sigv4-enabled", "true"),
            ("rest.signing-name", "s3tables"),
            ("client.region", "eu-west-3"),
            ("client.access-key-id", "ASIACATALOG"),
            ("client.secret-access-key", "catalog-secret"),
            ("client.session-token", "catalog-token"),
        ])
        .expect("properties");
    assert_eq!(session.region().as_deref(), Some("eu-west-3"));
    let keys = session
        .credentials(SystemTime::now())
        .expect("a walk")
        .expect("the stated set");
    assert_eq!(keys.access_key_id(), "ASIACATALOG");
    assert_eq!(keys.session_token(), Some("catalog-token"));
    assert_eq!(session.credential_source(), Some("explicit credentials"));
}

#[test]
fn the_bare_names_the_aws_names_and_the_client_names_are_one_name() {
    for (region, access, secret, token, profile) in [
        (
            "region",
            "access_key_id",
            "secret_access_key",
            "session_token",
            "profile",
        ),
        (
            "AWS_REGION",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "AWS_PROFILE",
        ),
        (
            "client.region",
            "client.access-key-id",
            "client.secret-access-key",
            "client.session-token",
            "client.profile-name",
        ),
        (
            "Region",
            "access-key",
            "secret.key",
            "aws.session.token",
            "profile_name",
        ),
    ] {
        let session = sealed()
            .with_properties([
                (region, "ap-southeast-1"),
                (access, "AKIAONE"),
                (secret, "one-secret"),
                (token, "one-token"),
                (profile, "trading"),
            ])
            .expect("properties");
        assert_eq!(
            session.region().as_deref(),
            Some("ap-southeast-1"),
            "{region}"
        );
        assert_eq!(session.profile_name(), "trading", "{profile}");
        let keys = session
            .credentials(SystemTime::now())
            .expect("a walk")
            .expect("the stated set");
        assert_eq!(keys.access_key_id(), "AKIAONE", "{access}");
        assert_eq!(keys.session_token(), Some("one-token"), "{token}");
        for name in [region, access, secret, token, profile] {
            assert!(Session::is_property(name), "{name}");
        }
    }
}

#[test]
fn a_catalogs_bearer_token_and_the_stores_own_names_are_not_read_here() {
    // `token` authorizes a catalog, `s3.*` is the object store's reader's,
    // and a bag carries a great deal that is nobody's identity.
    let session = sealed()
        .with_properties([
            ("token", "a catalog's bearer token"),
            ("bearer_token", "a store's"),
            ("s3.region", "us-west-2"),
            ("s3.access-key-id", "AKIASTORE"),
            ("s3.secret-access-key", "store-secret"),
            ("endpoint_url", "http://localhost:9000"),
            ("warehouse", "s3://lake"),
            ("regoin", "eu-west-3"),
            ("region", "   "),
        ])
        .expect("unknown names are ignored, and an empty value states nothing");
    assert_eq!(session.region(), None);
    assert_eq!(
        session.endpoint_url("s3").expect("a readable endpoint"),
        None
    );
    assert_eq!(
        session.credentials(SystemTime::now()).expect("a walk"),
        None
    );
    for name in [
        "token",
        "bearer_token",
        "s3.region",
        "s3.access-key-id",
        "endpoint_url",
        "warehouse",
        "regoin",
        "catalog.token",
    ] {
        assert!(!Session::is_property(name), "{name}");
    }
    for name in Session::PROPERTY_NAMES {
        assert!(Session::is_property(name), "{name}");
    }
}

#[test]
fn half_a_credential_set_is_refused_naming_the_half_that_is_missing() {
    let error = sealed()
        .with_properties([("client.access-key-id", "AKIAHALF")])
        .expect_err("a key with no secret");
    assert!(
        refusal(&error).contains("the secret_access_key is missing"),
        "{error}"
    );

    let error = sealed()
        .with_properties([("aws_secret_access_key", "secret")])
        .expect_err("a secret with no key");
    assert!(
        refusal(&error).contains("the access_key_id is missing"),
        "{error}"
    );

    let error = sealed()
        .with_properties([("session_token", "token")])
        .expect_err("a token with no pair");
    assert!(
        refusal(&error).contains("the access_key_id and the secret_access_key are missing"),
        "{error}"
    );
}

#[test]
fn anonymous_states_that_nothing_signs_and_a_value_that_is_no_boolean_is_refused() {
    let session = sealed()
        .with_properties([("no_sign_request", "true")])
        .expect("properties");
    assert!(session.anonymous());
    assert_eq!(
        session.credentials(SystemTime::now()).expect("a walk"),
        None
    );

    let error = sealed()
        .with_properties([("AWS_USE_FIPS_ENDPOINT", "perhaps")])
        .expect_err("not a boolean");
    let message = refusal(&error);
    assert!(
        message.contains("true/false") && message.contains("AWS_USE_FIPS_ENDPOINT"),
        "{message}"
    );
}

#[test]
fn a_role_and_the_switches_are_assembled_as_the_object_store_reader_assembles_them() {
    let session = sealed()
        .with_properties([
            ("client.role-arn", "arn:aws:iam::123456789012:role/reader"),
            ("role_session_name", "audit"),
            ("external_id", "partner"),
            ("role_duration", "1800"),
            ("sts_region", "eu-west-1"),
            ("mfa_serial", "arn:aws:iam::123456789012:mfa/dev"),
            ("credential_source", "Environment"),
            ("use_fips_endpoint", "yes"),
            ("use_dualstack_endpoint", "0"),
            ("sts_regional_endpoints", "legacy"),
            ("ec2_metadata_disabled", "true"),
            ("metadata_service_timeout", "2.5"),
        ])
        .expect("properties");
    let role = session.assumed_role().expect("a role");
    assert_eq!(role.role_arn(), "arn:aws:iam::123456789012:role/reader");
    assert_eq!(role.session_name(), Some("audit"));
    assert_eq!(role.external_id(), Some("partner"));
    assert_eq!(role.duration(), Duration::from_secs(1800));
    assert_eq!(role.region(), Some("eu-west-1"));
    assert_eq!(
        role.credential_source(),
        Some(CredentialSource::Environment)
    );
    assert!(session.use_fips_endpoint());
    assert!(!session.use_dualstack_endpoint());
    assert!(!session.sts_regional_endpoints().expect("a mode"));

    // An STS endpoint stated without a role still says where STS is.
    let session = sealed()
        .with_properties([("sts_endpoint", "http://localhost:4566/")])
        .expect("properties");
    assert_eq!(
        session.sts_endpoint("eu-west-3").expect("an STS endpoint"),
        "http://localhost:4566"
    );
}

#[test]
fn a_sign_in_is_four_values_or_none_and_names_what_is_missing() {
    let session = sealed()
        .with_properties([
            ("sso_start_url", "https://corp.awsapps.com/start"),
            ("sso_region", "eu-west-1"),
            ("sso_account_id", "123456789012"),
            ("sso_role_name", "Reader"),
            ("sso_session", "corp"),
        ])
        .expect("properties");
    let sso = session.sso().expect("a sign-in");
    assert_eq!(sso.region(), "eu-west-1");

    let error = sealed()
        .with_properties([
            ("sso_start_url", "https://corp.awsapps.com/start"),
            ("sso_region", "eu-west-1"),
        ])
        .expect_err("half a sign-in");
    assert!(
        refusal(&error).contains("needs sso_account_id, sso_role_name"),
        "{error}"
    );
}

#[test]
fn properties_are_stated_over_the_session_and_a_later_pair_replaces_an_earlier_one() {
    let base = sealed().with_region("us-east-1").with_profile("base");
    let session = base
        .with_properties([("region", "eu-west-3"), ("AWS_REGION", "eu-central-1")])
        .expect("properties");
    assert_eq!(session.region().as_deref(), Some("eu-central-1"));
    assert_eq!(
        session.profile_name(),
        "base",
        "what the bag does not name stands"
    );
    assert_eq!(
        base.region().as_deref(),
        Some("us-east-1"),
        "the session it came from is untouched"
    );

    let fresh = Session::from_properties([("profile", "trading")]).expect("properties");
    assert_eq!(fresh.profile_name(), "trading");
    assert!(
        fresh.reads_environment(),
        "from_properties starts from Session::new"
    );
}

#[test]
fn the_aws_tools_default_and_service_endpoint_names_are_read() {
    let session = sealed()
        .with_properties([
            ("AWS_DEFAULT_REGION", "eu-west-3"),
            ("AWS_DEFAULT_PROFILE", "desk"),
            ("AWS_ENDPOINT_URL_STS", "http://localhost:4566/"),
            ("aws_endpoint_url_s3tables", "http://localhost:4567/tables"),
            ("AWS_ENDPOINT_URL_SSO_OIDC", "http://localhost:4568"),
        ])
        .expect("properties");
    assert_eq!(session.region().as_deref(), Some("eu-west-3"));
    assert_eq!(session.profile_name(), "desk");
    for (service, endpoint) in [
        ("sts", "http://localhost:4566"),
        ("s3tables", "http://localhost:4567/tables"),
        ("sso-oidc", "http://localhost:4568"),
    ] {
        assert_eq!(
            session
                .endpoint_url(service)
                .expect("a readable endpoint")
                .as_deref(),
            Some(endpoint),
            "{service}"
        );
    }
    // Amazon S3's own endpoint is the object store's reader's.
    assert_eq!(
        session.endpoint_url("s3").expect("a readable endpoint"),
        None
    );
    assert!(!Session::is_property("AWS_ENDPOINT_URL_S3"));
    for name in [
        "AWS_DEFAULT_REGION",
        "AWS_DEFAULT_PROFILE",
        "AWS_ENDPOINT_URL_STS",
        "client.endpoint-url-s3tables",
    ] {
        assert!(Session::is_property(name), "{name}");
    }

    // A default is read where nothing else is stated, whatever the order.
    for pairs in [
        [
            ("AWS_REGION", "ap-south-1"),
            ("AWS_DEFAULT_REGION", "eu-west-3"),
            ("profile", "trading"),
            ("default_profile", "desk"),
        ],
        [
            ("AWS_DEFAULT_REGION", "eu-west-3"),
            ("AWS_REGION", "ap-south-1"),
            ("default_profile", "desk"),
            ("profile", "trading"),
        ],
    ] {
        let session = sealed().with_properties(pairs).expect("properties");
        assert_eq!(session.region().as_deref(), Some("ap-south-1"), "{pairs:?}");
        assert_eq!(session.profile_name(), "trading", "{pairs:?}");
    }

    // A stated sts_endpoint is STS's endpoint over AWS_ENDPOINT_URL_STS.
    let session = sealed()
        .with_properties([
            ("sts_endpoint", "http://localhost:4600"),
            ("AWS_ENDPOINT_URL_STS", "http://localhost:4566"),
        ])
        .expect("properties");
    assert_eq!(
        session
            .endpoint_url("sts")
            .expect("a readable endpoint")
            .as_deref(),
        Some("http://localhost:4600")
    );
}

#[test]
fn an_sts_endpoint_mode_is_regional_or_legacy_and_nothing_else() {
    // botocore takes the two words alone; a flag's spellings say the same
    // two things and are read through the one boolean table, and a typo or
    // another word is no mode, never read as either.
    for spelling in ["sometimes", "legasy", "global"] {
        let error = sealed()
            .with_properties([("AWS_STS_REGIONAL_ENDPOINTS", spelling)])
            .expect_err("no mode");
        let message = refusal(&error);
        assert!(
            message.contains("regional/legacy")
                && message.contains("AWS_STS_REGIONAL_ENDPOINTS")
                && message.contains(spelling),
            "{message}"
        );
    }
    for (spelling, regional) in [
        ("regional", true),
        ("REGIONAL", true),
        ("Legacy", false),
        (" legacy ", false),
        ("true", true),
        ("false", false),
    ] {
        let session = sealed()
            .with_properties([("AWS_STS_REGIONAL_ENDPOINTS", spelling)])
            .expect("a mode");
        assert_eq!(
            session.sts_regional_endpoints().expect("a mode"),
            regional,
            "{spelling}"
        );
    }
}

#[test]
fn a_property_role_names_one_source_and_an_arn() {
    let error = sealed()
        .with_properties([
            ("role_arn", "arn:aws:iam::123456789012:role/reader"),
            ("source_profile", "base"),
            ("credential_source", "Ec2InstanceMetadata"),
        ])
        .expect_err("two sources");
    assert!(
        refusal(&error).contains("both source_profile and credential_source"),
        "{error}"
    );

    for spelling in ["lake-reader", "arn:x"] {
        let error = sealed()
            .with_properties([("client.role-arn", spelling)])
            .expect_err("no ARN");
        let message = refusal(&error);
        assert!(
            message.contains("not an ARN") && message.contains("client.role-arn"),
            "{message}"
        );
    }
    assert!(
        Session::is_property("role_arn"),
        "a refused probe is a name"
    );

    for (source, credential_source) in [(Some("base"), None), (None, Some("Environment"))] {
        let mut pairs = vec![("role_arn", "arn:aws:iam::123456789012:role/reader")];
        pairs.extend(source.map(|source| ("source_profile", source)));
        pairs.extend(credential_source.map(|source| ("credential_source", source)));
        let session = sealed().with_properties(pairs).expect("one source");
        let role = session.assumed_role().expect("a role");
        assert_eq!(role.source_profile(), source);
        assert_eq!(
            role.credential_source(),
            credential_source.map(|_| CredentialSource::Environment)
        );
    }
}

#[test]
fn the_profile_key_duration_seconds_is_a_role_duration() {
    let error = sealed()
        .with_properties([
            ("role_arn", "arn:aws:iam::123456789012:role/reader"),
            ("duration_seconds", "an hour or so"),
        ])
        .expect_err("no duration");
    assert!(refusal(&error).contains("duration_seconds"), "{error}");

    let session = sealed()
        .with_properties([
            ("role_arn", "arn:aws:iam::123456789012:role/reader"),
            ("duration_seconds", "1800"),
        ])
        .expect("properties");
    assert_eq!(
        session.assumed_role().expect("a role").duration(),
        Duration::from_secs(1800)
    );
    assert!(Session::is_property("duration_seconds"));
    assert!(Session::is_property("AWS_DURATION_SECONDS"));
}

/// The `[sso-session corp]` section a sign-in stated by its name reads.
const CORP_SECTION: &str = "\
[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1
sso_registration_scopes = sso:account:access, codewhisperer:completions
";

const CORP_START_URL: &str = "https://corp.awsapps.com/start";

/// `sha1("corp")`: a sign-in that belongs to an `[sso-session]` files its
/// token under the session's name.
const CORP_TOKEN_KEY: &str = "ee0bfd2552fbd840c02cc48b6e823320543c450f";

/// The sign-in to `LakeReader` stated by its section's name alone.
const CORP_BY_SECTION: [(&str, &str); 3] = [
    ("sso_session", "corp"),
    ("sso_account_id", "123456789012"),
    ("sso_role_name", "LakeReader"),
];

/// A session reaching nothing but the fake, under `CORP_SECTION`, signing
/// in through `CORP_BY_SECTION`, and the directory it keeps `~/.aws` in.
fn corp_by_section(identity: &Identity, name: &str) -> (Session, PathBuf) {
    let session = crate::mod_::sealed(identity, name)
        .with_config_text(CORP_SECTION)
        .with_properties(CORP_BY_SECTION)
        .expect("a sign-in by its section");
    let directory = session.directory().expect("a sealed directory");
    (session, directory)
}

fn token_path(directory: &Path) -> PathBuf {
    directory
        .join("sso")
        .join("cache")
        .join(format!("{CORP_TOKEN_KEY}.json"))
}

/// Every request the fake handled, as `METHOD path`.
fn shape(identity: &Identity) -> Vec<String> {
    identity
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

#[test]
fn an_sso_session_property_takes_its_start_url_and_region_from_the_section() {
    // What the door refuses: an account or a role missing, and one of the
    // start URL and region stated over the section without the other.
    let error = sealed()
        .with_properties([("sso_session", "corp"), ("sso_role_name", "LakeReader")])
        .expect_err("no account");
    assert!(refusal(&error).contains("needs sso_account_id"), "{error}");
    let error = sealed()
        .with_properties(
            CORP_BY_SECTION
                .into_iter()
                .chain([("sso_start_url", CORP_START_URL)]),
        )
        .expect_err("a start URL without its region");
    let message = refusal(&error);
    assert!(
        message.contains("needs sso_region") && message.contains("[sso-session corp]"),
        "{message}"
    );

    // The door reads no file: it names the section, and the walk reads it.
    let stated = sealed()
        .with_properties(CORP_BY_SECTION)
        .expect("a sign-in by its section");
    let sso = stated.sso().expect("a sign-in");
    assert_eq!(sso.session_name(), Some("corp"));
    assert_eq!(sso.account_id(), "123456789012");
    assert_eq!(sso.role_name(), "LakeReader");

    // A sign-in never made is refused naming the start URL the section
    // holds, and nothing is traded.
    let identity = Identity::start();
    identity.set_imds_role(None);
    let (session, directory) = corp_by_section(&identity, "sso-section-unmade");
    let message = session
        .credentials(SystemTime::now())
        .expect_err("no sign-in to trade")
        .to_string();
    assert!(message.contains(CORP_START_URL), "{message}");
    assert!(message.contains("`aws sso login`"), "{message}");
    assert!(
        shape(&identity)
            .iter()
            .all(|request| !request.contains("federation")),
        "{:?}",
        shape(&identity)
    );

    // A sign-in made under the section registers for its scopes and files
    // its start URL and region.
    let login = session.with_sso_login(SsoLogin::Handler(Arc::new(|_: &DeviceAuthorization| {})));
    identity.clear_requests();
    login.login().expect("a sign-in");
    let requests = identity.requests();
    let register = requests
        .iter()
        .find(|request| request.path == "/client/register")
        .expect("a registration");
    let register: serde_json::Value =
        serde_json::from_str(&register.body).expect("a JSON registration");
    assert_eq!(
        register["scopes"],
        serde_json::json!(["sso:account:access", "codewhisperer:completions"])
    );
    let filed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(token_path(&directory)).expect("the filed sign-in"),
    )
    .expect("a JSON token");
    assert_eq!(filed["startUrl"], CORP_START_URL);
    assert_eq!(filed["region"], "eu-west-1");
    assert_eq!(filed["accessToken"], SSO_TOKEN);

    // A later session that never signs in trades the filed token.
    identity.clear_requests();
    let (later, _) = corp_by_section(&identity, "sso-section-later");
    let later = later.with_directory(&directory);
    let keys = later
        .credentials(SystemTime::now())
        .expect("a walk")
        .expect("the role's set");
    assert_eq!(keys.access_key_id(), "ASIALakeReader");
    assert_eq!(later.credential_source(), Some("sso"));
    assert_eq!(shape(&identity), ["GET /federation/credentials"]);
}

#[test]
fn an_sso_session_property_naming_no_section_is_refused_by_the_walk_naming_it() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = crate::mod_::sealed(&identity, "sso-section-ghost")
        .with_config_text(CORP_SECTION)
        .with_properties([
            ("sso_session", "ghost"),
            ("sso_account_id", "123456789012"),
            ("sso_role_name", "LakeReader"),
        ])
        .expect("the door reads no file");
    let message = session
        .credentials(SystemTime::now())
        .expect_err("no section to sign in through")
        .to_string();
    assert!(message.contains("[sso-session ghost]"), "{message}");
    assert_eq!(shape(&identity), Vec::<String>::new(), "nothing asked");
}
