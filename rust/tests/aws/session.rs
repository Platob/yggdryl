//! `rust/src/aws/session.rs`: the chain a session walks, the caches it shares
//! with the AWS CLI, and every setting it resolves without a socket.
//!
//! Every walk runs over the identity fake: a sealed session reads no process
//! environment, keeps its `~/.aws` in a directory of the test's own, and
//! reaches STS, IAM Identity Center, the container endpoint and the instance
//! metadata service at one loopback listener that records what it was asked.
//! The number of requests a walk makes is the contract, so it is counted.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::{
    AssumedRole, CredentialSource, Credentials, DeviceAuthorization, Session, Sso, SsoLogin,
};

use crate::identity::{
    CONTAINER_PATH, IMDS_TOKEN, Identity, Recorded, SSO_REFRESH_TOKEN, SSO_TOKEN, iso8601,
};
use crate::mod_::{scratch, sealed};

/// An expiry no test outlives, and the instant it names.
const FAR: &str = "2099-01-01T00:00:00Z";
const FAR_SECONDS: u64 = 4_070_908_800;

/// What one walk that ends at the instance metadata service asks it, in order.
const IMDS_WALK: [&str; 3] = [
    "PUT /latest/api/token",
    "GET /latest/meta-data/iam/security-credentials/",
    "GET /latest/meta-data/iam/security-credentials/instance-role",
];

/// The keys the fake's instance role answers.
const INSTANCE_KEY: &str = "ASIAINSTANCEROLE";

/// `pairs`, plus a `BOTO_CONFIG` naming a file under `directory` that does not
/// exist, so the legacy boto step reads neither `/etc/boto.cfg` nor the
/// machine's own `~/.boto`.
fn variables(directory: &Path, pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut given: Vec<(String, String)> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    given.push((
        "BOTO_CONFIG".to_owned(),
        directory.join("no-such-boto.cfg").display().to_string(),
    ));
    given
}

/// A sealed session reading exactly `pairs` as its environment, and the
/// directory it keeps its `~/.aws` under.
fn walled(identity: &Identity, name: &str, pairs: &[(&str, &str)]) -> (Session, PathBuf) {
    let session = sealed(identity, name);
    let directory = session
        .directory()
        .expect("a sealed session names its directory");
    (
        session.with_variables(variables(&directory, pairs)),
        directory,
    )
}

/// A second session over `directory`, as a second process on the same
/// machine would be: nothing resolved, the files and caches shared.
fn again(identity: &Identity, directory: &Path, pairs: &[(&str, &str)]) -> Session {
    sealed(identity, "again")
        .with_directory(directory)
        .with_variables(variables(directory, pairs))
}

/// A session that reads `pairs` as its whole environment, keeps its files
/// under a directory of its own, and never needs a socket: the resolution
/// questions read variables and text, nothing else.
fn offline(name: &str, pairs: &[(&str, &str)]) -> Session {
    Session::new()
        .with_variables(pairs.iter().copied())
        .with_directory(scratch(name))
        .with_metadata_disabled(true)
}

/// The set the session signs with now, which the test expects to exist.
fn found(session: &Session) -> Credentials {
    session
        .credentials(SystemTime::now())
        .expect("a walk that answers")
        .expect("a credential set rather than unsigned requests")
}

/// The refusal the session answers now, rendered.
fn refused(session: &Session) -> String {
    session
        .credentials(SystemTime::now())
        .expect_err("a walk that refuses")
        .to_string()
}

/// Every request the fake handled, as `METHOD path`.
fn shape(identity: &Identity) -> Vec<String> {
    identity
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// The STS exchanges for `action` the fake handled.
fn exchanges(identity: &Identity, action: &str) -> Vec<Recorded> {
    identity
        .requests()
        .into_iter()
        .filter(|request| request.is_sts(action))
        .collect()
}

/// One header of a recorded request, or the empty string.
fn header<'a>(request: &'a Recorded, name: &str) -> &'a str {
    request.header(name).unwrap_or_default()
}

/// Write `text` to `path`, creating its parents.
fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent of a file the test writes");
    }
    std::fs::write(path, text).expect("a file the test writes");
}

/// The JSON document a cache file holds.
fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} should have been written: {error}", path.display()));
    serde_json::from_str(&text).expect("a cache file is JSON")
}

/// The seconds since the epoch now.
fn now_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_secs();
    i64::try_from(seconds).expect("seconds that fit")
}

// --- what the caller states -------------------------------------------------

#[test]
fn explicit_credentials_answer_as_given_with_no_request_and_name_their_source() {
    let identity = Identity::start();
    let explicit = Credentials::new("AKIAEXPLICIT", "explicit-secret");
    let session = sealed(&identity, "explicit").with_credentials(explicit.clone());

    assert_eq!(
        session.credential_source(),
        None,
        "nothing is walked before the first ask"
    );
    assert_eq!(
        found(&session),
        explicit,
        "the set stated is the set signed with"
    );
    assert_eq!(session.credential_source(), Some("explicit credentials"));
    assert_eq!(identity.request_count(), 0, "a stated set costs no request");
}

#[test]
fn an_anonymous_session_answers_unsigned_even_over_keys_the_environment_holds() {
    let identity = Identity::start();
    let (session, _) = walled(
        &identity,
        "anonymous",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ],
    );
    let session = session.with_anonymous(true);

    assert!(session.anonymous());
    let answer = session
        .credentials(SystemTime::now())
        .expect("an anonymous session never refuses");
    assert_eq!(
        answer, None,
        "anonymous is unsigned, not the environment's keys"
    );
    assert_eq!(session.credential_source(), None);
    assert_eq!(identity.request_count(), 0, "anonymous asks nothing");
}

#[test]
fn a_session_that_reads_no_environment_and_states_nothing_is_unsigned_without_a_request_or_a_file()
{
    let identity = Identity::start();
    let (session, directory) = walled(
        &identity,
        "no-environment",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ],
    );
    write(
        &directory.join("config"),
        "[default]\naws_access_key_id = AKIAFILE\naws_secret_access_key = file-secret\n",
    );
    let session = session.with_environment(false);

    assert!(!session.reads_environment());
    assert_eq!(
        session
            .credentials(SystemTime::now())
            .expect("nothing configured is not a refusal"),
        None,
        "neither the variables nor the file on disk are consulted"
    );
    assert_eq!(
        session.variable("AWS_ACCESS_KEY_ID"),
        None,
        "no variable is read"
    );
    assert!(session.profile().is_none(), "no file is read either");
    assert_eq!(
        session.instance_region(),
        None,
        "and no metadata service is asked"
    );
    assert_eq!(
        identity.request_count(),
        0,
        "a sealed-off session makes no request"
    );
}

// --- the environment --------------------------------------------------------

#[test]
fn the_environment_keys_answer_with_their_session_token_and_their_expiry() {
    let identity = Identity::start();
    let (session, _) = walled(
        &identity,
        "environment",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
            ("AWS_SESSION_TOKEN", "env-token"),
            ("AWS_SECURITY_TOKEN", "older-token"),
            ("AWS_CREDENTIAL_EXPIRATION", FAR),
        ],
    );

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "AKIAENV");
    assert_eq!(
        keys.session_token(),
        Some("env-token"),
        "AWS_SESSION_TOKEN wins over the older AWS_SECURITY_TOKEN"
    );
    assert_eq!(
        keys.expires_at(),
        Some(UNIX_EPOCH + Duration::from_secs(FAR_SECONDS)),
        "AWS_CREDENTIAL_EXPIRATION is the set's expiry"
    );
    assert_eq!(session.credential_source(), Some("environment"));
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn aws_account_id_is_carried_beside_the_environment_keys() {
    let session = offline(
        "env-account",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
            ("AWS_ACCOUNT_ID", "123456789012"),
        ],
    );
    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "AKIAENV");
    assert_eq!(keys.account_id(), Some("123456789012"));
    assert_eq!(session.credential_source(), Some("environment"));
}

#[test]
fn aws_security_token_is_read_as_the_session_token_when_aws_session_token_is_absent() {
    let identity = Identity::start();
    let (session, _) = walled(
        &identity,
        "security-token",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
            ("AWS_SECURITY_TOKEN", "older-token"),
        ],
    );

    let keys = found(&session);
    assert_eq!(keys.session_token(), Some("older-token"));
    assert_eq!(
        keys.expires_at(),
        None,
        "no expiry stated is a set that does not lapse"
    );
}

#[test]
fn a_profile_stated_on_the_session_skips_the_environment_keys_and_aws_profile_does_not() {
    let identity = Identity::start();
    let pairs = [
        ("AWS_ACCESS_KEY_ID", "AKIAENV"),
        ("AWS_SECRET_ACCESS_KEY", "env-secret"),
        ("AWS_PROFILE", "desk"),
    ];
    let credentials = "[desk]\naws_access_key_id = AKIADESK\naws_secret_access_key = desk-secret\n";

    let (named_by_variable, directory) = walled(&identity, "profile-precedence", &pairs);
    let named_by_variable = named_by_variable.with_credentials_text(credentials);
    assert_eq!(named_by_variable.profile_name(), "desk");
    assert_eq!(
        found(&named_by_variable).access_key_id(),
        "AKIAENV",
        "a profile the environment names does not displace the environment's own keys"
    );
    assert_eq!(named_by_variable.credential_source(), Some("environment"));

    let stated = again(&identity, &directory, &pairs)
        .with_credentials_text(credentials)
        .with_profile("desk");
    assert_eq!(
        found(&stated).access_key_id(),
        "AKIADESK",
        "a profile the caller named is what the caller meant"
    );
    assert_eq!(stated.credential_source(), Some("shared credentials file"));
    assert_eq!(identity.request_count(), 0);
}

