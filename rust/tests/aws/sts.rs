//! `rust/src/aws/sts.rs`: trading a role for a session through AWS STS - the
//! role a caller names, the exchange a session sends, and the cache the AWS
//! CLI shares.
//!
//! What a caller can observe - the `AssumedRole` builder, `CredentialSource`,
//! and the exchanges a sealed `Session` sends to the identity fake - sits at
//! the top level. The reading of STS's documents, the CLI cache key and the
//! cache file itself, which no caller can name, are pinned in `internal`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::{AssumedRole, CredentialSource, Credentials, Session};

use crate::identity::{Identity, Recorded, iso8601};
use crate::mod_::{scratch, sealed};

/// The role every exchange here asks for; the fake answers it as
/// `ASIAlake-reader`, in account `123456789012`.
const ROLE: &str = "arn:aws:iam::123456789012:role/lake-reader";
/// The MFA device a role may name.
const MFA: &str = "arn:aws:iam::123456789012:mfa/desk";
/// The keys a session's environment hands it, which sign its exchanges.
const BASE_KEYS: [(&str, &str); 2] = [
    ("AWS_ACCESS_KEY_ID", "AKIABASE"),
    ("AWS_SECRET_ACCESS_KEY", "base-secret"),
];
/// The CLI cache file of `AssumedRole::new(ROLE)`: the SHA-1 of
/// `{"RoleArn": "arn:aws:iam::123456789012:role/lake-reader"}`, which is
/// Python's `json.dumps(args, sort_keys=True)` of botocore's arguments.
const ROLE_CACHE_KEY: &str = "ad68aaf73f590171fea5d792713e17b7f72ca6c0";
/// The same with `.with_session_name("power-desk")`: the SHA-1 of
/// `{"RoleArn": "arn:aws:iam::123456789012:role/lake-reader", "RoleSessionName": "power-desk"}`.
const NAMED_ROLE_CACHE_KEY: &str = "0cd42f0bb8246587d7a88254c15ea9fee51411fb";
/// A web identity token, as a platform writes one.
const WEB_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJwb2QtNyJ9.c2lnbmF0dXJl";
/// The token the platform rotated the file to.
const ROTATED_TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJwb2QtOCJ9.cm90YXRlZA";

/// What a session asks for an MFA code; `yggdryl::aws` names the type
/// nowhere a caller can reach, so the suite spells it.
type Prompt = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The one request the fake handled, which must be the STS exchange
/// `action`, sent as the query API sends one.
fn only_exchange(identity: &Identity, action: &str) -> Recorded {
    let requests = identity.requests();
    let actions: Vec<Option<&str>> = requests
        .iter()
        .map(|request| request.query("Action"))
        .collect();
    assert_eq!(
        requests.len(),
        1,
        "one round trip, the exchange; the actions sent were {actions:?}"
    );
    let exchange = requests.into_iter().next().expect("the exchange");
    assert!(
        exchange.is_sts(action),
        "expected {action}, got {:?}",
        exchange.query
    );
    assert_eq!(exchange.method, "GET", "the exchange is one query-API GET");
    assert_eq!(exchange.path, "/", "the query API answers at the root");
    exchange
}

/// The directory `session` shares the AWS CLI's role cache under.
fn cli_cache(session: &Session) -> PathBuf {
    session
        .directory()
        .expect("a sealed session names its scratch directory")
        .join("cli")
        .join("cache")
}

/// `at` without its fraction, which is all a cached expiration spells.
fn whole_seconds(at: SystemTime) -> SystemTime {
    UNIX_EPOCH
        + Duration::from_secs(
            at.duration_since(UNIX_EPOCH)
                .expect("an instant after the epoch")
                .as_secs(),
        )
}

/// `at` as the AWS CLI's cache spells an expiration: `YYYY-MM-DDThh:mm:ssUTC`.
fn cli_expiration(at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .expect("an instant after the epoch")
        .as_secs();
    iso8601(i64::try_from(seconds).expect("a representable instant")).replace('Z', "UTC")
}

/// File a session under `key` in `directory` the way the AWS CLI does: the
/// whole `AssumeRole` answer, of which only `Credentials` is the cache's.
fn file_cli_session(directory: &Path, key: &str, access_key: &str, expiration: &str) {
    std::fs::create_dir_all(directory).expect("the CLI cache directory");
    let document = serde_json::json!({
        "Credentials": {
            "AccessKeyId": access_key,
            "SecretAccessKey": "cli-secret",
            "SessionToken": "cli-token",
            "Expiration": expiration,
        },
        "AssumedRoleUser": {
            "AssumedRoleId": "AROA:botocore-session-1",
            "Arn": "arn:aws:sts::123456789012:assumed-role/lake-reader/botocore-session-1",
        },
        "ResponseMetadata": {"RequestId": "cli", "HTTPStatusCode": 200},
    });
    std::fs::write(directory.join(format!("{key}.json")), document.to_string())
        .expect("the CLI cache file");
}

// --- the role a caller names ------------------------------------------------

#[test]
fn a_bare_role_asks_for_one_hour_and_states_nothing_else() {
    let role = AssumedRole::new(ROLE);
    assert_eq!(role.role_arn(), ROLE, "the role asked for");
    assert_eq!(
        role.session_name(),
        None,
        "no name was chosen, so one is generated per exchange"
    );
    assert_eq!(role.external_id(), None, "no external id was named");
    assert_eq!(
        role.duration(),
        Duration::from_secs(3600),
        "an unstated duration is STS's own default, one hour"
    );
    assert_eq!(
        role.region(),
        None,
        "STS is reached in the session's region"
    );
    assert_eq!(
        role.endpoint(),
        None,
        "STS is reached at its published host"
    );
    assert_eq!(role.mfa_serial(), None, "no MFA device was named");
    assert_eq!(role.source_profile(), None, "no source profile was named");
    assert_eq!(
        role.credential_source(),
        None,
        "no credential source was named"
    );
    assert_eq!(
        role.web_identity_token_file(),
        None,
        "no web identity token file was named"
    );
}