// --- the shared files -------------------------------------------------------

#[test]
fn a_profile_s_keys_answer_from_the_credentials_file_or_from_the_config_file_each_named() {
    let identity = Identity::start();

    let (from_credentials, _) = walled(&identity, "credentials-file", &[]);
    let from_credentials = from_credentials.with_credentials_text(
        "[default]\naws_access_key_id = AKIACREDENTIALS\naws_secret_access_key = credentials-secret\naws_session_token = credentials-token\n",
    );
    let keys = found(&from_credentials);
    assert_eq!(keys.access_key_id(), "AKIACREDENTIALS");
    assert_eq!(keys.session_token(), Some("credentials-token"));
    assert_eq!(
        from_credentials.credential_source(),
        Some("shared credentials file")
    );

    let (from_config, _) = walled(&identity, "config-file", &[]);
    let from_config = from_config.with_config_text(
        "[default]\naws_access_key_id = AKIACONFIG\naws_secret_access_key = config-secret\n",
    );
    assert_eq!(found(&from_config).access_key_id(), "AKIACONFIG");
    assert_eq!(from_config.credential_source(), Some("config file"));

    assert_eq!(
        identity.request_count(),
        0,
        "keys in a file cost no request"
    );
}

#[test]
fn where_both_files_name_one_profile_the_credentials_file_wins_key_by_key() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "both-files", &[]);
    let session = session
        .with_config_text(
            "[profile desk]\nregion = eu-west-1\noutput = json\naws_access_key_id = AKIACONFIG\naws_secret_access_key = config-secret\n",
        )
        .with_credentials_text(
            "[desk]\nregion = us-west-2\naws_access_key_id = AKIACREDENTIALS\naws_secret_access_key = credentials-secret\n",
        )
        .with_profile("desk");

    let profile = session.profile().expect("the profile both files name");
    assert_eq!(
        profile.region(),
        Some("us-west-2"),
        "the credentials file's value wins"
    );
    assert_eq!(
        profile.output(),
        Some("json"),
        "a key only the config file has is kept"
    );
    assert_eq!(found(&session).access_key_id(), "AKIACREDENTIALS");
    assert_eq!(session.credential_source(), Some("shared credentials file"));
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn a_profile_nobody_wrote_is_passed_over_rather_than_failing_the_walk() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "ghost-profile", &[("AWS_PROFILE", "ghost")]);

    assert_eq!(
        found(&session).access_key_id(),
        INSTANCE_KEY,
        "a profile that is not in either file is one absent source, not a refusal"
    );
    assert_eq!(session.credential_source(), Some("instance metadata"));
}

// --- a profile's role -------------------------------------------------------

const ROLE_CONFIG: &str = "\
[profile trader]
role_arn = arn:aws:iam::123456789012:role/lake-reader
source_profile = base
role_session_name = power-desk
external_id = desk-42
duration_seconds = 1800
";

const BASE_CREDENTIALS: &str = "\
[base]
aws_access_key_id = AKIABASE
aws_secret_access_key = base-secret
";

/// The SHA-1 botocore names this role's CLI cache file by: the exchange's
/// arguments as JSON, keys sorted, Python's default separators.
const ROLE_CACHE_KEY: &str = "ecd3883227681b9861a50b5d92de3b6e2d5d26cf";

#[test]
fn a_profile_role_chained_to_a_profile_with_keys_is_one_signed_exchange_filed_in_the_cli_cache() {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "profile-role", &[]);
    let session = session
        .with_config_text(ROLE_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("trader");

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIAlake-reader");
    assert_eq!(keys.session_token(), Some("token-lake-reader-power-desk"));
    assert!(keys.expires_at().is_some(), "an assumed role's set lapses");
    assert_eq!(keys.account_id(), Some("123456789012"));
    assert_eq!(session.credential_source(), Some("assumed role"));

    let requests = identity.requests();
    assert_eq!(requests.len(), 1, "one exchange: {:?}", shape(&identity));
    let exchange = &requests[0];
    assert!(exchange.is_sts("AssumeRole"), "{exchange:?}");
    assert_eq!(exchange.method, "GET");
    assert_eq!(exchange.query("Version"), Some("2011-06-15"));
    assert_eq!(
        exchange.query("RoleArn"),
        Some("arn:aws:iam::123456789012:role/lake-reader")
    );
    assert_eq!(exchange.query("RoleSessionName"), Some("power-desk"));
    assert_eq!(exchange.query("ExternalId"), Some("desk-42"));
    assert_eq!(exchange.query("DurationSeconds"), Some("1800"));
    assert_eq!(exchange.query("TokenCode"), None, "no MFA device was named");
    let authorization = header(exchange, "authorization");
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIABASE/"),
        "the source profile's keys sign the exchange: {authorization}"
    );
    assert!(
        authorization.contains("/eu-west-3/sts/aws4_request"),
        "signed for STS in the session's region: {authorization}"
    );
    assert_eq!(
        exchange.header("x-amz-security-token"),
        None,
        "a long-lived base carries no token"
    );

    let cached = read_json(
        &directory
            .join("cli")
            .join("cache")
            .join(format!("{ROLE_CACHE_KEY}.json")),
    );
    let held = &cached["Credentials"];
    assert_eq!(held["AccessKeyId"], "ASIAlake-reader");
    assert_eq!(held["SecretAccessKey"], "secret-of-lake-reader");
    assert_eq!(held["SessionToken"], "token-lake-reader-power-desk");
    let expiration = held["Expiration"].as_str().unwrap_or_default();
    assert!(
        expiration.ends_with("UTC"),
        "the CLI spells the zone as UTC: {expiration}"
    );
}

#[test]
fn a_second_session_on_the_same_directory_reads_the_role_from_the_cli_cache_without_sts() {
    let identity = Identity::start();
    let (first, directory) = walled(&identity, "role-cache", &[]);
    let first = first
        .with_config_text(ROLE_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("trader");
    assert_eq!(found(&first).access_key_id(), "ASIAlake-reader");
    assert_eq!(exchanges(&identity, "AssumeRole").len(), 1);

    identity.clear_requests();
    let second = again(&identity, &directory, &[])
        .with_config_text(ROLE_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("trader");
    let keys = found(&second);
    assert_eq!(keys.access_key_id(), "ASIAlake-reader");
    assert_eq!(keys.session_token(), Some("token-lake-reader-power-desk"));
    assert_eq!(second.credential_source(), Some("assumed role"));
    assert_eq!(
        identity.request_count(),
        0,
        "a cached session that lasts is not traded for again: {:?}",
        shape(&identity)
    );
}

#[test]
fn a_profile_that_names_itself_as_source_profile_signs_the_exchange_with_its_own_keys() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "self-source", &[]);
    let session = session
        .with_config_text(
            "[profile solo]\nrole_arn = arn:aws:iam::123456789012:role/solo-reader\nsource_profile = solo\n",
        )
        .with_credentials_text(
            "[solo]\naws_access_key_id = AKIASOLO\naws_secret_access_key = solo-secret\n",
        )
        .with_profile("solo");

    assert_eq!(found(&session).access_key_id(), "ASIAsolo-reader");
    assert_eq!(session.credential_source(), Some("assumed role"));
    let exchange = exchanges(&identity, "AssumeRole");
    assert_eq!(exchange.len(), 1);
    assert!(
        header(&exchange[0], "authorization").contains("Credential=AKIASOLO/"),
        "the profile's own keys sign its role: {}",
        header(&exchange[0], "authorization")
    );
}

#[test]
fn credential_source_environment_signs_the_exchange_with_the_environment_keys_and_token() {
    let identity = Identity::start();
    let (session, _) = walled(
        &identity,
        "credential-source",
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENVBASE"),
            ("AWS_SECRET_ACCESS_KEY", "env-base-secret"),
            ("AWS_SESSION_TOKEN", "env-base-token"),
        ],
    );
    let session = session
        .with_config_text(
            "[profile envrole]\nrole_arn = arn:aws:iam::123456789012:role/env-reader\ncredential_source = Environment\n",
        )
        .with_profile("envrole");

    assert_eq!(found(&session).access_key_id(), "ASIAenv-reader");
    assert_eq!(session.credential_source(), Some("assumed role"));
    let exchange = exchanges(&identity, "AssumeRole");
    assert_eq!(exchange.len(), 1, "{:?}", shape(&identity));
    assert!(header(&exchange[0], "authorization").contains("Credential=AKIAENVBASE/"));
    assert_eq!(
        exchange[0].header("x-amz-security-token"),
        Some("env-base-token"),
        "a temporary base presents its token with the exchange"
    );
}

#[test]
fn a_cycle_of_source_profiles_is_refused_by_name_rather_than_walked_forever() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "cycle", &[]);
    let session = session
        .with_config_text(
            "[profile a]\nrole_arn = arn:aws:iam::123456789012:role/a\nsource_profile = b\n\n\
             [profile b]\nrole_arn = arn:aws:iam::123456789012:role/b\nsource_profile = a\n",
        )
        .with_profile("a")
        .with_metadata_disabled(true);

    let message = refused(&session);
    assert!(message.contains("assumed role"), "{message}");
    assert!(message.contains("cycle"), "the refusal says why: {message}");
    assert_eq!(
        identity.request_count(),
        0,
        "no exchange is attempted without a base: {:?}",
        shape(&identity)
    );
}

// --- web identity -----------------------------------------------------------