#[test]
fn every_knob_a_role_is_given_reads_back_trimmed() {
    let role = AssumedRole::new(ROLE)
        .with_session_name("  power-desk ")
        .with_external_id(" desk-42 ")
        .with_duration(Duration::from_secs(7200))
        .with_region("eu-central-1")
        .with_endpoint(" https://sts.eu-central-1.amazonaws.com/ ")
        .with_mfa_serial(format!(" {MFA} "))
        .with_source_profile(" base ")
        .with_credential_source(CredentialSource::EcsContainer)
        .with_web_identity_token_file("/var/run/secrets/eks.amazonaws.com/serviceaccount/token");
    assert_eq!(role.role_arn(), ROLE, "the role asked for");
    assert_eq!(role.session_name(), Some("power-desk"), "the name, trimmed");
    assert_eq!(
        role.external_id(),
        Some("desk-42"),
        "the external id, trimmed"
    );
    assert_eq!(
        role.duration(),
        Duration::from_secs(7200),
        "a duration inside STS's bounds is kept"
    );
    assert_eq!(role.region(), Some("eu-central-1"), "the STS region");
    assert_eq!(
        role.endpoint(),
        Some("https://sts.eu-central-1.amazonaws.com"),
        "the endpoint, trimmed of blanks and its trailing slash"
    );
    assert_eq!(role.mfa_serial(), Some(MFA), "the MFA serial, trimmed");
    assert_eq!(
        role.source_profile(),
        Some("base"),
        "the source profile, trimmed"
    );
    assert_eq!(
        role.credential_source(),
        Some(CredentialSource::EcsContainer),
        "the credential source"
    );
    assert_eq!(
        role.web_identity_token_file(),
        Some(Path::new(
            "/var/run/secrets/eks.amazonaws.com/serviceaccount/token"
        )),
        "the token file"
    );
}

#[test]
fn a_duration_is_clamped_to_what_sts_will_issue() {
    for (asked, issued) in [
        (0, 900),
        (60, 900),
        (900, 900),
        (3600, 3600),
        (43_200, 43_200),
        (86_400, 43_200),
    ] {
        assert_eq!(
            AssumedRole::new(ROLE)
                .with_duration(Duration::from_secs(asked))
                .duration(),
            Duration::from_secs(issued),
            "{asked} s asked: STS issues between 15 minutes and 12 hours"
        );
    }
}

#[test]
fn an_empty_or_blank_knob_is_no_knob() {
    let role = AssumedRole::new(ROLE)
        .with_session_name("   ")
        .with_external_id("")
        .with_endpoint(" / ")
        .with_mfa_serial("\t")
        .with_source_profile("");
    assert_eq!(role.session_name(), None, "a blank name is no name");
    assert_eq!(role.external_id(), None, "an empty external id is none");
    assert_eq!(role.endpoint(), None, "a slash names no host");
    assert_eq!(role.mfa_serial(), None, "a blank serial names no device");
    assert_eq!(role.source_profile(), None, "an empty profile name is none");

    let cleared = AssumedRole::new(ROLE)
        .with_session_name("power-desk")
        .with_session_name("");
    assert_eq!(
        cleared.session_name(),
        None,
        "a blank name clears the one an earlier call chose"
    );
}

#[test]
fn an_empty_sts_region_is_no_region_rather_than_a_hostless_endpoint() {
    // `sts..amazonaws.com` is what an empty region would address.
    assert_eq!(
        AssumedRole::new(ROLE).with_region("").region(),
        None,
        "an empty region is the session's region"
    );
    assert_eq!(
        AssumedRole::new(ROLE).with_region("  ").region(),
        None,
        "a blank region is the session's region"
    );
}

#[test]
fn a_credential_source_reads_in_any_case_and_writes_as_the_config_file_spells_it() {
    for (spelled, source) in [
        ("Environment", CredentialSource::Environment),
        ("environment", CredentialSource::Environment),
        ("ENVIRONMENT", CredentialSource::Environment),
        (
            " Ec2InstanceMetadata ",
            CredentialSource::Ec2InstanceMetadata,
        ),
        ("ec2instancemetadata", CredentialSource::Ec2InstanceMetadata),
        ("EcsContainer", CredentialSource::EcsContainer),
        ("ECSCONTAINER", CredentialSource::EcsContainer),
    ] {
        let parsed: CredentialSource = spelled.parse().expect("a spelling the config file uses");
        assert_eq!(parsed, source, "{spelled:?}");
    }
    for (source, spelled) in [
        (CredentialSource::Environment, "Environment"),
        (CredentialSource::Ec2InstanceMetadata, "Ec2InstanceMetadata"),
        (CredentialSource::EcsContainer, "EcsContainer"),
    ] {
        assert_eq!(source.as_str(), spelled, "the config file's own spelling");
        assert_eq!(source.to_string(), spelled, "Display is that spelling");
        assert_eq!(
            spelled
                .parse::<CredentialSource>()
                .expect("the spelling it writes"),
            source,
            "what is written reads back"
        );
    }
}

#[test]
fn a_credential_source_the_config_file_does_not_define_is_refused_naming_the_three_it_does() {
    for spelled in ["", "Ec2Instance", "SourceProfile", "ecs-container"] {
        let message = spelled
            .parse::<CredentialSource>()
            .expect_err("not a credential source")
            .to_string();
        assert!(
            message.contains("Environment, Ec2InstanceMetadata or EcsContainer"),
            "the refusal names what is accepted: {message}"
        );
        assert!(
            message.contains(&format!("{spelled:?}")),
            "the refusal names what was given: {message}"
        );
    }
}

// --- the exchange, over the identity fake ---------------------------------------