#[test]
fn aws_role_arn_with_a_web_identity_token_file_is_one_unsigned_web_identity_exchange() {
    let identity = Identity::start();
    identity.require_web_identity_token(Some("web-identity-token"));
    let directory = scratch("web-identity-token");
    let token_file = directory.join("token");
    write(&token_file, "web-identity-token\n");
    let token_location = token_file.display().to_string();
    let (session, _) = walled(
        &identity,
        "web-identity",
        &[
            ("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/web-reader"),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", token_location.as_str()),
            ("AWS_ROLE_SESSION_NAME", "pod-7"),
        ],
    );

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIAweb-reader");
    assert_eq!(keys.session_token(), Some("token-web-reader-pod-7"));
    assert_eq!(session.credential_source(), Some("web identity"));

    let requests = identity.requests();
    assert_eq!(requests.len(), 1, "{:?}", shape(&identity));
    let exchange = &requests[0];
    assert!(exchange.is_sts("AssumeRoleWithWebIdentity"), "{exchange:?}");
    assert_eq!(
        exchange.query("RoleArn"),
        Some("arn:aws:iam::123456789012:role/web-reader")
    );
    assert_eq!(exchange.query("RoleSessionName"), Some("pod-7"));
    assert_eq!(
        exchange.query("WebIdentityToken"),
        Some("web-identity-token"),
        "the token file's content, trimmed"
    );
    assert_eq!(exchange.query("DurationSeconds"), Some("3600"));
    assert_eq!(
        exchange.header("authorization"),
        None,
        "the token is the proof, so the exchange is unsigned"
    );
}

#[test]
fn a_profile_s_web_identity_token_file_is_traded_unsigned_under_a_generated_session_name() {
    let identity = Identity::start();
    identity.require_web_identity_token(Some("pod-token"));
    let (session, directory) = walled(&identity, "profile-web-identity", &[]);
    let token_file = directory.join("pod-token");
    write(&token_file, "pod-token");
    let session = session
        .with_config_text(format!(
            "[profile pod]\nrole_arn = arn:aws:iam::123456789012:role/pod-reader\nweb_identity_token_file = {}\n",
            token_file.display()
        ))
        .with_profile("pod");

    assert_eq!(found(&session).access_key_id(), "ASIApod-reader");
    assert_eq!(session.credential_source(), Some("assumed role"));
    let exchange = exchanges(&identity, "AssumeRoleWithWebIdentity");
    assert_eq!(exchange.len(), 1, "{:?}", shape(&identity));
    assert_eq!(exchange[0].query("WebIdentityToken"), Some("pod-token"));
    assert_eq!(exchange[0].header("authorization"), None);
    let name = exchange[0].query("RoleSessionName").unwrap_or_default();
    assert!(
        name.starts_with("yggdryl-session-"),
        "a session name nobody chose is generated: {name}"
    );
    assert!(exchanges(&identity, "AssumeRole").is_empty());
}

// --- MFA --------------------------------------------------------------------

const MFA_CONFIG: &str = "\
[profile guarded]
role_arn = arn:aws:iam::123456789012:role/mfa-reader
source_profile = base
mfa_serial = arn:aws:iam::123456789012:mfa/trader
";

#[test]
fn an_mfa_role_asks_the_prompt_for_the_serial_s_code_and_presents_it_to_sts() {
    let identity = Identity::start();
    identity.require_token_code(Some("123456"));
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = Arc::clone(&asked);
    let (session, _) = walled(&identity, "mfa", &[]);
    let session = session
        .with_config_text(MFA_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("guarded")
        .with_mfa_prompt(Arc::new(move |serial: &str| -> Option<String> {
            record
                .lock()
                .expect("the prompt's record")
                .push(serial.to_owned());
            Some("123456".to_owned())
        }));

    assert_eq!(found(&session).access_key_id(), "ASIAmfa-reader");
    assert_eq!(
        *asked.lock().expect("the prompt's record"),
        vec!["arn:aws:iam::123456789012:mfa/trader".to_owned()],
        "the prompt is asked once, for the device the profile names"
    );
    let exchange = exchanges(&identity, "AssumeRole");
    assert_eq!(exchange.len(), 1, "{:?}", shape(&identity));
    assert_eq!(
        exchange[0].query("SerialNumber"),
        Some("arn:aws:iam::123456789012:mfa/trader")
    );
    assert_eq!(exchange[0].query("TokenCode"), Some("123456"));
}

#[test]
fn an_mfa_role_without_a_prompt_is_a_failed_source_and_the_chain_walks_on() {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "mfa-no-prompt", &[]);
    let session = session
        .with_config_text(MFA_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("guarded");

    assert_eq!(
        found(&session).access_key_id(),
        INSTANCE_KEY,
        "the role could not be assumed, and the instance's role still answers"
    );
    assert_eq!(session.credential_source(), Some("instance metadata"));
    assert!(
        exchanges(&identity, "AssumeRole").is_empty(),
        "no exchange is sent without the code it needs: {:?}",
        shape(&identity)
    );

    identity.set_imds_role(None);
    let alone = again(&identity, &directory, &[])
        .with_config_text(MFA_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("guarded");
    let message = refused(&alone);
    assert!(message.contains("assumed role"), "{message}");
    assert!(
        message.contains("arn:aws:iam::123456789012:mfa/trader"),
        "{message}"
    );
    assert!(
        message.contains("Session::with_mfa_prompt"),
        "the refusal names the way out: {message}"
    );

    let declined = again(&identity, &directory, &[])
        .with_config_text(MFA_CONFIG)
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("guarded")
        .with_mfa_prompt(Arc::new(|_: &str| -> Option<String> { None }));
    let message = refused(&declined);
    assert!(
        message.contains("no code was given"),
        "a person who declined is a refusal of its own: {message}"
    );
    assert!(exchanges(&identity, "AssumeRole").is_empty());
}

// --- IAM Identity Center ----------------------------------------------------

const SSO_CONFIG: &str = "\
[profile desk]
sso_session = corp
sso_account_id = 123456789012
sso_role_name = LakeReader

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1
sso_registration_scopes = sso:account:access
";

const CORP_START_URL: &str = "https://corp.awsapps.com/start";

/// `sha1("corp")`: the token of a sign-in that belongs to an `[sso-session]`
/// is filed under the session's name.
const CORP_TOKEN_KEY: &str = "ee0bfd2552fbd840c02cc48b6e823320543c450f";

/// The SHA-1 of the compact, key-sorted JSON botocore keys the role's keys
/// by: `{"accountId":..,"roleName":..,"sessionName":"corp"}` - the session
/// name in place of the start URL where the sign-in belongs to one.
const CORP_CREDENTIALS_KEY: &str = "c0d9da29b60937b7b6389cf6e5796a56a300cabf";

const LEGACY_SSO_CONFIG: &str = "\
[profile legacy]
sso_start_url = https://legacy.awsapps.com/start
sso_region = eu-west-1
sso_account_id = 210987654321
sso_role_name = Auditor
";

/// `sha1("https://legacy.awsapps.com/start")`: the older profile shape files
/// its token under the start URL.
const LEGACY_TOKEN_KEY: &str = "79e435d7a515078e81c9dffc35f38d5687ebd3a7";

/// The same key for the legacy shape, which has no `sessionName`.
const LEGACY_CREDENTIALS_KEY: &str = "0e2164ab0423b65e257a7dbbfad8bdf56a142c69";

fn token_path(directory: &Path, key: &str) -> PathBuf {
    directory
        .join("sso")
        .join("cache")
        .join(format!("{key}.json"))
}

fn cli_cache_path(directory: &Path, key: &str) -> PathBuf {
    directory
        .join("cli")
        .join("cache")
        .join(format!("{key}.json"))
}

/// A sign-in token as `aws sso login` files it.
fn write_token(directory: &Path, key: &str, document: &serde_json::Value) {
    write(&token_path(directory, key), &document.to_string());
}

fn corp_sso() -> Sso {
    Sso::new(CORP_START_URL, "eu-west-1", "123456789012", "LakeReader").with_session_name("corp")
}

#[test]
fn a_cached_sign_in_is_traded_at_the_portal_once_with_the_bearer_header_and_filed_for_the_cli() {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "sso-cached", &[]);
    write_token(
        &directory,
        CORP_TOKEN_KEY,
        &serde_json::json!({
            "startUrl": CORP_START_URL,
            "region": "eu-west-1",
            "accessToken": SSO_TOKEN,
            "expiresAt": FAR,
        }),
    );
    let session = session.with_config_text(SSO_CONFIG).with_profile("desk");

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIALakeReader");
    assert_eq!(keys.session_token(), Some("sso-session-token-LakeReader"));
    assert_eq!(keys.account_id(), Some("123456789012"));
    assert!(keys.expires_at().is_some());
    assert_eq!(session.credential_source(), Some("sso"));

    let requests = identity.requests();
    assert_eq!(
        shape(&identity),
        ["GET /federation/credentials"],
        "one portal request, no refresh, no sign-in"
    );
    let portal = &requests[0];
    assert_eq!(portal.query("role_name"), Some("LakeReader"));
    assert_eq!(portal.query("account_id"), Some("123456789012"));
    assert_eq!(portal.header("x-amz-sso_bearer_token"), Some(SSO_TOKEN));

    let cached = read_json(&cli_cache_path(&directory, CORP_CREDENTIALS_KEY));
    assert_eq!(cached["Credentials"]["AccessKeyId"], "ASIALakeReader");
    assert_eq!(
        cached["Credentials"]["SessionToken"],
        "sso-session-token-LakeReader"
    );

    identity.clear_requests();
    let second = again(&identity, &directory, &[])
        .with_config_text(SSO_CONFIG)
        .with_profile("desk");
    assert_eq!(found(&second).access_key_id(), "ASIALakeReader");
    assert_eq!(second.credential_source(), Some("sso"));
    assert_eq!(
        identity.request_count(),
        0,
        "the keys the CLI cache holds are not asked for again: {:?}",
        shape(&identity)
    );
}

#[test]
fn the_legacy_sso_profile_shape_files_its_token_under_the_start_url() {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "sso-legacy", &[]);
    write_token(
        &directory,
        LEGACY_TOKEN_KEY,
        &serde_json::json!({
            "startUrl": "https://legacy.awsapps.com/start",
            "region": "eu-west-1",
            "accessToken": SSO_TOKEN,
            "expiresAt": FAR,
        }),
    );
    let session = session
        .with_config_text(LEGACY_SSO_CONFIG)
        .with_profile("legacy");

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIAAuditor");
    assert_eq!(keys.account_id(), Some("210987654321"));
    assert_eq!(session.credential_source(), Some("sso"));
    let requests = identity.requests();
    assert_eq!(shape(&identity), ["GET /federation/credentials"]);
    assert_eq!(requests[0].query("account_id"), Some("210987654321"));
    assert_eq!(
        requests[0].header("x-amz-sso_bearer_token"),
        Some(SSO_TOKEN)
    );
    let cached = read_json(&cli_cache_path(&directory, LEGACY_CREDENTIALS_KEY));
    assert_eq!(cached["Credentials"]["AccessKeyId"], "ASIAAuditor");
}