#[test]
fn a_role_named_on_the_session_is_traded_once_with_the_explicit_keys_beneath_it() {
    let identity = Identity::start();
    let session = sealed(&identity, "sts-explicit-base")
        .with_credentials(
            Credentials::new("AKIAEXPLICIT", "explicit-secret")
                .with_session_token("explicit-token"),
        )
        .with_assumed_role(
            AssumedRole::new(ROLE)
                .with_session_name("power-desk")
                .with_external_id("desk-42"),
        );
    let now = SystemTime::now();

    let traded = session
        .credentials(now)
        .expect("the exchange answers")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session signs, never the explicit keys beneath it"
    );
    assert_eq!(
        traded.session_token(),
        Some("token-lake-reader-power-desk"),
        "the session token STS handed back"
    );
    assert_eq!(
        traded.account_id(),
        Some("123456789012"),
        "the account is read off the assumed role's ARN"
    );
    assert!(
        traded.expires_at().is_some_and(|expiry| expiry > now),
        "a session lapses, and this one has not yet"
    );
    assert_eq!(session.credential_source(), Some("assumed role"));
    let again = session
        .credentials(now + Duration::from_secs(60))
        .expect("the set in hand")
        .expect("a credential set");
    assert_eq!(
        again, traded,
        "the set in hand answers until it nears its expiry"
    );

    let exchange = only_exchange(&identity, "AssumeRole");
    assert_eq!(
        exchange.query("Version"),
        Some("2011-06-15"),
        "the STS API version"
    );
    assert_eq!(exchange.query("RoleArn"), Some(ROLE), "the role asked for");
    assert_eq!(
        exchange.query("RoleSessionName"),
        Some("power-desk"),
        "the name chosen"
    );
    assert_eq!(
        exchange.query("ExternalId"),
        Some("desk-42"),
        "the external id the trust policy needs"
    );
    assert_eq!(
        exchange.query("DurationSeconds"),
        Some("3600"),
        "the default hour, stated"
    );
    assert_eq!(
        exchange.query("SerialNumber"),
        None,
        "no MFA device was named"
    );
    assert_eq!(exchange.query("TokenCode"), None, "so no code is presented");
    let authorization = exchange.header("authorization").unwrap_or_default();
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAEXPLICIT/"),
        "the explicit keys sign the exchange: {authorization}"
    );
    assert!(
        authorization.contains("/eu-west-3/sts/aws4_request"),
        "signed for STS in the session's region, a signing key of its own: {authorization}"
    );
    assert_eq!(
        exchange.header("x-amz-security-token"),
        Some("explicit-token"),
        "the base set's session token rides the exchange"
    );
}

#[test]
fn a_traded_session_is_filed_where_the_aws_cli_reads_it_and_a_fresh_session_reuses_it() {
    let identity = Identity::start();
    let session = sealed(&identity, "sts-files-session")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE).with_session_name("power-desk"));

    let traded = session
        .credentials(SystemTime::now())
        .expect("the exchange answers")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session"
    );
    let exchange = only_exchange(&identity, "AssumeRole");
    let authorization = exchange.header("authorization").unwrap_or_default();
    assert!(
        authorization.contains("Credential=AKIABASE/"),
        "the environment's keys sign the exchange: {authorization}"
    );
    assert!(
        authorization.contains("/sts/aws4_request"),
        "signed for STS: {authorization}"
    );
    assert_eq!(
        exchange.header("x-amz-security-token"),
        None,
        "the base keys carry no token, so none is sent"
    );

    // The file the AWS CLI would look for, named as botocore names it.
    let file = cli_cache(&session).join(format!("{NAMED_ROLE_CACHE_KEY}.json"));
    let text = std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("the CLI cache file {}: {error}", file.display()));
    let document: serde_json::Value = serde_json::from_str(&text).expect("the cache file is JSON");
    let held = &document["Credentials"];
    assert_eq!(held["AccessKeyId"], "ASIAlake-reader", "{text}");
    assert_eq!(held["SecretAccessKey"], "secret-of-lake-reader", "{text}");
    assert_eq!(
        held["SessionToken"], "token-lake-reader-power-desk",
        "{text}"
    );
    let expiration = held["Expiration"].as_str().unwrap_or_default();
    assert!(
        expiration.ends_with("UTC") && expiration.len() == "2027-01-15T09:00:00UTC".len(),
        "the expiration as the CLI spells it: {text}"
    );

    // A session over the same directory that holds nothing yet reads it
    // back rather than asking STS again.
    let fresh = session.with_region("eu-west-3");
    let reused = fresh
        .credentials(SystemTime::now())
        .expect("the cache answers")
        .expect("a credential set");
    assert_eq!(
        reused.access_key_id(),
        "ASIAlake-reader",
        "the cached session"
    );
    assert_eq!(
        reused.session_token(),
        Some("token-lake-reader-power-desk"),
        "the cached token"
    );
    assert_eq!(
        identity.request_count(),
        1,
        "the second session costs no exchange"
    );
}

#[test]
fn a_session_the_aws_cli_cached_is_reused_without_an_exchange() {
    let identity = Identity::start();
    let session = sealed(&identity, "sts-reads-cli")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE));
    let expiry = whole_seconds(SystemTime::now() + Duration::from_secs(2 * 3600));
    file_cli_session(
        &cli_cache(&session),
        ROLE_CACHE_KEY,
        "ASIAFROMCLI",
        &cli_expiration(expiry),
    );

    let reused = session
        .credentials(SystemTime::now())
        .expect("the cache answers")
        .expect("a credential set");
    assert_eq!(reused.access_key_id(), "ASIAFROMCLI", "the CLI's session");
    assert_eq!(reused.session_token(), Some("cli-token"), "the CLI's token");
    assert_eq!(reused.expires_at(), Some(expiry), "the CLI's expiration");
    assert_eq!(session.credential_source(), Some("assumed role"));
    assert_eq!(
        identity.request_count(),
        0,
        "a session the CLI obtained is not asked for again"
    );
}

#[test]
fn a_cached_session_lapsing_within_fifteen_minutes_is_traded_again_and_filed_over() {
    let identity = Identity::start();
    let session = sealed(&identity, "sts-stale-cli")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE));
    let directory = cli_cache(&session);
    file_cli_session(
        &directory,
        ROLE_CACHE_KEY,
        "ASIASTALE",
        &cli_expiration(SystemTime::now() + Duration::from_secs(5 * 60)),
    );

    let traded = session
        .credentials(SystemTime::now())
        .expect("the exchange answers")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "a session about to lapse is replaced, not signed with"
    );
    only_exchange(&identity, "AssumeRole");
    let text = std::fs::read_to_string(directory.join(format!("{ROLE_CACHE_KEY}.json")))
        .expect("the CLI cache file");
    assert!(
        text.contains("ASIAlake-reader") && !text.contains("ASIASTALE"),
        "the fresh session is filed over the stale one: {text}"
    );
}

#[test]
fn a_role_sts_refuses_is_a_refusal_naming_the_code_held_for_the_pause_and_asked_again_after_it() {
    let identity = Identity::start();
    identity.refuse_sts("AccessDenied", 1);
    let session = sealed(&identity, "sts-refused")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE));
    let now = SystemTime::now();

    let message = session
        .credentials(now)
        .expect_err("a role STS refuses is a refusal, never the keys beneath it")
        .to_string();
    for fact in ["assumed role", "AssumeRole", "403", "AccessDenied", ROLE] {
        assert!(
            message.contains(fact),
            "the refusal names {fact}: {message}"
        );
    }
    assert_eq!(session.credential_source(), None, "nothing answered");
    assert_eq!(identity.request_count(), 1, "one exchange, refused");

    let again = session
        .credentials(now + Duration::from_secs(10))
        .expect_err("the failure is held")
        .to_string();
    assert!(again.contains("AccessDenied"), "the same refusal: {again}");
    assert_eq!(
        identity.request_count(),
        1,
        "a failed walk is answered again for the pause rather than walked again"
    );

    let traded = session
        .credentials(now + Duration::from_secs(31))
        .expect("STS answers now")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session"
    );
    assert_eq!(
        identity.request_count(),
        2,
        "after the pause the chain is walked again, once"
    );
}

#[test]
fn a_role_behind_an_mfa_device_asks_the_prompt_once_and_presents_its_code() {
    let identity = Identity::start();
    identity.require_token_code(Some("123456"));
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = Arc::clone(&asked);
    let prompt: Prompt = Arc::new(move |serial: &str| -> Option<String> {
        record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(serial.to_owned());
        Some("123456".to_owned())
    });
    let session = sealed(&identity, "sts-mfa")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE).with_mfa_serial(MFA))
        .with_mfa_prompt(prompt);

    let traded = session
        .credentials(SystemTime::now())
        .expect("the code is accepted")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session"
    );
    assert_eq!(
        *asked.lock().unwrap_or_else(PoisonError::into_inner),
        [MFA],
        "the prompt is asked once, for the device the role names"
    );
    let exchange = only_exchange(&identity, "AssumeRole");
    assert_eq!(exchange.query("SerialNumber"), Some(MFA), "the device");
    assert_eq!(
        exchange.query("TokenCode"),
        Some("123456"),
        "the code it showed"
    );

    // The session a code was entered for is filed in the CLI cache, so a
    // fresh session over the same directory neither prompts nor exchanges.
    let fresh = session.with_region("eu-west-3");
    let reused = fresh
        .credentials(SystemTime::now())
        .expect("the cache answers")
        .expect("a credential set");
    assert_eq!(
        reused.access_key_id(),
        "ASIAlake-reader",
        "the cached session"
    );
    assert_eq!(
        asked.lock().unwrap_or_else(PoisonError::into_inner).len(),
        1,
        "a cached session asks no code"
    );
    assert_eq!(identity.request_count(), 1, "and costs no exchange");
}

#[test]
fn a_role_behind_an_mfa_device_with_no_code_to_present_is_refused_before_any_exchange() {
    let identity = Identity::start();
    let role = AssumedRole::new(ROLE).with_mfa_serial(MFA);

    let unprompted = sealed(&identity, "sts-mfa-unprompted")
        .with_variables(BASE_KEYS)
        .with_assumed_role(role.clone());
    let message = unprompted
        .credentials(SystemTime::now())
        .expect_err("no prompt to ask a code of")
        .to_string();
    assert!(
        message.contains(MFA),
        "the refusal names the device: {message}"
    );
    assert!(
        message.contains("with_mfa_prompt"),
        "the refusal names the remedy: {message}"
    );

    let declined: Prompt = Arc::new(|_: &str| -> Option<String> { None });
    let declining = sealed(&identity, "sts-mfa-declined")
        .with_variables(BASE_KEYS)
        .with_assumed_role(role)
        .with_mfa_prompt(declined);
    let message = declining
        .credentials(SystemTime::now())
        .expect_err("the person declined")
        .to_string();
    assert!(
        message.contains("no code was given") && message.contains(MFA),
        "the refusal says no code was given for the device: {message}"
    );

    assert_eq!(
        identity.request_count(),
        0,
        "a code is asked for before the exchange, so neither session reached STS"
    );
}

#[test]
fn a_code_sts_does_not_accept_is_a_refusal_naming_what_sts_said() {
    let identity = Identity::start();
    identity.require_token_code(Some("123456"));
    let prompt: Prompt = Arc::new(|_: &str| -> Option<String> { Some("000000".to_owned()) });
    let session = sealed(&identity, "sts-mfa-wrong")
        .with_variables(BASE_KEYS)
        .with_assumed_role(AssumedRole::new(ROLE).with_mfa_serial(MFA))
        .with_mfa_prompt(prompt);

    let message = session
        .credentials(SystemTime::now())
        .expect_err("STS refuses the code")
        .to_string();
    assert!(
        message.contains("AccessDenied") && message.contains("MultiFactorAuthentication"),
        "STS's own code and message: {message}"
    );
    let exchange = only_exchange(&identity, "AssumeRole");
    assert_eq!(
        exchange.query("TokenCode"),
        Some("000000"),
        "the code presented"
    );
}