#[test]
fn a_lapsed_sign_in_that_cannot_refresh_is_a_failed_source_naming_aws_sso_login_and_the_chain_walks_on()
 {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "sso-lapsed", &[]);
    write_token(
        &directory,
        CORP_TOKEN_KEY,
        &serde_json::json!({
            "startUrl": CORP_START_URL,
            "region": "eu-west-1",
            "accessToken": "lapsed-token",
            "expiresAt": "2000-01-01T00:00:00Z",
        }),
    );
    let session = session
        .with_config_text(SSO_CONFIG)
        .with_profile("desk")
        .with_sso_login(SsoLogin::Never);

    assert_eq!(
        found(&session).access_key_id(),
        INSTANCE_KEY,
        "a lapsed sign-in is passed over, and the instance's role answers"
    );
    assert_eq!(session.credential_source(), Some("instance metadata"));
    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "no refresh without a client, no sign-in under SsoLogin::Never, no portal request"
    );

    identity.set_imds_role(None);
    let alone = again(&identity, &directory, &[])
        .with_config_text(SSO_CONFIG)
        .with_profile("desk");
    let message = refused(&alone);
    assert!(message.contains("sso"), "{message}");
    assert!(
        message.contains("`aws sso login`"),
        "the refusal names the way out: {message}"
    );
    assert!(message.contains("nothing configured in"), "{message}");
    assert!(message.contains("instance metadata"), "{message}");
}

#[test]
fn a_lapsed_sign_in_that_registered_a_client_is_refreshed_filed_again_and_traded() {
    let identity = Identity::start();
    identity.set_sso_token("fresh-sso-token");
    identity.require_refresh_token(Some("cached-refresh-token"));
    let (session, directory) = walled(&identity, "sso-refresh", &[]);
    write_token(
        &directory,
        CORP_TOKEN_KEY,
        &serde_json::json!({
            "startUrl": CORP_START_URL,
            "region": "eu-west-1",
            "accessToken": "lapsed-token",
            "expiresAt": "2000-01-01T00:00:00Z",
            "clientId": "client-1",
            "clientSecret": "client-secret-1",
            "registrationExpiresAt": FAR,
            "refreshToken": "cached-refresh-token",
        }),
    );
    let session = session.with_config_text(SSO_CONFIG).with_profile("desk");

    assert_eq!(found(&session).access_key_id(), "ASIALakeReader");
    assert_eq!(session.credential_source(), Some("sso"));
    assert_eq!(
        shape(&identity),
        ["POST /token", "GET /federation/credentials"],
        "one refresh, then one portal request"
    );
    let requests = identity.requests();
    let refresh: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("a JSON refresh");
    assert_eq!(refresh["grantType"], "refresh_token");
    assert_eq!(refresh["refreshToken"], "cached-refresh-token");
    assert_eq!(refresh["clientId"], "client-1");
    assert_eq!(
        requests[1].header("x-amz-sso_bearer_token"),
        Some("fresh-sso-token"),
        "the refreshed token is what is traded"
    );

    let filed = read_json(&token_path(&directory, CORP_TOKEN_KEY));
    assert_eq!(filed["accessToken"], "fresh-sso-token");
    assert_eq!(filed["refreshToken"], SSO_REFRESH_TOKEN);
    assert_eq!(filed["clientId"], "client-1", "the registration is kept");
    assert_eq!(filed["startUrl"], CORP_START_URL);
    assert_ne!(filed["expiresAt"], "2000-01-01T00:00:00Z");
}

#[test]
fn a_sign_in_near_its_expiry_whose_refresh_fails_is_still_traded_while_it_stands() {
    let identity = Identity::start();
    identity.require_refresh_token(Some("the-only-refresh-token-the-service-knows"));
    let (session, directory) = walled(&identity, "sso-refresh-fails", &[]);
    let expires_at = iso8601(now_seconds() + 300);
    write_token(
        &directory,
        CORP_TOKEN_KEY,
        &serde_json::json!({
            "startUrl": CORP_START_URL,
            "region": "eu-west-1",
            "accessToken": SSO_TOKEN,
            "expiresAt": expires_at,
            "clientId": "client-1",
            "clientSecret": "client-secret-1",
            "registrationExpiresAt": FAR,
            "refreshToken": "revoked-refresh-token",
        }),
    );
    let session = session.with_config_text(SSO_CONFIG).with_profile("desk");

    assert_eq!(
        found(&session).access_key_id(),
        "ASIALakeReader",
        "a failed refresh keeps a token that still stands"
    );
    assert_eq!(
        shape(&identity),
        ["POST /token", "GET /federation/credentials"]
    );
    assert_eq!(
        identity.requests()[0].status,
        400,
        "the refresh was refused"
    );
    let filed = read_json(&token_path(&directory, CORP_TOKEN_KEY));
    assert_eq!(
        filed["accessToken"], SSO_TOKEN,
        "the held token is not overwritten"
    );
}

#[test]
fn a_sign_in_nobody_made_is_made_through_the_device_flow_when_the_session_says_how_to_show_it() {
    let identity = Identity::start();
    identity.sso_pending_polls(1);
    let shown = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = Arc::clone(&shown);
    let (session, directory) = walled(&identity, "sso-device", &[]);
    let session = session
        .with_sso(corp_sso())
        .with_sso_login(SsoLogin::Handler(Arc::new(
            move |authorization: &DeviceAuthorization| {
                record
                    .lock()
                    .expect("the handler's record")
                    .push(authorization.user_code().to_owned());
            },
        )));

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIALakeReader");
    assert_eq!(session.credential_source(), Some("sso"));
    assert_eq!(
        *shown.lock().expect("the handler's record"),
        vec!["ABCD-EFGH".to_owned()],
        "the person is shown the code once"
    );
    assert_eq!(
        shape(&identity),
        [
            "POST /client/register",
            "POST /device_authorization",
            "POST /token",
            "POST /token",
            "GET /federation/credentials",
        ],
        "register, authorize, poll until confirmed, trade"
    );
    let requests = identity.requests();
    let registered: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("a JSON registration");
    assert_eq!(registered["clientType"], "public");
    assert_eq!(
        registered["scopes"],
        serde_json::json!(["sso:account:access"])
    );
    let started: serde_json::Value =
        serde_json::from_str(&requests[1].body).expect("a JSON authorization");
    assert_eq!(started["startUrl"], CORP_START_URL);
    let polled: serde_json::Value = serde_json::from_str(&requests[2].body).expect("a JSON poll");
    assert_eq!(
        polled["grantType"],
        "urn:ietf:params:oauth:grant-type:device_code"
    );
    assert_eq!(polled["deviceCode"], "device-1");

    let filed = read_json(&token_path(&directory, CORP_TOKEN_KEY));
    assert_eq!(filed["accessToken"], SSO_TOKEN);
    assert_eq!(filed["refreshToken"], SSO_REFRESH_TOKEN);
    assert_eq!(filed["clientId"], "client-1");
    assert_eq!(filed["clientSecret"], "client-secret-1");
    assert_eq!(filed["startUrl"], CORP_START_URL);
    assert_eq!(filed["region"], "eu-west-1");
}

#[test]
fn login_files_the_sign_in_where_a_later_session_that_never_signs_in_reads_it() {
    let identity = Identity::start();
    let (session, directory) = walled(&identity, "sso-login", &[]);
    let session = session
        .with_sso(corp_sso())
        .with_sso_login(SsoLogin::Handler(Arc::new(|_: &DeviceAuthorization| {})));

    session.login().expect("a sign-in");
    assert_eq!(
        shape(&identity),
        [
            "POST /client/register",
            "POST /device_authorization",
            "POST /token",
        ]
    );
    assert_eq!(
        read_json(&token_path(&directory, CORP_TOKEN_KEY))["accessToken"],
        SSO_TOKEN
    );

    identity.clear_requests();
    let later = again(&identity, &directory, &[]).with_sso(corp_sso());
    assert!(matches!(later.sso_login(), SsoLogin::Never));
    assert_eq!(found(&later).access_key_id(), "ASIALakeReader");
    assert_eq!(shape(&identity), ["GET /federation/credentials"]);
}