#[test]
fn a_web_identity_token_is_traded_unsigned_for_the_role_the_environment_names() {
    let identity = Identity::start();
    identity.require_web_identity_token(Some(WEB_TOKEN));
    let token_file = scratch("sts-web-identity-token").join("token");
    std::fs::write(&token_file, format!("{WEB_TOKEN}\n")).expect("the token file");
    let token_path = token_file.to_string_lossy().into_owned();
    let session = sealed(&identity, "sts-web-identity").with_variables([
        ("AWS_ROLE_ARN", ROLE),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", token_path.as_str()),
        ("AWS_ROLE_SESSION_NAME", "pod-7"),
    ]);
    let now = SystemTime::now();

    let traded = session
        .credentials(now)
        .expect("the token is traded")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session"
    );
    assert_eq!(
        traded.session_token(),
        Some("token-lake-reader-pod-7"),
        "the session token STS handed back"
    );
    assert_eq!(
        traded.account_id(),
        Some("123456789012"),
        "the account is read off the assumed role's ARN"
    );
    assert_eq!(session.credential_source(), Some("web identity"));

    let exchange = only_exchange(&identity, "AssumeRoleWithWebIdentity");
    assert_eq!(
        exchange.header("authorization"),
        None,
        "a web identity exchange is not signed: the token is the proof"
    );
    assert_eq!(
        exchange.header("x-amz-security-token"),
        None,
        "nor carries a token of keys"
    );
    assert_eq!(
        exchange.query("Version"),
        Some("2011-06-15"),
        "the STS API version"
    );
    assert_eq!(
        exchange.query("RoleArn"),
        Some(ROLE),
        "the role AWS_ROLE_ARN names"
    );
    assert_eq!(
        exchange.query("WebIdentityToken"),
        Some(WEB_TOKEN),
        "the file's content, its trailing newline trimmed"
    );
    assert_eq!(
        exchange.query("RoleSessionName"),
        Some("pod-7"),
        "the name AWS_ROLE_SESSION_NAME gives"
    );
    assert_eq!(
        exchange.query("DurationSeconds"),
        Some("3600"),
        "the default hour"
    );

    // The platform rotates the file, and the next exchange presents what the
    // file holds then.
    std::fs::write(&token_file, ROTATED_TOKEN).expect("the rotated token file");
    identity.require_web_identity_token(Some(ROTATED_TOKEN));
    identity.clear_requests();
    session.invalidate();
    session
        .credentials(now)
        .expect("the rotated token is traded")
        .expect("a credential set");
    let exchange = only_exchange(&identity, "AssumeRoleWithWebIdentity");
    assert_eq!(
        exchange.query("WebIdentityToken"),
        Some(ROTATED_TOKEN),
        "the file is read at every exchange"
    );
}

#[test]
fn a_role_named_with_a_web_identity_token_file_is_traded_unsigned_with_no_keys_beneath_it() {
    let identity = Identity::start();
    // An instance without a role: the chain has no keys to offer, and a web
    // identity exchange needs none.
    identity.set_imds_role(None);
    identity.require_web_identity_token(Some(WEB_TOKEN));
    let token_file = scratch("sts-explicit-web-identity-token").join("token");
    std::fs::write(&token_file, WEB_TOKEN).expect("the token file");
    let session = sealed(&identity, "sts-explicit-web-identity")
        .with_assumed_role(AssumedRole::new(ROLE).with_web_identity_token_file(token_file));
    let now = SystemTime::now();

    let traded = session
        .credentials(now)
        .expect("the token is traded")
        .expect("a credential set");
    assert_eq!(
        traded.access_key_id(),
        "ASIAlake-reader",
        "the role's session"
    );

    let exchange = only_exchange(&identity, "AssumeRoleWithWebIdentity");
    assert_eq!(
        exchange.header("authorization"),
        None,
        "the token file replaces signing"
    );
    assert_eq!(
        exchange.query("WebIdentityToken"),
        Some(WEB_TOKEN),
        "the file's token"
    );
    let generated = format!(
        "yggdryl-session-{}",
        now.duration_since(UNIX_EPOCH)
            .expect("an instant after the epoch")
            .as_secs()
    );
    assert_eq!(
        exchange.query("RoleSessionName"),
        Some(generated.as_str()),
        "a name generated from the exchange's instant when none was chosen"
    );
}

#[cfg(feature = "internals")]
mod internal {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use yggdryl::aws::{AssumedRole, CredentialSource, Credentials};
    use yggdryl::internals::aws_sts::{
        cache_key, parse, parse_error, read_cache, session_name_at, write_cache,
    };

    use super::{MFA, NAMED_ROLE_CACHE_KEY, ROLE, ROLE_CACHE_KEY, file_cli_session};
    use crate::mod_::scratch;

    /// The instant every reading here is made at: 2027-01-15T08:00:00Z.
    const NOW: u64 = 1_800_000_000;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// The `AssumeRole` answer of the STS API reference, as STS sends it.
    const ASSUME_ROLE_ANSWER: &str = r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <SourceIdentity>Alice</SourceIdentity>
    <AssumedRoleUser>
      <Arn>arn:aws:sts::123456789012:assumed-role/demo/TestAR</Arn>
      <AssumedRoleId>ARO123EXAMPLE123:TestAR</AssumedRoleId>
    </AssumedRoleUser>
    <Credentials>
      <AccessKeyId>ASIAIOSFODNN7EXAMPLE</AccessKeyId>
      <SecretAccessKey>wJalrXUtnFEMI/K7MDENG/bPxRfiCYzEXAMPLEKEY</SecretAccessKey>
      <SessionToken>AQoDYXdzEPT//////////wEXAMPLEtc764bNrC9SAPBSM22wDOk4x4HIZ8j4FZTwdQW</SessionToken>
      <Expiration>2019-11-09T13:34:41Z</Expiration>
    </Credentials>
    <PackedPolicySize>6</PackedPolicySize>
  </AssumeRoleResult>
  <ResponseMetadata>
    <RequestId>c6104cbe-af31-11e0-8154-cbc7ccf896c7</RequestId>
  </ResponseMetadata>
</AssumeRoleResponse>"#;

    /// The `AssumeRoleWithWebIdentity` answer of the STS API reference.
    const WEB_IDENTITY_ANSWER: &str = r#"<AssumeRoleWithWebIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleWithWebIdentityResult>
    <SubjectFromWebIdentityToken>amzn1.account.AF6RHO7KZU5XRVQJGXK6HB56KR2A</SubjectFromWebIdentityToken>
    <Audience>client.5498841531868486423.1548@apps.example.com</Audience>
    <AssumedRoleUser>
      <Arn>arn:aws:sts::123456789012:assumed-role/FederatedWebIdentityRole/app1</Arn>
      <AssumedRoleId>AROACLKWSDQRAOEXAMPLE:app1</AssumedRoleId>
    </AssumedRoleUser>
    <Credentials>
      <SessionToken>AQoDYXdzEE0a8ANXXXXXXXXNO1ewxE5TijQyp+IEXAMPLE</SessionToken>
      <SecretAccessKey>wJalrXUtnFEMI/K7MDENG/bPxRfiCYzEXAMPLEKEY</SecretAccessKey>
      <Expiration>2014-10-24T23:00:23Z</Expiration>
      <AccessKeyId>ASgeIAIOSFODNN7EXAMPLE</AccessKeyId>
    </Credentials>
    <SourceIdentity>SourceIdentityValue</SourceIdentity>
    <Provider>www.amazon.com</Provider>
  </AssumeRoleWithWebIdentityResult>
  <ResponseMetadata>
    <RequestId>ad4156e9-bce1-11e2-82e6-6b6efEXAMPLE</RequestId>
  </ResponseMetadata>
</AssumeRoleWithWebIdentityResponse>"#;

    /// A refusal as STS sends one.
    const ERROR_RESPONSE: &str = r#"<ErrorResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <Error>
    <Type>Sender</Type>
    <Code>AccessDenied</Code>
    <Message>User: arn:aws:iam::123456789012:user/desk is not authorized to perform: sts:AssumeRole on resource: arn:aws:iam::123456789012:role/lake-reader</Message>
  </Error>
  <RequestId>4a4c2b9e-8c2f-11e0-8154-cbc7ccf896c7</RequestId>
</ErrorResponse>"#;

    // --- STS's answers ------------------------------------------------------

    #[test]
    fn an_assume_role_answer_is_read_as_its_keys_token_expiry_and_account() {
        let read = parse(ASSUME_ROLE_ANSWER.as_bytes(), "AssumeRole", at(NOW))
            .expect("the answer carries a credential set");
        assert_eq!(
            read,
            Credentials::new(
                "ASIAIOSFODNN7EXAMPLE",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYzEXAMPLEKEY"
            )
            .with_session_token(
                "AQoDYXdzEPT//////////wEXAMPLEtc764bNrC9SAPBSM22wDOk4x4HIZ8j4FZTwdQW"
            )
            .with_expiry(at(1_573_306_481))
            .with_account_id("123456789012"),
            "keys, token, the stated expiry, and the account off the assumed role's ARN"
        );
    }

    #[test]
    fn a_web_identity_answer_is_read_whatever_order_its_credentials_come_in() {
        let read = parse(
            WEB_IDENTITY_ANSWER.as_bytes(),
            "AssumeRoleWithWebIdentity",
            at(NOW),
        )
        .expect("the answer carries a credential set");
        assert_eq!(
            read,
            Credentials::new(
                "ASgeIAIOSFODNN7EXAMPLE",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYzEXAMPLEKEY"
            )
            .with_session_token("AQoDYXdzEE0a8ANXXXXXXXXNO1ewxE5TijQyp+IEXAMPLE")
            .with_expiry(at(1_414_191_623))
            .with_account_id("123456789012"),
            "keys, token, the stated expiry, and the account off the assumed role's ARN"
        );
    }

    #[test]
    fn an_answer_stating_no_expiry_is_taken_to_last_one_hour_never_forever() {
        let now = at(NOW);
        let body = "<AssumeRoleResponse><AssumeRoleResult><Credentials>\
            <AccessKeyId>ASIANOEXPIRY</AccessKeyId>\
            <SecretAccessKey>secret</SecretAccessKey>\
            <SessionToken>token</SessionToken>\
            </Credentials></AssumeRoleResult></AssumeRoleResponse>";
        let read = parse(body.as_bytes(), "AssumeRole", now).expect("a credential set");
        assert_eq!(
            read.expires_at(),
            Some(now + Duration::from_secs(3600)),
            "a conservative hour rather than a permanent session"
        );
        assert!(read.is_temporary(), "an STS session is temporary");
        assert_eq!(read.account_id(), None, "no assumed role user, no account");

        let unreadable = body.replace(
            "</SessionToken>",
            "</SessionToken><Expiration>tomorrow</Expiration>",
        );
        assert_eq!(
            parse(unreadable.as_bytes(), "AssumeRole", now)
                .expect("a credential set")
                .expires_at(),
            Some(now + Duration::from_secs(3600)),
            "an expiry nothing reads is the same conservative hour"
        );

        let foreign_arn = body.replace(
            "</Credentials>",
            "</Credentials><AssumedRoleUser><Arn>not-an-arn</Arn></AssumedRoleUser>",
        );
        assert_eq!(
            parse(foreign_arn.as_bytes(), "AssumeRole", now)
                .expect("a credential set")
                .account_id(),
            None,
            "an ARN without an account field names no account"
        );
    }