#[test]
fn login_refuses_without_a_sign_in_to_make_or_a_way_to_show_it() {
    let identity = Identity::start();
    let (nothing, _) = walled(&identity, "sso-login-nothing", &[]);
    let message = nothing
        .login()
        .expect_err("no sign-in is configured")
        .to_string();
    assert!(
        message.contains("names an IAM Identity Center sign-in"),
        "{message}"
    );

    let (never, _) = walled(&identity, "sso-login-never", &[]);
    let message = never
        .with_sso(corp_sso())
        .login()
        .expect_err("SsoLogin::Never makes no sign-in")
        .to_string();
    assert!(message.contains("`aws sso login`"), "{message}");
    assert_eq!(
        identity.request_count(),
        0,
        "a refused sign-in asks nothing"
    );
}

// --- the container endpoint -------------------------------------------------

#[test]
fn the_container_endpoint_answers_with_the_authorization_token_the_environment_holds() {
    let identity = Identity::start();
    identity.require_container_authorization(Some("container-token-1"));
    let uri = identity.container_uri();
    let (session, _) = walled(
        &identity,
        "container",
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "container-token-1"),
        ],
    );

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIACONTAINER");
    assert_eq!(keys.session_token(), Some("container-token"));
    assert_eq!(session.credential_source(), Some("container"));
    let requests = identity.requests();
    assert_eq!(
        shape(&identity),
        [format!("GET {CONTAINER_PATH}")],
        "one request, and the instance metadata service is never reached"
    );
    assert_eq!(
        requests[0].header("authorization"),
        Some("container-token-1")
    );
}

#[test]
fn the_container_authorization_token_file_wins_over_the_token_variable() {
    let identity = Identity::start();
    identity.require_container_authorization(Some("container-token-2"));
    identity.set_container_credentials("ASIATASK", FAR);
    let (session, directory) = walled(&identity, "container-token-file", &[]);
    let token_file = directory.join("container-token");
    write(&token_file, "container-token-2\n");
    let uri = identity.container_uri();
    let token_location = token_file.display().to_string();
    let session = session.with_variables(variables(
        &directory,
        &[
            ("AWS_CONTAINER_CREDENTIALS_FULL_URI", uri.as_str()),
            (
                "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
                token_location.as_str(),
            ),
            ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "stale-token"),
        ],
    ));

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), "ASIATASK");
    assert_eq!(
        keys.expires_at(),
        Some(UNIX_EPOCH + Duration::from_secs(FAR_SECONDS))
    );
    assert_eq!(session.credential_source(), Some("container"));
    assert_eq!(
        identity.requests()[0].header("authorization"),
        Some("container-token-2"),
        "the file's token, trimmed of its line break"
    );
}

// --- the instance metadata service ------------------------------------------

#[test]
fn the_instance_metadata_service_answers_its_role_s_keys_over_an_imdsv2_session() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "imds", &[]);

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), INSTANCE_KEY);
    assert_eq!(keys.session_token(), Some("token-of-instance-role"));
    assert!(keys.expires_at().is_some());
    assert_eq!(session.credential_source(), Some("instance metadata"));

    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "a token, the listing, the role's keys"
    );
    let requests = identity.requests();
    assert_eq!(
        requests[0].header("x-aws-ec2-metadata-token-ttl-seconds"),
        Some("21600")
    );
    for read in &requests[1..] {
        assert_eq!(
            read.header("x-aws-ec2-metadata-token"),
            Some(IMDS_TOKEN),
            "every read presents the session token: {read:?}"
        );
    }
}

#[test]
fn an_instance_without_a_role_is_nothing_configured_rather_than_a_refusal() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let (session, _) = walled(&identity, "imds-no-role", &[]);

    assert_eq!(
        session
            .credentials(SystemTime::now())
            .expect("an absent source is not a failure"),
        None
    );
    assert_eq!(session.credential_source(), None);
    assert_eq!(
        shape(&identity),
        [
            "PUT /latest/api/token",
            "GET /latest/meta-data/iam/security-credentials/",
        ]
    );
}

#[test]
fn a_disabled_metadata_service_is_never_asked_for_keys_or_for_its_region() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "imds-disabled", &[]);
    let session = session.with_metadata_disabled(true);

    assert_eq!(
        session
            .credentials(SystemTime::now())
            .expect("nothing configured is not a refusal"),
        None
    );
    assert_eq!(session.instance_region(), None);
    assert_eq!(
        identity.request_count(),
        0,
        "disabled means no request at all"
    );

    let (by_variable, _) = walled(
        &identity,
        "imds-disabled-variable",
        &[("AWS_EC2_METADATA_DISABLED", "true")],
    );
    assert_eq!(
        by_variable
            .credentials(SystemTime::now())
            .expect("nothing configured"),
        None
    );
    assert_eq!(
        identity.request_count(),
        0,
        "AWS_EC2_METADATA_DISABLED disables it too"
    );
}

#[test]
fn the_instance_region_is_the_identity_document_s_asked_when_asked() {
    let identity = Identity::start();
    identity.set_imds_region("ap-south-1");
    let (session, _) = walled(&identity, "imds-region", &[]);

    assert_eq!(
        session.region().as_deref(),
        Some("eu-west-3"),
        "the region is the stated one; asking the instance is a question of its own"
    );
    assert_eq!(identity.request_count(), 0);
    assert_eq!(session.instance_region().as_deref(), Some("ap-south-1"));
    assert_eq!(
        shape(&identity),
        [
            "PUT /latest/api/token",
            "GET /latest/dynamic/instance-identity/document",
        ]
    );
}

// --- best effort, then a named refusal --------------------------------------

#[test]
fn a_broken_profile_role_is_recorded_and_passed_over_when_the_instance_answers() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "broken-best-effort", &[]);
    let session = session
        .with_config_text("[profile broken]\nrole_arn = arn:aws:iam::123456789012:role/orphan\n")
        .with_profile("broken");

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(session.credential_source(), Some("instance metadata"));
    assert_eq!(
        shape(&identity),
        IMDS_WALK,
        "a role with no source asks STS nothing"
    );
}

#[test]
fn nothing_answering_is_a_refusal_naming_each_failure_and_each_absent_source_held_for_thirty_seconds()
 {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let (session, _) = walled(&identity, "broken-refusal", &[]);
    let session = session
        .with_config_text("[profile broken]\nrole_arn = arn:aws:iam::123456789012:role/orphan\n")
        .with_profile("broken");

    let now = SystemTime::now();
    let first = session
        .credentials(now)
        .expect_err("a configured source failed and nothing answered")
        .to_string();
    assert!(
        first.contains("no AWS credentials could be obtained"),
        "{first}"
    );
    assert!(first.contains("assumed role"), "{first}");
    assert!(
        first.contains("the profile broken names role_arn without"),
        "the failure says what is wrong with which profile: {first}"
    );
    assert!(first.contains("nothing configured in"), "{first}");
    assert!(first.contains("container"), "{first}");
    assert!(first.contains("instance metadata"), "{first}");
    let walked = identity.request_count();
    assert_eq!(
        walked, 2,
        "one walk asked the metadata service for a token and a role"
    );

    let second = session
        .credentials(now + Duration::from_secs(10))
        .expect_err("the failure is answered again")
        .to_string();
    assert!(second.contains("names role_arn without"), "{second}");
    assert_eq!(
        identity.request_count(),
        walked,
        "within the pause the failure is answered without walking again"
    );

    let third = session
        .credentials(now + Duration::from_secs(31))
        .expect_err("still nothing answers")
        .to_string();
    assert!(third.contains("names role_arn without"), "{third}");
    assert_eq!(
        identity.request_count(),
        walked + 2,
        "after the pause the chain is walked again"
    );
}

#[test]
fn malformed_environment_keys_are_a_failed_source_and_the_chain_walks_on() {
    let identity = Identity::start();
    let (half, directory) = walled(&identity, "half-keys", &[("AWS_ACCESS_KEY_ID", "AKIAHALF")]);
    assert_eq!(
        found(&half).access_key_id(),
        INSTANCE_KEY,
        "a key without its secret is passed over"
    );

    identity.set_imds_role(None);
    let alone = again(&identity, &directory, &[("AWS_ACCESS_KEY_ID", "AKIAHALF")]);
    let message = refused(&alone);
    assert!(message.contains("environment"), "{message}");
    assert!(
        message.contains("AWS_ACCESS_KEY_ID is set without AWS_SECRET_ACCESS_KEY"),
        "{message}"
    );

    let lapsing = again(
        &identity,
        &directory,
        &[
            ("AWS_ACCESS_KEY_ID", "AKIAENV"),
            ("AWS_SECRET_ACCESS_KEY", "env-secret"),
            ("AWS_CREDENTIAL_EXPIRATION", "next tuesday"),
        ],
    );
    let message = refused(&lapsing);
    assert!(
        message.contains("AWS_CREDENTIAL_EXPIRATION is not an ISO 8601 instant"),
        "{message}"
    );
    assert!(
        !message.contains("env-secret"),
        "a refusal never renders a secret: {message}"
    );
}

// --- refresh and invalidation -----------------------------------------------

#[test]
fn a_set_that_is_stale_at_once_is_traded_again_on_every_ask_until_one_lasts() {
    let identity = Identity::start();
    identity.expire_sessions(true);
    let (session, _) = walled(&identity, "refresh", &[]);
    let session = session
        .with_config_text(
            "[profile trader]\nrole_arn = arn:aws:iam::123456789012:role/lake-reader\nsource_profile = base\n",
        )
        .with_credentials_text(BASE_CREDENTIALS)
        .with_profile("trader");

    let now = SystemTime::now();
    let first = session
        .credentials(now)
        .expect("an exchange")
        .expect("a set");
    assert_eq!(first.access_key_id(), "ASIAlake-reader");
    session
        .credentials(now)
        .expect("an exchange")
        .expect("a set");
    assert_eq!(
        exchanges(&identity, "AssumeRole").len(),
        2,
        "a set inside its refresh window is replaced rather than signed with"
    );

    identity.expire_sessions(false);
    session
        .credentials(now)
        .expect("an exchange")
        .expect("a set");
    session
        .credentials(now)
        .expect("the set in hand")
        .expect("a set");
    assert_eq!(
        exchanges(&identity, "AssumeRole").len(),
        3,
        "a set that lasts is held rather than traded again"
    );
}