    #[test]
    fn an_answer_that_is_not_the_actions_own_carries_no_credential_set() {
        let secretless = "<AssumeRoleResponse><AssumeRoleResult><Credentials>\
            <AccessKeyId>ASIA</AccessKeyId><SessionToken>token</SessionToken>\
            </Credentials></AssumeRoleResult></AssumeRoleResponse>";
        let keyless = "<AssumeRoleResponse><AssumeRoleResult><Credentials>\
            <SecretAccessKey>secret</SecretAccessKey><SessionToken>token</SessionToken>\
            </Credentials></AssumeRoleResult></AssumeRoleResponse>";
        let cases: [(&str, &str, &str); 9] = [
            (
                "the other action's answer",
                ASSUME_ROLE_ANSWER,
                "AssumeRoleWithWebIdentity",
            ),
            (
                "the other action's answer, the other way round",
                WEB_IDENTITY_ANSWER,
                "AssumeRole",
            ),
            ("a refusal", ERROR_RESPONSE, "AssumeRole"),
            (
                "no result",
                "<AssumeRoleResponse><ResponseMetadata/></AssumeRoleResponse>",
                "AssumeRole",
            ),
            (
                "no credentials",
                "<AssumeRoleResponse><AssumeRoleResult><AssumedRoleUser>\
                 <Arn>arn:aws:sts::123456789012:assumed-role/demo/x</Arn>\
                 </AssumedRoleUser></AssumeRoleResult></AssumeRoleResponse>",
                "AssumeRole",
            ),
            ("no secret", secretless, "AssumeRole"),
            ("no access key", keyless, "AssumeRole"),
            (
                "a truncated answer",
                "<AssumeRoleResponse><AssumeRoleResult>",
                "AssumeRole",
            ),
            ("not XML at all", "{\"Credentials\": {}}", "AssumeRole"),
        ];
        for (case, body, action) in cases {
            assert!(
                parse(body.as_bytes(), action, at(NOW)).is_none(),
                "{case} is no credential set"
            );
        }
        assert!(
            parse(b"", "AssumeRole", at(NOW)).is_none(),
            "an empty body is no credential set"
        );
    }

    #[test]
    fn a_refusal_is_read_as_its_code_and_message_wrapped_or_bare() {
        assert_eq!(
            parse_error(ERROR_RESPONSE.as_bytes()),
            Some((
                "AccessDenied".to_owned(),
                "User: arn:aws:iam::123456789012:user/desk is not authorized to perform: \
                 sts:AssumeRole on resource: arn:aws:iam::123456789012:role/lake-reader"
                    .to_owned()
            )),
            "<ErrorResponse><Error> is STS's own shape"
        );
        assert_eq!(
            parse_error(
                b"<Error><Code>ExpiredToken</Code>\
                  <Message>The security token included in the request is expired</Message></Error>"
            ),
            Some((
                "ExpiredToken".to_owned(),
                "The security token included in the request is expired".to_owned()
            )),
            "a bare <Error> reads the same"
        );
        assert_eq!(
            parse_error(b"<ErrorResponse><Error><Code>Throttling</Code></Error></ErrorResponse>"),
            Some(("Throttling".to_owned(), String::new())),
            "a refusal without a message still has its code"
        );
        for (case, body) in [
            ("a credential answer", ASSUME_ROLE_ANSWER.as_bytes()),
            (
                "an error response without its error",
                b"<ErrorResponse><RequestId>x</RequestId></ErrorResponse>".as_slice(),
            ),
            ("not XML at all", b"Service Unavailable".as_slice()),
            ("an empty body", b"".as_slice()),
        ] {
            assert_eq!(parse_error(body), None, "{case} is no refusal");
        }
    }

    // --- the CLI cache key ----------------------------------------------------

    #[test]
    fn a_roles_cache_file_is_named_as_botocore_names_it() {
        // Each hex is `hashlib.sha1(json.dumps(args, sort_keys=True).encode()).hexdigest()`
        // over the arguments botocore keys its cache by.
        assert_eq!(
            cache_key(&AssumedRole::new(ROLE)),
            ROLE_CACHE_KEY,
            r#"sha1 of {{"RoleArn": "arn:aws:iam::123456789012:role/lake-reader"}}"#
        );
        assert_eq!(
            cache_key(&AssumedRole::new(ROLE).with_session_name("power-desk")),
            NAMED_ROLE_CACHE_KEY,
            "a chosen session name is part of the key"
        );
        let full = AssumedRole::new(ROLE)
            .with_session_name("power-desk")
            .with_external_id("desk-42")
            .with_duration(Duration::from_secs(7200))
            .with_mfa_serial(MFA);
        assert_eq!(
            cache_key(&full),
            "78de243c8d911e2623a26e73d3eb0d6ca2970fb7",
            r#"sha1 of {{"DurationSeconds": 7200, "ExternalId": "desk-42", "RoleArn": "…", "RoleSessionName": "power-desk", "SerialNumber": "…:mfa/desk"}}, keys sorted, the duration a number"#
        );
        assert_eq!(
            cache_key(&AssumedRole::new(ROLE).with_duration(Duration::from_secs(60))),
            "6e84ab22dfceb49a40a6af6f4143d9fd89f54d3f",
            r#"sha1 of {{"DurationSeconds": 900, "RoleArn": "…"}}: the duration asked for, clamped"#
        );
    }

    #[test]
    fn a_generated_session_name_and_where_the_exchange_goes_stay_out_of_the_cache_key() {
        let role = AssumedRole::new(ROLE);
        let generated = session_name_at(&role, at(NOW));
        assert!(
            generated.starts_with("yggdryl-session-"),
            "a name is generated: {generated}"
        );
        assert_eq!(
            cache_key(&role),
            ROLE_CACHE_KEY,
            "a generated name changes per exchange, so the key leaves it out"
        );
        let routed = AssumedRole::new(ROLE)
            .with_region("eu-central-1")
            .with_endpoint("https://sts.example.test")
            .with_source_profile("base")
            .with_credential_source(CredentialSource::Environment)
            .with_web_identity_token_file("/var/run/secrets/token");
        assert_eq!(
            cache_key(&routed),
            ROLE_CACHE_KEY,
            "where the exchange goes and what signs it are not what it asks for"
        );
    }

    #[test]
    fn a_session_name_is_generated_from_the_instant_only_when_none_was_chosen() {
        let role = AssumedRole::new(ROLE);
        assert_eq!(
            session_name_at(&role, at(NOW)),
            "yggdryl-session-1800000000",
            "the prefix and the instant's whole seconds"
        );
        assert_eq!(
            session_name_at(&role, at(NOW) + Duration::from_millis(999)),
            "yggdryl-session-1800000000",
            "a fraction of a second is not part of the name"
        );
        assert_eq!(
            session_name_at(&role, UNIX_EPOCH - Duration::from_secs(5)),
            "yggdryl-session-0",
            "an instant before the epoch reads as the epoch"
        );
        let named = role.with_session_name("power-desk");
        assert_eq!(
            session_name_at(&named, at(NOW)),
            "power-desk",
            "a chosen name is used as it is"
        );
    }

    // --- the CLI cache ----------------------------------------------------------

    #[test]
    fn a_written_session_reads_back_and_spells_its_expiration_as_the_cli_does() {
        // A directory that does not exist yet: the write creates it.
        let directory = scratch("sts-cache-round-trip").join("cli").join("cache");
        let now = at(NOW);
        let expiry = now + Duration::from_secs(3600);
        let session = Credentials::new("ASIAROUNDTRIP", "round-trip-secret")
            .with_session_token("round-trip-token")
            .with_expiry(expiry)
            .with_account_id("123456789012");
        write_cache(&directory, ROLE_CACHE_KEY, &session);

        let text = std::fs::read_to_string(directory.join(format!("{ROLE_CACHE_KEY}.json")))
            .expect("the directory is created and the file written");
        let document: serde_json::Value = serde_json::from_str(&text).expect("the file is JSON");
        assert_eq!(
            document,
            serde_json::json!({
                "Credentials": {
                    "AccessKeyId": "ASIAROUNDTRIP",
                    "SecretAccessKey": "round-trip-secret",
                    "SessionToken": "round-trip-token",
                    "Expiration": "2027-01-15T09:00:00UTC",
                    "AccountId": "123456789012",
                }
            }),
            "the shape the AWS CLI reads, its expiration ending in UTC: {text}"
        );

        assert_eq!(
            read_cache(&directory, ROLE_CACHE_KEY, now),
            Some(
                Credentials::new("ASIAROUNDTRIP", "round-trip-secret")
                    .with_session_token("round-trip-token")
                    .with_expiry(expiry)
                    .with_account_id("123456789012")
            ),
            "the session reads back with the account filed beside its keys"
        );
    }

    #[test]
    fn a_set_without_an_expiry_is_not_cached() {
        let directory = scratch("sts-cache-long-lived");
        write_cache(
            &directory,
            ROLE_CACHE_KEY,
            &Credentials::new("AKIALONGLIVED", "long-lived-secret"),
        );
        assert!(
            std::fs::read(directory.join(format!("{ROLE_CACHE_KEY}.json"))).is_err(),
            "a long-lived set is not a session to cache"
        );
    }

    #[test]
    fn a_cached_session_lapsing_within_fifteen_minutes_is_not_read_back() {
        let directory = scratch("sts-cache-window");
        let now = at(NOW);
        for (lasts, read) in [
            (0, false),
            (600, false),
            (900, false),
            (901, true),
            (3600, true),
        ] {
            write_cache(
                &directory,
                ROLE_CACHE_KEY,
                &Credentials::new("ASIAWINDOW", "window-secret")
                    .with_session_token("window-token")
                    .with_expiry(now + Duration::from_secs(lasts)),
            );
            assert_eq!(
                read_cache(&directory, ROLE_CACHE_KEY, now).is_some(),
                read,
                "a session lasting {lasts} s more"
            );
        }
        write_cache(
            &directory,
            ROLE_CACHE_KEY,
            &Credentials::new("ASIALAPSED", "lapsed-secret")
                .with_session_token("lapsed-token")
                .with_expiry(now - Duration::from_secs(60)),
        );
        assert_eq!(
            read_cache(&directory, ROLE_CACHE_KEY, now),
            None,
            "a lapsed session is not read back"
        );
    }

    #[test]
    fn a_cache_file_the_aws_cli_wrote_is_read_whatever_else_it_holds() {
        let directory = scratch("sts-cache-cli-shape");
        let now = at(NOW);
        for expiration in ["2027-01-15T10:00:00UTC", "2027-01-15T10:00:00Z"] {
            file_cli_session(&directory, ROLE_CACHE_KEY, "ASIAFROMCLI", expiration);
            assert_eq!(
                read_cache(&directory, ROLE_CACHE_KEY, now),
                Some(
                    Credentials::new("ASIAFROMCLI", "cli-secret")
                        .with_session_token("cli-token")
                        .with_expiry(at(NOW + 7200))
                ),
                "the CLI's session, its expiration spelled {expiration}"
            );
        }
    }

    #[test]
    fn a_cache_file_nothing_can_read_answers_nothing() {
        let directory = scratch("sts-cache-unreadable");
        let now = at(NOW);
        assert_eq!(
            read_cache(&directory, ROLE_CACHE_KEY, now),
            None,
            "no file, no session"
        );
        let file = directory.join(format!("{ROLE_CACHE_KEY}.json"));
        for body in [
            "",
            "{not json",
            "[]",
            r#"{"Credentials": null}"#,
            r#"{"AccessKeyId": "ASIA", "SecretAccessKey": "s", "SessionToken": "t", "Expiration": "2027-01-15T10:00:00UTC"}"#,
            r#"{"Credentials": {"AccessKeyId": "ASIA", "SecretAccessKey": "s", "SessionToken": "t"}}"#,
            r#"{"Credentials": {"AccessKeyId": "ASIA", "SecretAccessKey": "s", "SessionToken": "t", "Expiration": "soon"}}"#,
            r#"{"Credentials": {"SecretAccessKey": "s", "SessionToken": "t", "Expiration": "2027-01-15T10:00:00UTC"}}"#,
            r#"{"Credentials": {"AccessKeyId": "ASIA", "SessionToken": "t", "Expiration": "2027-01-15T10:00:00UTC"}}"#,
        ] {
            std::fs::write(&file, body).expect("the cache file");
            assert_eq!(
                read_cache(&directory, ROLE_CACHE_KEY, now),
                None,
                "{body:?} is no session"
            );
        }
        std::fs::create_dir_all(directory.join("folder.json")).expect("a directory");
        assert_eq!(
            read_cache(&directory, "folder", now),
            None,
            "a directory where the file would be is no session"
        );
    }
}