#[test]
fn a_found_set_is_held_until_invalidate_forgets_it() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "invalidate", &[]);

    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(identity.request_count(), 3);
    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(
        identity.request_count(),
        3,
        "the set in hand costs no request"
    );

    session.invalidate();
    assert_eq!(
        session.credential_source(),
        None,
        "invalidated, nothing is in hand"
    );
    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
    assert_eq!(
        identity.request_count(),
        6,
        "the next ask walks the chain again"
    );
    assert_eq!(session.credential_source(), Some("instance metadata"));
}

#[test]
fn nothing_configured_is_held_as_unsigned_until_invalidate() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let (session, _) = walled(&identity, "anonymous-held", &[]);

    assert_eq!(
        session.credentials(SystemTime::now()).expect("unsigned"),
        None
    );
    let walked = identity.request_count();

    identity.set_imds_role(Some("instance-role"));
    assert_eq!(
        session.credentials(SystemTime::now()).expect("unsigned"),
        None,
        "an unsigned session stays unsigned"
    );
    assert_eq!(
        identity.request_count(),
        walked,
        "and walks nothing to find out"
    );

    session.invalidate();
    assert_eq!(found(&session).access_key_id(), INSTANCE_KEY);
}

// --- the explicit role ------------------------------------------------------

#[test]
fn an_explicit_role_is_traded_for_with_the_explicit_credentials_signing_the_exchange() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "explicit-role", &[]);
    let session = session
        .with_credentials(
            Credentials::new("AKIAEXPLICIT", "explicit-secret")
                .with_session_token("explicit-token"),
        )
        .with_assumed_role(
            AssumedRole::new("arn:aws:iam::123456789012:role/lake-writer")
                .with_session_name("desk")
                .with_external_id("ext-1"),
        );

    assert_eq!(
        found(&session).access_key_id(),
        "ASIAlake-writer",
        "the role is what signs, not the keys that traded for it"
    );
    assert_eq!(session.credential_source(), Some("assumed role"));
    let requests = identity.requests();
    assert_eq!(requests.len(), 1, "{:?}", shape(&identity));
    let exchange = &requests[0];
    assert!(exchange.is_sts("AssumeRole"));
    assert_eq!(exchange.query("RoleSessionName"), Some("desk"));
    assert_eq!(exchange.query("ExternalId"), Some("ext-1"));
    assert!(
        header(exchange, "authorization").contains("Credential=AKIAEXPLICIT/"),
        "{}",
        header(exchange, "authorization")
    );
    assert_eq!(
        exchange.header("x-amz-security-token"),
        Some("explicit-token")
    );
}

#[test]
fn a_refused_explicit_role_is_a_refusal_naming_sts_s_code_never_the_keys_beneath_it() {
    let identity = Identity::start();
    identity.refuse_sts("AccessDenied", 1);
    let (session, _) = walled(&identity, "explicit-role-refused", &[]);
    let session = session
        .with_credentials(Credentials::new("AKIAEXPLICIT", "explicit-secret"))
        .with_assumed_role(AssumedRole::new(
            "arn:aws:iam::123456789012:role/lake-writer",
        ));

    let now = SystemTime::now();
    let message = session
        .credentials(now)
        .expect_err("a role that cannot be assumed is not a request signed as somebody else")
        .to_string();
    assert!(message.contains("assumed role"), "{message}");
    assert!(message.contains("AccessDenied"), "{message}");
    assert!(!message.contains("explicit-secret"), "{message}");
    assert_eq!(exchanges(&identity, "AssumeRole").len(), 1);

    // Once STS relents and the pause has passed, the role is traded for.
    let keys = session
        .credentials(now + Duration::from_secs(31))
        .expect("the exchange succeeds")
        .expect("a set");
    assert_eq!(keys.access_key_id(), "ASIAlake-writer");
    assert_eq!(exchanges(&identity, "AssumeRole").len(), 2);
}

#[test]
fn an_explicit_role_naming_a_source_profile_is_signed_by_that_profile_and_walks_no_chain() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "explicit-role-source-profile", &[]);
    let session = session
        .with_credentials_text(BASE_CREDENTIALS)
        .with_assumed_role(
            AssumedRole::new("arn:aws:iam::123456789012:role/lake-writer")
                .with_session_name("desk")
                .with_source_profile("base"),
        );

    assert_eq!(found(&session).access_key_id(), "ASIAlake-writer");
    assert_eq!(session.credential_source(), Some("assumed role"));
    let walked = shape(&identity);
    assert_eq!(
        walked.len(),
        1,
        "one exchange, and no walk of the chain: {walked:?}"
    );
    let exchange = exchanges(&identity, "AssumeRole");
    assert!(
        header(&exchange[0], "authorization").contains("Credential=AKIABASE/"),
        "{}",
        header(&exchange[0], "authorization")
    );

    let (missing, _) = walled(&identity, "explicit-role-source-missing", &[]);
    let message = refused(
        &missing.with_assumed_role(
            AssumedRole::new("arn:aws:iam::123456789012:role/lake-writer")
                .with_source_profile("nobody"),
        ),
    );
    assert!(
        message.contains(
            "the role arn:aws:iam::123456789012:role/lake-writer names source_profile nobody, \
             which is in neither file"
        ),
        "{message}"
    );
    assert_eq!(
        exchanges(&identity, "AssumeRole").len(),
        1,
        "a source that is not there is not a chain to walk instead"
    );

    let (walled_off, _) = walled(&identity, "explicit-role-source-sealed", &[]);
    let message = refused(
        &walled_off.with_environment(false).with_assumed_role(
            AssumedRole::new("arn:aws:iam::123456789012:role/lake-writer")
                .with_credential_source(CredentialSource::Environment),
        ),
    );
    assert!(
        message.contains("names credential_source, and the session consults no environment"),
        "{message}"
    );
    assert_eq!(
        exchanges(&identity, "AssumeRole").len(),
        1,
        "a session consulting no environment reads no keys off the process"
    );
}

#[test]
fn an_explicit_role_wraps_whatever_the_rest_of_the_chain_answers() {
    let identity = Identity::start();
    let (session, _) = walled(&identity, "explicit-role-over-chain", &[]);
    let session = session.with_assumed_role(
        AssumedRole::new("arn:aws:iam::123456789012:role/lake-writer").with_session_name("desk"),
    );

    assert_eq!(found(&session).access_key_id(), "ASIAlake-writer");
    assert_eq!(session.credential_source(), Some("assumed role"));
    let walked = shape(&identity);
    assert_eq!(
        walked.len(),
        4,
        "the instance's keys, then one exchange: {walked:?}"
    );
    assert_eq!(walked[..3], IMDS_WALK);
    let exchange = exchanges(&identity, "AssumeRole");
    assert_eq!(exchange.len(), 1);
    assert!(
        header(&exchange[0], "authorization").contains(&format!("Credential={INSTANCE_KEY}/")),
        "{}",
        header(&exchange[0], "authorization")
    );
    assert_eq!(
        exchange[0].header("x-amz-security-token"),
        Some("token-of-instance-role")
    );

    identity.set_imds_role(None);
    let (bare, _) = walled(&identity, "explicit-role-bare", &[]);
    let message = refused(&bare.with_assumed_role(AssumedRole::new(
        "arn:aws:iam::123456789012:role/lake-writer",
    )));
    assert!(
        message.contains("found no keys to assume arn:aws:iam::123456789012:role/lake-writer"),
        "{message}"
    );
    assert_eq!(
        exchanges(&identity, "AssumeRole").len(),
        1,
        "no unsigned exchange is tried"
    );
}

// --- what resolves without a socket -----------------------------------------

#[test]
fn the_profile_is_the_stated_one_then_aws_default_profile_then_aws_profile_then_default() {
    assert_eq!(offline("profile-none", &[]).profile_name(), "default");
    assert_eq!(
        offline("profile-default", &[("AWS_DEFAULT_PROFILE", "fallback")]).profile_name(),
        "fallback"
    );
    let both = offline(
        "profile-both",
        &[("AWS_PROFILE", "desk"), ("AWS_DEFAULT_PROFILE", "fallback")],
    );
    assert_eq!(
        both.profile_name(),
        "fallback",
        "AWS_DEFAULT_PROFILE first, the order botocore reads the pair in"
    );
    assert_eq!(both.with_profile("stated").profile_name(), "stated");
    assert_eq!(
        both.with_profile("stated")
            .with_profile("  ")
            .profile_name(),
        "fallback",
        "a blank name states nothing"
    );
    assert_eq!(
        offline("profile-blank", &[("AWS_PROFILE", "   ")]).profile_name(),
        "default",
        "a blank variable is unset"
    );
}

const REGIONS: &str = "\
[default]
region = sa-east-1

[profile desk]
region = ca-central-1
";

#[test]
fn the_region_is_the_stated_one_then_aws_region_then_aws_default_region_then_the_profile_s() {
    assert_eq!(offline("region-none", &[]).region(), None);
    assert_eq!(
        offline("region-profile", &[])
            .with_config_text(REGIONS)
            .region()
            .as_deref(),
        Some("sa-east-1")
    );
    assert_eq!(
        offline("region-named-profile", &[("AWS_PROFILE", "desk")])
            .with_config_text(REGIONS)
            .region()
            .as_deref(),
        Some("ca-central-1")
    );
    let default_region = offline(
        "region-default-variable",
        &[("AWS_PROFILE", "desk"), ("AWS_DEFAULT_REGION", "us-west-1")],
    )
    .with_config_text(REGIONS);
    assert_eq!(default_region.region().as_deref(), Some("us-west-1"));
    let region = offline(
        "region-variable",
        &[
            ("AWS_DEFAULT_REGION", "us-west-1"),
            ("AWS_REGION", "eu-north-1"),
        ],
    )
    .with_config_text(REGIONS);
    assert_eq!(region.region().as_deref(), Some("eu-north-1"));
    assert_eq!(
        region.with_region("ap-south-1").region().as_deref(),
        Some("ap-south-1")
    );
}

const ENDPOINTS: &str = "\
[default]
endpoint_url = http://profile-wide:1000/
services = local

[services local]
s3 =
  endpoint_url = http://services-s3:2000/
";

#[test]
fn a_service_endpoint_is_stated_then_its_variable_then_aws_endpoint_url_then_services_then_the_profile_s()
 {
    assert_eq!(offline("endpoint-none", &[]).endpoint_url("s3"), None);

    let profile_only = offline("endpoint-profile", &[])
        .with_config_text("[default]\nendpoint_url = http://profile-wide:1000/\n");
    assert_eq!(
        profile_only.endpoint_url("s3").as_deref(),
        Some("http://profile-wide:1000"),
        "a trailing slash is dropped"
    );

    let services = offline("endpoint-services", &[]).with_config_text(ENDPOINTS);
    assert_eq!(
        services.endpoint_url("s3").as_deref(),
        Some("http://services-s3:2000")
    );
    assert_eq!(
        services.endpoint_url("sts").as_deref(),
        Some("http://profile-wide:1000"),
        "a service the [services] section does not name falls to the profile's"
    );

    let global = offline(
        "endpoint-global",
        &[("AWS_ENDPOINT_URL", "http://env-wide:3000")],
    )
    .with_config_text(ENDPOINTS);
    assert_eq!(
        global.endpoint_url("s3").as_deref(),
        Some("http://env-wide:3000")
    );

    let pairs = [
        ("AWS_ENDPOINT_URL", "http://env-wide:3000"),
        ("AWS_ENDPOINT_URL_S3", "http://env-s3:4000/"),
        ("AWS_ENDPOINT_URL_SSO_OIDC", "http://env-oidc:6000"),
    ];
    let specific = offline("endpoint-specific", &pairs).with_config_text(ENDPOINTS);
    assert_eq!(
        specific.endpoint_url("s3").as_deref(),
        Some("http://env-s3:4000")
    );
    assert_eq!(
        specific.endpoint_url("S3").as_deref(),
        Some("http://env-s3:4000")
    );
    assert_eq!(
        specific.endpoint_url("sts").as_deref(),
        Some("http://env-wide:3000")
    );
    assert_eq!(
        specific.endpoint_url("sso-oidc").as_deref(),
        Some("http://env-oidc:6000"),
        "a hyphen in the service id is an underscore in the variable"
    );

    let mut ignoring = pairs.to_vec();
    ignoring.push(("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "true"));
    let ignored = offline("endpoint-ignored", &ignoring).with_config_text(ENDPOINTS);
    assert_eq!(ignored.endpoint_url("s3"), None);
    assert_eq!(ignored.endpoint_url("sts"), None);
    let stated = ignored.with_endpoint_url("http://explicit:5000/");
    assert_eq!(
        stated.endpoint_url("s3").as_deref(),
        Some("http://explicit:5000"),
        "what the caller states is never ignored"
    );
    assert_eq!(
        stated.endpoint_url("sts").as_deref(),
        Some("http://explicit:5000")
    );

    let ignored_by_profile = offline("endpoint-ignored-profile", &[]).with_config_text(
        "[default]\nendpoint_url = http://profile-wide:1000\nignore_configured_endpoint_urls = true\n",
    );
    assert_eq!(ignored_by_profile.endpoint_url("s3"), None);
}

#[test]
fn the_sts_endpoint_is_regional_by_default_and_global_only_for_the_legacy_regions_in_legacy_mode() {
    let regional = offline("sts-regional", &[]);
    assert!(regional.sts_regional_endpoints());
    assert_eq!(
        regional.sts_endpoint("eu-west-3"),
        "https://sts.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        regional.sts_endpoint("us-east-1"),
        "https://sts.us-east-1.amazonaws.com"
    );

    let legacy = regional.with_sts_regional_endpoints(false);
    assert!(!legacy.sts_regional_endpoints());
    assert_eq!(
        legacy.sts_endpoint("us-east-1"),
        "https://sts.amazonaws.com"
    );
    assert_eq!(
        legacy.sts_endpoint("eu-west-3"),
        "https://sts.amazonaws.com"
    );
    assert_eq!(
        legacy.sts_endpoint("af-south-1"),
        "https://sts.af-south-1.amazonaws.com",
        "a region outside the legacy list is regional in both modes"
    );
    assert_eq!(
        legacy.sts_endpoint("cn-north-1"),
        "https://sts.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        legacy
            .with_use_fips_endpoint(true)
            .sts_endpoint("us-east-1"),
        "https://sts-fips.us-east-1.amazonaws.com",
        "FIPS is regional even in legacy mode"
    );

    assert_eq!(
        regional
            .with_use_fips_endpoint(true)
            .sts_endpoint("us-west-2"),
        "https://sts-fips.us-west-2.amazonaws.com"
    );
    assert_eq!(
        regional
            .with_use_dualstack_endpoint(true)
            .sts_endpoint("eu-west-3"),
        "https://sts.eu-west-3.api.aws"
    );
    assert_eq!(
        regional
            .with_use_fips_endpoint(true)
            .with_use_dualstack_endpoint(true)
            .sts_endpoint("us-east-1"),
        "https://sts-fips.us-east-1.api.aws"
    );
    assert_eq!(
        regional.sts_endpoint("cn-northwest-1"),
        "https://sts.cn-northwest-1.amazonaws.com.cn"
    );

    assert_eq!(
        regional
            .with_endpoint_url("http://localhost:4566/")
            .sts_endpoint("eu-west-3"),
        "http://localhost:4566",
        "a configured endpoint replaces the published host"
    );
    assert_eq!(
        offline(
            "sts-variable",
            &[("AWS_ENDPOINT_URL_STS", "http://sts.local:8000")]
        )
        .sts_endpoint("eu-west-3"),
        "http://sts.local:8000"
    );
}

#[test]
fn the_sts_endpoint_mode_is_read_from_the_variable_then_the_profile() {
    assert!(
        !offline(
            "sts-mode-variable",
            &[("AWS_STS_REGIONAL_ENDPOINTS", "legacy")]
        )
        .sts_regional_endpoints()
    );
    assert!(
        !offline("sts-mode-case", &[("AWS_STS_REGIONAL_ENDPOINTS", "LEGACY")])
            .sts_regional_endpoints()
    );
    assert!(
        offline(
            "sts-mode-regional",
            &[("AWS_STS_REGIONAL_ENDPOINTS", "regional")]
        )
        .sts_regional_endpoints()
    );
    let profile = offline("sts-mode-profile", &[])
        .with_config_text("[default]\nsts_regional_endpoints = legacy\n");
    assert!(!profile.sts_regional_endpoints());
    assert_eq!(
        profile.sts_endpoint("us-east-1"),
        "https://sts.amazonaws.com"
    );
    assert!(
        offline(
            "sts-mode-over-profile",
            &[("AWS_STS_REGIONAL_ENDPOINTS", "regional")]
        )
        .with_config_text("[default]\nsts_regional_endpoints = legacy\n")
        .sts_regional_endpoints(),
        "the variable beats the profile"
    );
    assert!(
        offline(
            "sts-mode-stated",
            &[("AWS_STS_REGIONAL_ENDPOINTS", "legacy")]
        )
        .with_sts_regional_endpoints(true)
        .sts_regional_endpoints(),
        "what the caller states beats both"
    );
}

#[test]
fn fips_and_dual_stack_are_stated_then_read_from_the_variable_then_the_profile() {
    assert!(!offline("fips-none", &[]).use_fips_endpoint());
    assert!(offline("fips-variable", &[("AWS_USE_FIPS_ENDPOINT", "true")]).use_fips_endpoint());
    assert!(
        offline("fips-profile", &[])
            .with_config_text("[default]\nuse_fips_endpoint = true\n")
            .use_fips_endpoint()
    );
    assert!(
        !offline("fips-variable-false", &[("AWS_USE_FIPS_ENDPOINT", "false")])
            .with_config_text("[default]\nuse_fips_endpoint = true\n")
            .use_fips_endpoint(),
        "the variable beats the profile, false included"
    );
    assert!(
        !offline("fips-stated", &[("AWS_USE_FIPS_ENDPOINT", "true")])
            .with_use_fips_endpoint(false)
            .use_fips_endpoint()
    );

    assert!(!offline("dualstack-none", &[]).use_dualstack_endpoint());
    assert!(
        offline(
            "dualstack-variable",
            &[("AWS_USE_DUALSTACK_ENDPOINT", "TRUE")]
        )
        .use_dualstack_endpoint()
    );
    let profile = offline("dualstack-profile", &[])
        .with_config_text("[default]\nuse_dualstack_endpoint = true\n");
    assert!(profile.use_dualstack_endpoint());
    assert_eq!(
        profile.sts_endpoint("eu-west-3"),
        "https://sts.eu-west-3.api.aws"
    );
    assert!(
        !profile
            .with_use_dualstack_endpoint(false)
            .use_dualstack_endpoint()
    );
}

#[test]
fn max_attempts_is_read_from_the_variable_then_the_profile_and_a_count_that_is_not_one_is_none() {
    assert_eq!(offline("attempts-none", &[]).max_attempts(), None);
    assert_eq!(
        offline("attempts-variable", &[("AWS_MAX_ATTEMPTS", "5")]).max_attempts(),
        Some(5)
    );
    assert_eq!(
        offline("attempts-profile", &[])
            .with_config_text("[default]\nmax_attempts = 7\n")
            .max_attempts(),
        Some(7)
    );
    assert_eq!(
        offline("attempts-both", &[("AWS_MAX_ATTEMPTS", "5")])
            .with_config_text("[default]\nmax_attempts = 7\n")
            .max_attempts(),
        Some(5)
    );
    assert_eq!(
        offline("attempts-zero", &[("AWS_MAX_ATTEMPTS", "0")]).max_attempts(),
        None
    );
    assert_eq!(
        offline("attempts-word", &[("AWS_MAX_ATTEMPTS", "many")]).max_attempts(),
        None
    );
}

#[test]
fn the_ca_bundle_is_stated_then_aws_ca_bundle_then_the_profile_s() {
    assert_eq!(offline("ca-none", &[]).ca_bundle(), None);
    let profile =
        offline("ca-profile", &[]).with_config_text("[default]\nca_bundle = /srv/ca/profile.pem\n");
    assert_eq!(
        profile.ca_bundle(),
        Some(PathBuf::from("/srv/ca/profile.pem"))
    );
    let variable = offline("ca-variable", &[("AWS_CA_BUNDLE", "/opt/ca/env.pem")])
        .with_config_text("[default]\nca_bundle = /srv/ca/profile.pem\n");
    assert_eq!(variable.ca_bundle(), Some(PathBuf::from("/opt/ca/env.pem")));
    assert_eq!(
        variable.with_ca_bundle("/etc/ssl/explicit.pem").ca_bundle(),
        Some(PathBuf::from("/etc/ssl/explicit.pem"))
    );
}

#[test]
fn the_shared_files_are_stated_then_named_by_the_environment_then_under_the_directory() {
    let directory = scratch("file-paths");
    let under = Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(&directory);
    assert_eq!(under.directory(), Some(directory.clone()));
    assert_eq!(under.config_file(), Some(directory.join("config")));
    assert_eq!(
        under.credentials_file(),
        Some(directory.join("credentials"))
    );

    let named = Session::new()
        .with_variables([
            ("AWS_CONFIG_FILE", "/srv/aws/config"),
            ("AWS_SHARED_CREDENTIALS_FILE", "/srv/aws/credentials"),
        ])
        .with_directory(&directory);
    assert_eq!(named.config_file(), Some(PathBuf::from("/srv/aws/config")));
    assert_eq!(
        named.credentials_file(),
        Some(PathBuf::from("/srv/aws/credentials"))
    );

    let stated = named
        .with_config_file("/etc/aws/config")
        .with_credentials_file("/etc/aws/credentials");
    assert_eq!(stated.config_file(), Some(PathBuf::from("/etc/aws/config")));
    assert_eq!(
        stated.credentials_file(),
        Some(PathBuf::from("/etc/aws/credentials"))
    );
}

#[test]
fn a_shared_file_holding_a_byte_that_is_not_utf8_still_reads_every_profile() {
    let directory = scratch("files-stray-byte");
    std::fs::write(
        directory.join("config"),
        b"# caf\xe9\n[default]\nregion = eu-west-3\n",
    )
    .expect("write the config");
    let session = Session::new()
        .with_variables::<&str, &str>([])
        .with_directory(&directory)
        .with_metadata_disabled(true);
    assert_eq!(
        session.region().as_deref(),
        Some("eu-west-3"),
        "the stray byte is replaced and the profile read"
    );
    assert_eq!(session.available_profiles(), vec!["default".to_owned()]);
}

#[test]
fn the_files_under_the_directory_or_where_aws_config_file_points_are_read_from_disk() {
    let directory = scratch("files-on-disk");
    write(
        &directory.join("config"),
        "[profile disk]\nregion = eu-central-1\n",
    );
    write(
        &directory.join("credentials"),
        "[disk]\naws_access_key_id = AKIADISK\naws_secret_access_key = disk-secret\n",
    );
    let session = Session::new()
        .with_variables([("AWS_PROFILE", "disk")])
        .with_directory(&directory)
        .with_metadata_disabled(true);
    assert_eq!(session.region().as_deref(), Some("eu-central-1"));
    assert_eq!(session.available_profiles(), vec!["disk".to_owned()]);
    assert_eq!(found(&session).access_key_id(), "AKIADISK");
    assert_eq!(session.credential_source(), Some("shared credentials file"));

    let elsewhere = directory.join("elsewhere").join("config");
    write(&elsewhere, "[profile disk]\nregion = ap-northeast-1\n");
    let elsewhere_path = elsewhere.display().to_string();
    let pointed = Session::new()
        .with_variables([
            ("AWS_PROFILE", "disk"),
            ("AWS_CONFIG_FILE", elsewhere_path.as_str()),
        ])
        .with_directory(&directory)
        .with_metadata_disabled(true);
    assert_eq!(pointed.region().as_deref(), Some("ap-northeast-1"));

    assert_eq!(
        pointed
            .with_config_text("[profile disk]\nregion = us-east-2\n")
            .region()
            .as_deref(),
        Some("us-east-2"),
        "text stated on the session is read in place of the file"
    );
}

#[test]
fn available_profiles_names_every_profile_either_file_holds_and_no_other_section() {
    let session = offline("available", &[])
        .with_config_text(
            "[default]\nregion = eu-west-3\n\n[profile trading]\nregion = eu-west-1\n\n\
             [sso-session corp]\nsso_start_url = https://corp.awsapps.com/start\n\n\
             [services local]\ns3 =\n  endpoint_url = http://localhost:9000\n\n\
             [plugins]\ncli_legacy_plugin_path = /opt/plugins\n\n[stray]\nregion = us-east-1\n",
        )
        .with_credentials_text(
            "[ci]\naws_access_key_id = AKIACI\naws_secret_access_key = ci-secret\n\n\
             [trading]\naws_access_key_id = AKIATRADING\naws_secret_access_key = trading-secret\n",
        );

    assert_eq!(
        session.available_profiles(),
        vec!["ci".to_owned(), "default".to_owned(), "trading".to_owned()],
        "in name order, once each; a config section without `profile` is not one"
    );
}

#[test]
fn debug_output_never_renders_a_secret() {
    let session = Session::new()
        .with_variables([("AWS_SECRET_ACCESS_KEY", "variable-secret")])
        .with_directory(scratch("debug"))
        .with_credentials(
            Credentials::new("AKIADEBUG", "debug-secret").with_session_token("debug-token"),
        )
        .with_mfa_prompt(Arc::new(|_: &str| -> Option<String> { None }));

    let rendered = format!("{session:?}");
    assert!(
        rendered.contains("AKIADEBUG"),
        "the key id is not a secret: {rendered}"
    );
    assert!(!rendered.contains("debug-secret"), "{rendered}");
    assert!(!rendered.contains("debug-token"), "{rendered}");
    assert!(!rendered.contains("variable-secret"), "{rendered}");
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::aws::Session;
    use yggdryl::internals::aws_session::imds_endpoint;

    use crate::mod_::scratch;

    /// A session reading `pairs` as its whole environment and `config` as its
    /// configuration file, the metadata service left enabled.
    fn reaching(pairs: &[(&str, &str)], config: &str) -> Session {
        Session::new()
            .with_variables(pairs.iter().copied())
            .with_directory(scratch("imds-family"))
            .with_config_text(config)
    }

    #[test]
    fn the_endpoint_mode_then_imds_use_ipv6_pick_the_metadata_address_family() {
        let endpoint =
            |pairs: &[(&str, &str)], config: &str| imds_endpoint(&reaching(pairs, config));
        assert_eq!(endpoint(&[], "").as_deref(), Some("http://169.254.169.254"));
        assert_eq!(
            endpoint(&[("AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE", "IPv6")], "").as_deref(),
            Some("http://[fd00:ec2::254]")
        );
        assert_eq!(
            endpoint(&[("AWS_IMDS_USE_IPV6", "true")], "").as_deref(),
            Some("http://[fd00:ec2::254]"),
            "the older switch, read where no mode is stated"
        );
        assert_eq!(
            endpoint(&[], "[default]\nimds_use_ipv6 = true\n").as_deref(),
            Some("http://[fd00:ec2::254]"),
            "and read from the profile"
        );
        assert_eq!(
            endpoint(
                &[
                    ("AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE", "ipv4"),
                    ("AWS_IMDS_USE_IPV6", "true"),
                ],
                ""
            )
            .as_deref(),
            Some("http://169.254.169.254"),
            "a stated mode wins over the switch"
        );
        assert_eq!(
            endpoint(
                &[
                    (
                        "AWS_EC2_METADATA_SERVICE_ENDPOINT",
                        "http://localhost:9999/"
                    ),
                    ("AWS_IMDS_USE_IPV6", "true"),
                ],
                ""
            )
            .as_deref(),
            Some("http://localhost:9999"),
            "a stated endpoint wins over both"
        );
        assert_eq!(endpoint(&[("AWS_EC2_METADATA_DISABLED", "true")], ""), None);
        assert_eq!(
            imds_endpoint(&reaching(&[], "").with_environment(false)),
            None,
            "a session consulting no environment reaches no metadata service"
        );
    }
}
