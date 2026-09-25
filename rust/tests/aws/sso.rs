//! `rust/src/aws/sso.rs`: IAM Identity Center - the sign-in a caller states,
//! the token cache the AWS CLI shares, the refresh, the device flow and the
//! portal exchange.
//!
//! What a caller can observe - the `Sso` builder, `SsoLogin`, the
//! `DeviceAuthorization` a handler is shown, and what a sealed `Session`
//! stating an `Sso` sends to the identity fake and files on disk - sits at
//! the top level. The token document, the cache file names and the windows a
//! token moves through, which no caller can name, are pinned in `internal`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yggdryl::aws::{Credentials, DeviceAuthorization, Session, Sso, SsoLogin};

use crate::identity::{Identity, Recorded, SSO_REFRESH_TOKEN, SSO_TOKEN, iso8601};
use crate::mod_::sealed;

/// The sign-in every exchange here states.
const START_URL: &str = "https://trading.awsapps.com/start";
const SSO_REGION: &str = "eu-west-1";
const ACCOUNT: &str = "123456789012";
/// The fake's portal answers this role as `ASIALakeReader`.
const ROLE: &str = "LakeReader";
const ACCESS_KEY: &str = "ASIALakeReader";
/// The `[sso-session]` the sign-in belongs to.
const SESSION_NAME: &str = "trading";
/// `sha1("trading")`: a sign-in that belongs to an `[sso-session]` is filed
/// under the session's name, as botocore's `SSOTokenLoader` files it.
const TOKEN_KEY: &str = "a31f0db8caea0a61995e27920e5a1da0c0128998";
/// `sha1("https://trading.awsapps.com/start")`: the older profile shape,
/// with no session, files its token under the start URL.
const START_URL_TOKEN_KEY: &str = "34fbc6bc520916e2ef4f2e751ef6f7f4cb8619a6";
/// The portal's path.
const PORTAL: &str = "/federation/credentials";

/// What a device sign-in showed the handler, in order.
type Shown = Arc<Mutex<Vec<DeviceAuthorization>>>;

fn trading() -> Sso {
    Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE).with_session_name(SESSION_NAME)
}

fn now_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_secs();
    i64::try_from(seconds).expect("a representable instant")
}

fn directory(session: &Session) -> PathBuf {
    session
        .directory()
        .expect("a sealed session names its scratch directory")
}

/// Where `aws sso login` files the token of the sign-in keyed `key`.
fn token_file(session: &Session, key: &str) -> PathBuf {
    directory(session)
        .join("sso")
        .join("cache")
        .join(format!("{key}.json"))
}

/// Where the AWS CLI files the role's keys.
fn cli_cache(session: &Session) -> PathBuf {
    directory(session).join("cli").join("cache")
}

/// A sign-in token as `aws sso login` files it, lapsing `expires_in` seconds
/// from now.
fn signed_in(access_token: &str, expires_in: i64) -> serde_json::Value {
    serde_json::json!({
        "startUrl": START_URL,
        "region": SSO_REGION,
        "accessToken": access_token,
        "expiresAt": iso8601(now_seconds() + expires_in),
    })
}

/// The same, with the client registration a refresh needs.
fn refreshable(access_token: &str, expires_in: i64, refresh_token: &str) -> serde_json::Value {
    let mut document = signed_in(access_token, expires_in);
    document["clientId"] = "client-1".into();
    document["clientSecret"] = "client-secret-1".into();
    document["registrationExpiresAt"] = iso8601(now_seconds() + 90 * 86_400).into();
    document["refreshToken"] = refresh_token.into();
    document
}

fn file_token(session: &Session, key: &str, document: &serde_json::Value) {
    let path = token_file(session, key);
    std::fs::create_dir_all(path.parent().expect("the token file has a directory"))
        .expect("the SSO cache directory");
    std::fs::write(&path, document.to_string()).expect("the token file");
}

fn read_json(path: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is JSON: {error}: {text}", path.display()))
}

/// The one file the session filed in the CLI cache.
fn only_cli_file(session: &Session) -> serde_json::Value {
    let files: Vec<PathBuf> = std::fs::read_dir(cli_cache(session))
        .expect("the CLI cache directory")
        .map(|entry| entry.expect("a readable entry").path())
        .collect();
    assert_eq!(files.len(), 1, "one set is filed: {files:?}");
    read_json(&files[0])
}

/// `METHOD /path` of every request the fake handled, in arrival order.
fn shape(identity: &Identity) -> Vec<String> {
    identity
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// The requests that did not go to the instance metadata service.
fn off_instance(identity: &Identity) -> Vec<String> {
    shape(identity)
        .into_iter()
        .filter(|request| !request.contains("/latest/"))
        .collect()
}

fn body(request: &Recorded) -> serde_json::Value {
    serde_json::from_str(&request.body)
        .unwrap_or_else(|error| panic!("a JSON body: {error}: {}", request.body))
}

fn found(session: &Session) -> Credentials {
    session
        .credentials(SystemTime::now())
        .expect("the chain answers")
        .expect("a credential set")
}

fn refused(session: &Session) -> String {
    session
        .credentials(SystemTime::now())
        .expect_err("nothing answers")
        .to_string()
}

/// A login that records what it is shown.
fn capturing() -> (SsoLogin, Shown) {
    let shown: Shown = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&shown);
    let login = SsoLogin::Handler(Arc::new(move |authorization: &DeviceAuthorization| {
        record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(authorization.clone());
    }));
    (login, shown)
}

fn shown_to(shown: &Shown) -> Vec<DeviceAuthorization> {
    shown.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

// --- the sign-in a caller states --------------------------------------------

#[test]
fn a_sign_in_states_its_start_url_region_account_and_role_trimmed_and_nothing_else() {
    let sso = Sso::new(
        " https://trading.awsapps.com/start ",
        " eu-west-1\n",
        "123456789012 ",
        "\tLakeReader",
    );
    assert_eq!(sso.start_url(), START_URL, "trimmed");
    assert_eq!(sso.region(), SSO_REGION, "trimmed");
    assert_eq!(sso.account_id(), ACCOUNT, "trimmed");
    assert_eq!(sso.role_name(), ROLE, "trimmed");
    assert_eq!(sso.session_name(), None, "no session was named");
    assert!(
        sso.scopes().is_empty(),
        "no scope was named, so the sign-in registers for the default"
    );
    assert_eq!(
        sso,
        Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE),
        "the same sign-in whichever way it was spelled"
    );
}

#[test]
fn a_session_name_is_trimmed_and_a_blank_one_is_no_session() {
    let named = Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE).with_session_name("  trading ");
    assert_eq!(named.session_name(), Some(SESSION_NAME), "trimmed");
    assert_eq!(named, trading());
    let blank = named.with_session_name("   ");
    assert_eq!(
        blank.session_name(),
        None,
        "a blank name is no name, and clears the one before it"
    );
}

#[test]
fn scopes_replace_the_default_registration_and_read_back_in_order() {
    let sso = trading().with_scopes(["sso:account:access", "codewhisperer:completions"]);
    assert_eq!(
        sso.scopes().to_vec(),
        vec![
            "sso:account:access".to_owned(),
            "codewhisperer:completions".to_owned()
        ],
        "in the order given"
    );
    assert!(
        sso.with_scopes(Vec::<String>::new()).scopes().is_empty(),
        "an empty list is the default again"
    );
}

#[test]
fn a_session_reads_back_the_sign_in_it_was_given() {
    let session = Session::new().with_sso(trading());
    assert_eq!(session.sso(), Some(&trading()));
    assert_eq!(
        session.sso().map(Sso::session_name),
        Some(Some(SESSION_NAME))
    );
    assert!(Session::new().sso().is_none(), "none unless stated");
}

#[test]
fn the_login_a_session_is_told_debugs_as_its_variant_and_defaults_to_never() {
    assert_eq!(format!("{:?}", SsoLogin::default()), "Never");
    assert_eq!(format!("{:?}", SsoLogin::Never), "Never");
    assert_eq!(format!("{:?}", SsoLogin::Stderr), "Stderr");
    let handler = SsoLogin::Handler(Arc::new(|_: &DeviceAuthorization| {}));
    assert_eq!(
        format!("{handler:?}"),
        "Handler",
        "a handler is named, never rendered"
    );
    assert!(
        matches!(Session::new().sso_login(), SsoLogin::Never),
        "a session signs nobody in unless told how to show the sign-in"
    );
    let told = Session::new().with_sso_login(handler);
    assert_eq!(format!("{:?}", told.sso_login()), "Handler");
}

// --- a cached sign-in -------------------------------------------------------

#[test]
fn a_cached_sign_in_is_traded_at_the_portal_once_and_the_role_s_keys_filed_where_the_cli_reads_them()
 {
    let identity = Identity::start();
    let session = sealed(&identity, "sso-cached").with_sso(trading());
    file_token(&session, TOKEN_KEY, &signed_in(SSO_TOKEN, 3600));

    let keys = found(&session);
    assert_eq!(keys.access_key_id(), ACCESS_KEY, "the role's keys");
    assert_eq!(keys.session_token(), Some("sso-session-token-LakeReader"));
    assert_eq!(
        keys.account_id(),
        Some(ACCOUNT),
        "the account the role is in"
    );
    assert!(
        keys.expires_at().is_some_and(|at| at > SystemTime::now()),
        "the portal's expiry is kept: {:?}",
        keys.expires_at()
    );
    assert_eq!(session.credential_source(), Some("sso"));
    assert_eq!(
        shape(&identity),
        [format!("GET {PORTAL}")],
        "one portal request: no refresh, no sign-in, nothing else in the chain"
    );
    let portal = &identity.requests()[0];
    assert_eq!(portal.query("role_name"), Some(ROLE));
    assert_eq!(portal.query("account_id"), Some(ACCOUNT));
    assert_eq!(
        portal.header("x-amz-sso_bearer_token"),
        Some(SSO_TOKEN),
        "the cached token is the bearer"
    );

    let filed = only_cli_file(&session);
    let held = &filed["Credentials"];
    assert_eq!(held["AccessKeyId"], ACCESS_KEY, "{filed}");
    assert_eq!(
        held["SecretAccessKey"], "secret-of-LakeReader-in-123456789012",
        "{filed}"
    );
    assert_eq!(
        held["SessionToken"], "sso-session-token-LakeReader",
        "{filed}"
    );
    assert!(
        held["Expiration"].as_str().is_some_and(|at| !at.is_empty()),
        "the expiration the CLI reads back: {filed}"
    );
}

#[test]
fn a_second_session_over_the_same_directory_answers_from_the_cli_cache_without_a_request() {
    let identity = Identity::start();
    let session = sealed(&identity, "sso-reuse").with_sso(trading());
    file_token(&session, TOKEN_KEY, &signed_in(SSO_TOKEN, 3600));
    found(&session);
    identity.clear_requests();
    // Without the token, only the CLI cache can answer.
    std::fs::remove_file(token_file(&session, TOKEN_KEY)).expect("the token file");

    let second = session.with_region("eu-west-3");
    let keys = found(&second);
    assert_eq!(keys.access_key_id(), ACCESS_KEY, "the cached keys");
    assert_eq!(keys.session_token(), Some("sso-session-token-LakeReader"));
    assert_eq!(second.credential_source(), Some("sso"));
    assert_eq!(
        identity.request_count(),
        0,
        "the keys the CLI cache holds are not asked for again: {:?}",
        shape(&identity)
    );
}

#[test]
fn a_sign_in_without_a_session_reads_the_token_filed_under_its_start_url() {
    let identity = Identity::start();
    let session =
        sealed(&identity, "sso-start-url").with_sso(Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE));
    file_token(&session, START_URL_TOKEN_KEY, &signed_in(SSO_TOKEN, 3600));

    assert_eq!(found(&session).access_key_id(), ACCESS_KEY);
    assert_eq!(shape(&identity), [format!("GET {PORTAL}")]);
}

// --- a token near or past its expiry ------------------------------------------

#[test]
fn a_token_lapsing_within_fifteen_minutes_is_refreshed_once_filed_again_and_the_fresh_one_traded() {
    let identity = Identity::start();
    identity.set_sso_token("fresh");
    identity.require_refresh_token(Some("cached-refresh"));
    let session = sealed(&identity, "sso-refresh").with_sso(trading());
    file_token(
        &session,
        TOKEN_KEY,
        &refreshable("stale", 5 * 60, "cached-refresh"),
    );

    assert_eq!(found(&session).access_key_id(), ACCESS_KEY);
    assert_eq!(session.credential_source(), Some("sso"));
    assert_eq!(
        shape(&identity),
        ["POST /token".to_owned(), format!("GET {PORTAL}")],
        "one refresh, then one trade"
    );
    let requests = identity.requests();
    let refresh = body(&requests[0]);
    assert_eq!(refresh["grantType"], "refresh_token", "{refresh}");
    assert_eq!(refresh["refreshToken"], "cached-refresh", "{refresh}");
    assert_eq!(refresh["clientId"], "client-1", "{refresh}");
    assert_eq!(refresh["clientSecret"], "client-secret-1", "{refresh}");
    assert_eq!(
        requests[1].header("x-amz-sso_bearer_token"),
        Some("fresh"),
        "the refreshed token is what is traded"
    );

    let filed = read_json(&token_file(&session, TOKEN_KEY));
    assert_eq!(filed["accessToken"], "fresh", "{filed}");
    assert_eq!(
        filed["refreshToken"], SSO_REFRESH_TOKEN,
        "the refresh token the service rotated to: {filed}"
    );
    assert_eq!(
        filed["clientId"], "client-1",
        "the registration is kept: {filed}"
    );
    assert_eq!(filed["clientSecret"], "client-secret-1", "{filed}");
    assert_eq!(filed["startUrl"], START_URL, "{filed}");
    assert_eq!(filed["region"], SSO_REGION, "{filed}");
    let expires_at = filed["expiresAt"].as_str().unwrap_or_default();
    assert!(
        expires_at > iso8601(now_seconds() + 30 * 60).as_str(),
        "the fresh token lasts the hour the service granted: {filed}"
    );
}

#[test]
fn a_token_near_its_expiry_whose_registration_lapsed_is_traded_as_it_is_without_a_refresh() {
    let identity = Identity::start();
    let session = sealed(&identity, "sso-registration-lapsed").with_sso(trading());
    let mut document = refreshable(SSO_TOKEN, 5 * 60, "cached-refresh");
    document["registrationExpiresAt"] = iso8601(now_seconds() - 60).into();
    file_token(&session, TOKEN_KEY, &document);

    assert_eq!(
        found(&session).access_key_id(),
        ACCESS_KEY,
        "the token still stands, so it is traded"
    );
    assert_eq!(
        shape(&identity),
        [format!("GET {PORTAL}")],
        "a client whose registration lapsed cannot refresh, so none is attempted"
    );
    let filed = read_json(&token_file(&session, TOKEN_KEY));
    assert_eq!(
        filed["accessToken"], SSO_TOKEN,
        "the token file is left as it was"
    );
}

#[test]
fn a_lapsed_token_with_no_refresh_under_never_is_a_refusal_naming_aws_sso_login() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = sealed(&identity, "sso-lapsed")
        .with_sso(trading())
        .with_sso_login(SsoLogin::Never);
    file_token(&session, TOKEN_KEY, &signed_in("lapsed", -60));

    let message = refused(&session);
    assert!(message.contains("sso: "), "the source is named: {message}");
    assert!(
        message.contains("`aws sso login`"),
        "the refusal names the way out: {message}"
    );
    assert!(message.contains(START_URL), "and the sign-in: {message}");
    assert_eq!(
        off_instance(&identity),
        Vec::<String>::new(),
        "no refresh without a client, no sign-in under SsoLogin::Never, no portal request"
    );
    assert!(!cli_cache(&session).exists(), "a refusal files nothing");
}

#[test]
fn a_token_file_that_is_not_a_token_is_no_sign_in_rather_than_a_failure_of_its_own() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = sealed(&identity, "sso-garbled").with_sso(trading());
    let path = token_file(&session, TOKEN_KEY);
    std::fs::create_dir_all(path.parent().expect("a directory")).expect("the SSO cache");
    std::fs::write(&path, "{\"accessToken\": \"half-written").expect("a torn token file");

    let message = refused(&session);
    assert!(
        message.contains("`aws sso login`"),
        "a torn file reads as no sign-in, whose way out is a sign-in: {message}"
    );
    assert_eq!(off_instance(&identity), Vec::<String>::new());
}

#[test]
fn a_token_the_portal_no_longer_accepts_is_a_recorded_failure_and_the_chain_walks_on_past_it() {
    let identity = Identity::start();
    identity.set_imds_role(None);
    let session = sealed(&identity, "sso-portal-refused").with_sso(trading());
    file_token(&session, TOKEN_KEY, &signed_in(SSO_TOKEN, 3600));
    identity.set_sso_token("other");

    let message = refused(&session);
    assert!(message.contains("sso: "), "the source is named: {message}");
    assert!(
        message.contains("GetRoleCredentials") && message.contains("401"),
        "the portal's operation and status: {message}"
    );
    assert!(
        message.contains("UnauthorizedException"),
        "the portal's own code: {message}"
    );
    let portal: Vec<Recorded> = identity
        .requests()
        .into_iter()
        .filter(|request| request.path == PORTAL)
        .collect();
    assert_eq!(portal.len(), 1, "the portal is asked once");
    assert_eq!(portal[0].status, 401);
    assert!(!cli_cache(&session).exists(), "a refusal files nothing");

    // With a role on the instance, the refused sign-in is passed over.
    identity.set_imds_role(Some("instance-role"));
    let walked = session.with_region("eu-west-3");
    assert_eq!(found(&walked).access_key_id(), "ASIAINSTANCEROLE");
    assert_eq!(walked.credential_source(), Some("instance metadata"));
}

// --- the device sign-in -----------------------------------------------------

#[test]
fn a_device_sign_in_registers_authorizes_polls_until_confirmed_then_trades_and_files_a_refreshable_token()
 {
    let identity = Identity::start();
    identity.sso_pending_polls(2);
    let (login, shown) = capturing();
    let session = sealed(&identity, "sso-device")
        .with_sso(trading().with_scopes(["sso:account:access", "codewhisperer:completions"]))
        .with_sso_login(login);

    assert_eq!(found(&session).access_key_id(), ACCESS_KEY);
    assert_eq!(session.credential_source(), Some("sso"));
    assert_eq!(
        shown_to(&shown).len(),
        1,
        "the person is shown the sign-in once"
    );
    assert_eq!(
        shape(&identity),
        [
            "POST /client/register".to_owned(),
            "POST /device_authorization".to_owned(),
            "POST /token".to_owned(),
            "POST /token".to_owned(),
            "POST /token".to_owned(),
            format!("GET {PORTAL}"),
        ],
        "register, authorize, poll until confirmed, trade"
    );
    let requests = identity.requests();
    let statuses: Vec<u16> = requests.iter().map(|request| request.status).collect();
    assert_eq!(
        statuses,
        [200, 200, 400, 400, 200, 200],
        "two polls pending"
    );
    let registered = body(&requests[0]);
    assert_eq!(registered["clientType"], "public", "{registered}");
    assert_eq!(
        registered["scopes"],
        serde_json::json!(["sso:account:access", "codewhisperer:completions"]),
        "the scopes the sign-in states: {registered}"
    );
    let started = body(&requests[1]);
    assert_eq!(started["clientId"], "client-1", "{started}");
    assert_eq!(started["clientSecret"], "client-secret-1", "{started}");
    assert_eq!(started["startUrl"], START_URL, "{started}");
    for poll in requests[2..5].iter().map(body) {
        assert_eq!(
            poll["grantType"], "urn:ietf:params:oauth:grant-type:device_code",
            "{poll}"
        );
        assert_eq!(poll["deviceCode"], "device-1", "{poll}");
        assert_eq!(poll["clientId"], "client-1", "{poll}");
    }
    assert_eq!(
        requests[5].header("x-amz-sso_bearer_token"),
        Some(SSO_TOKEN),
        "the token the sign-in produced is what is traded"
    );

    let filed = read_json(&token_file(&session, TOKEN_KEY));
    assert_eq!(filed["accessToken"], SSO_TOKEN, "{filed}");
    assert_eq!(filed["refreshToken"], SSO_REFRESH_TOKEN, "{filed}");
    assert_eq!(filed["clientId"], "client-1", "{filed}");
    assert_eq!(filed["clientSecret"], "client-secret-1", "{filed}");
    assert!(filed["registrationExpiresAt"].is_string(), "{filed}");
    assert_eq!(filed["startUrl"], START_URL, "{filed}");
    assert_eq!(filed["region"], SSO_REGION, "{filed}");
    assert_eq!(
        only_cli_file(&session)["Credentials"]["AccessKeyId"],
        ACCESS_KEY
    );
}

#[test]
fn a_device_sign_in_shows_the_handler_the_page_and_the_code_to_confirm() {
    let identity = Identity::start();
    let (login, shown) = capturing();
    let session = sealed(&identity, "sso-device-shown")
        .with_sso(trading())
        .with_sso_login(login);
    found(&session);

    let shown = shown_to(&shown);
    assert_eq!(shown.len(), 1, "shown once");
    let authorization = &shown[0];
    assert_eq!(authorization.user_code(), "ABCD-EFGH");
    assert_eq!(authorization.verification_uri(), "https://device.sso.test/");
    assert_eq!(
        authorization.verification_uri_complete(),
        Some("https://device.sso.test/?user_code=ABCD-EFGH")
    );
    assert_eq!(authorization.expires_in(), Duration::from_secs(600));
    assert_eq!(authorization.interval(), Duration::ZERO);
    assert!(
        !format!("{authorization:?}").contains("device-1"),
        "the device code is a secret the handler is not shown: {authorization:?}"
    );
    assert_eq!(
        authorization.to_string(),
        "Open https://device.sso.test/?user_code=ABCD-EFGH and confirm the code ABCD-EFGH",
        "the page with the code filled in, when the service offered one"
    );
}

#[test]
fn login_files_the_token_without_asking_the_portal_and_the_next_request_trades_it() {
    let identity = Identity::start();
    let (login, shown) = capturing();
    let session = sealed(&identity, "sso-login")
        .with_sso(trading())
        .with_sso_login(login);

    session.login().expect("the sign-in completes");
    assert_eq!(
        shape(&identity),
        [
            "POST /client/register",
            "POST /device_authorization",
            "POST /token"
        ],
        "a sign-in and nothing else: the portal is not asked"
    );
    assert_eq!(
        body(&identity.requests()[0])["scopes"],
        serde_json::json!(["sso:account:access"]),
        "a sign-in stating no scope registers for the default"
    );
    assert_eq!(shown_to(&shown).len(), 1);
    let filed = read_json(&token_file(&session, TOKEN_KEY));
    assert_eq!(filed["accessToken"], SSO_TOKEN, "{filed}");
    assert_eq!(filed["refreshToken"], SSO_REFRESH_TOKEN, "{filed}");
    assert!(
        !cli_cache(&session).exists(),
        "no role's keys were asked for, so none are filed"
    );

    identity.clear_requests();
    assert_eq!(found(&session).access_key_id(), ACCESS_KEY);
    assert_eq!(
        shape(&identity),
        [format!("GET {PORTAL}")],
        "the filed token is what the next request trades"
    );
}

#[test]
fn login_under_never_is_a_refusal_naming_aws_sso_login_with_no_request() {
    let identity = Identity::start();
    let session = sealed(&identity, "sso-login-never").with_sso(trading());

    let message = session
        .login()
        .expect_err("SsoLogin::Never signs nobody in")
        .to_string();
    assert!(message.contains("`aws sso login`"), "{message}");
    assert_eq!(identity.request_count(), 0, "{:?}", shape(&identity));
    assert!(
        !token_file(&session, TOKEN_KEY).exists(),
        "nothing is filed"
    );
}

#[test]
fn login_with_no_sign_in_stated_or_configured_is_a_refusal_naming_the_profile() {
    let identity = Identity::start();
    let (login, shown) = capturing();
    let session = sealed(&identity, "sso-login-none").with_sso_login(login);

    let message = session
        .login()
        .expect_err("there is nothing to sign in to")
        .to_string();
    assert!(
        message.contains("profile default") && message.contains("IAM Identity Center"),
        "{message}"
    );
    assert_eq!(identity.request_count(), 0, "{:?}", shape(&identity));
    assert!(shown_to(&shown).is_empty(), "nobody is shown anything");
}

#[cfg(feature = "internals")]
mod internal {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use yggdryl::aws::Sso;
    use yggdryl::internals::aws_sso::{
        Token, can_refresh, credentials_cache_key, is_stale, token_cache_key,
    };

    use super::{
        ACCOUNT, ROLE, SESSION_NAME, SSO_REGION, START_URL, START_URL_TOKEN_KEY, TOKEN_KEY, trading,
    };
    use crate::mod_::scratch;

    /// The instant every reading here is made at: 2027-01-15T08:00:00Z.
    const NOW: u64 = 1_800_000_000;

    /// The SHA-1 botocore's `SSOCredentialFetcher` files a role's keys under
    /// for a sign-in with no session: `json.dumps(args, sort_keys=True,
    /// separators=(',', ':'))` of
    /// `{"accountId":"123456789012","roleName":"LakeReader","startUrl":"https://trading.awsapps.com/start"}`.
    const START_URL_CREDENTIALS_KEY: &str = "284fc425e745ce990a6c162c2106d8b0da3c1e0e";

    /// The same for a sign-in that belongs to a session, which botocore
    /// keys by the session's name *in place of* the start URL:
    /// `{"accountId":"123456789012","roleName":"LakeReader","sessionName":"trading"}`.
    const CREDENTIALS_KEY: &str = "5a4979b4c3e157ee406d307c873db51fce1206ce";

    /// A token as `aws sso login` files it after a sign-in at `NOW`.
    const CLI_TOKEN: &str = r#"{
  "startUrl": "https://trading.awsapps.com/start",
  "region": "eu-west-1",
  "accessToken": "aoa-access-token",
  "expiresAt": "2027-01-15T09:00:00Z",
  "clientId": "client-1",
  "clientSecret": "client-secret-1",
  "registrationExpiresAt": "2027-04-15T08:00:00Z",
  "refreshToken": "aor-refresh-token"
}"#;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn cli_token() -> Token {
        Token::parse(CLI_TOKEN.as_bytes()).expect("the CLI's document is a token")
    }

    /// `CLI_TOKEN` without `key`.
    fn cli_token_without(key: &str) -> Token {
        let mut document: serde_json::Value =
            serde_json::from_str(CLI_TOKEN).expect("the CLI's document is JSON");
        let removed = document.as_object_mut().expect("an object").remove(key);
        assert!(removed.is_some(), "the CLI's document states {key}");
        Token::parse(document.to_string().as_bytes()).expect("still a token")
    }

    #[test]
    fn a_token_the_aws_cli_filed_reads_every_field_it_states() {
        let token = cli_token();
        assert_eq!(token.expires_at, at(NOW + 3600), "expiresAt");
        assert_eq!(token.client_id.as_deref(), Some("client-1"), "clientId");
        assert_eq!(
            token.registration_expires_at,
            Some(at(NOW + 90 * 86_400)),
            "registrationExpiresAt"
        );
        assert_eq!(token.region.as_deref(), Some(SSO_REGION), "region");
        assert_eq!(token.start_url.as_deref(), Some(START_URL), "startUrl");
        assert!(token.refresh_token.is_some(), "refreshToken");
        assert!(token.client_secret.is_some(), "clientSecret");
    }

    #[test]
    fn a_token_renders_back_to_the_document_the_aws_cli_filed() {
        let token = cli_token();
        let rendered: serde_json::Value =
            serde_json::from_str(&token.render()).expect("the rendering is JSON");
        let original: serde_json::Value =
            serde_json::from_str(CLI_TOKEN).expect("the CLI's document is JSON");
        assert_eq!(
            rendered, original,
            "every key the CLI wrote, spelled as the CLI spells it"
        );
        let again = Token::parse(token.render().as_bytes()).expect("the rendering reads back");
        assert_eq!(again.render(), token.render(), "a round trip is exact");
    }

    #[test]
    fn a_token_stating_only_what_it_must_reads_and_renders_only_that() {
        let token = Token::parse(br#"{"accessToken":"aoa","expiresAt":"2027-01-15T09:00:00Z"}"#)
            .expect("an access token and an expiry are a token");
        assert_eq!(token.expires_at, at(NOW + 3600));
        assert!(token.refresh_token.is_none());
        assert!(token.client_id.is_none());
        assert!(token.client_secret.is_none());
        assert!(token.registration_expires_at.is_none());
        assert!(token.region.is_none());
        assert!(token.start_url.is_none());
        let rendered: serde_json::Value =
            serde_json::from_str(&token.render()).expect("the rendering is JSON");
        assert_eq!(
            rendered,
            serde_json::json!({"accessToken": "aoa", "expiresAt": "2027-01-15T09:00:00Z"}),
            "an absent field is not written"
        );
    }

    #[test]
    fn a_token_the_older_cli_filed_with_a_utc_suffix_reads_the_same_instant() {
        let token = Token::parse(br#"{"accessToken":"aoa","expiresAt":"2027-01-15T09:00:00UTC"}"#)
            .expect("the older spelling is a token");
        assert_eq!(token.expires_at, at(NOW + 3600));
    }

    #[test]
    fn a_registration_expiry_that_is_not_an_instant_is_dropped_rather_than_the_token() {
        let token = Token::parse(
            br#"{"accessToken":"aoa","expiresAt":"2027-01-15T09:00:00Z","registrationExpiresAt":"never"}"#,
        )
        .expect("the token stands without it");
        assert_eq!(token.registration_expires_at, None);
    }

    #[test]
    fn a_document_missing_its_access_token_or_its_expiry_is_no_token() {
        for document in [
            r#"{"expiresAt":"2027-01-15T09:00:00Z"}"#,
            r#"{"accessToken":"","expiresAt":"2027-01-15T09:00:00Z"}"#,
            r#"{"accessToken":"aoa"}"#,
            r#"{"accessToken":"aoa","expiresAt":""}"#,
            r#"{"accessToken":"aoa","expiresAt":"tomorrow"}"#,
            r#"{"accessToken":"aoa","expiresAt":1800003600}"#,
            r#"{"accessToken":"aoa","expiresAt":"2027-01-15T09:00:00Z""#,
            "",
        ] {
            assert!(
                Token::parse(document.as_bytes()).is_none(),
                "{document:?} is no token"
            );
        }
    }

    #[test]
    fn a_token_s_debug_never_shows_its_secrets() {
        let debug = format!("{:?}", cli_token());
        for secret in ["aoa-access-token", "aor-refresh-token", "client-secret-1"] {
            assert!(!debug.contains(secret), "{secret} leaked: {debug}");
        }
    }

    #[test]
    fn a_token_written_to_a_cache_path_creates_the_directory_and_reads_back() {
        let directory = scratch("sso-token-write");
        let path = directory
            .join("sso")
            .join("cache")
            .join(format!("{TOKEN_KEY}.json"));
        let token = cli_token();
        token.write(&path).expect("the token file is written");
        let read = Token::read(&path).expect("the written file reads back");
        assert_eq!(read.render(), token.render());
        assert!(
            Token::read(&directory.join("absent.json")).is_none(),
            "no file is no token"
        );
    }

    #[test]
    fn a_token_is_stale_from_fifteen_minutes_before_it_lapses() {
        let token = cli_token();
        assert!(!is_stale(&token, at(NOW)), "an hour to go");
        assert!(
            !is_stale(&token, at(NOW + 3600 - 15 * 60 - 1)),
            "a second outside the window"
        );
        assert!(
            is_stale(&token, at(NOW + 3600 - 15 * 60)),
            "the window's edge is inside it"
        );
        assert!(is_stale(&token, at(NOW + 3600)), "lapsed");
        assert!(is_stale(&token, at(NOW + 7200)), "long lapsed");
    }

    #[test]
    fn a_token_can_refresh_only_with_a_refresh_token_a_client_and_a_standing_registration() {
        let token = cli_token();
        assert!(can_refresh(&token, at(NOW)));
        assert!(
            can_refresh(&token, at(NOW + 7200)),
            "a lapsed access token is refreshable while the registration stands"
        );
        assert!(
            !can_refresh(&token, at(NOW + 90 * 86_400)),
            "a registration lapses at its instant"
        );
        for missing in ["refreshToken", "clientId", "clientSecret"] {
            assert!(
                !can_refresh(&cli_token_without(missing), at(NOW)),
                "nothing to refresh with, without {missing}"
            );
        }
        assert!(
            can_refresh(&cli_token_without("registrationExpiresAt"), at(NOW)),
            "a registration that states no expiry is tried: a refused refresh costs one \
             request and keeps the token in hand"
        );
    }

    #[test]
    fn a_sign_in_s_token_is_filed_under_the_sha1_of_its_session_name_else_its_start_url() {
        assert_eq!(token_cache_key(&trading()), TOKEN_KEY, "the session's name");
        assert_eq!(
            token_cache_key(&Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE)),
            START_URL_TOKEN_KEY,
            "the start URL, for the older shape"
        );
        assert_eq!(
            token_cache_key(
                &Sso::new(START_URL, "us-east-1", "210987654321", "Auditor")
                    .with_session_name(SESSION_NAME)
            ),
            TOKEN_KEY,
            "every role of one sign-in shares its token"
        );
    }

    #[test]
    fn a_role_s_keys_are_filed_under_the_sha1_botocore_keys_them_by() {
        assert_eq!(
            credentials_cache_key(&Sso::new(START_URL, SSO_REGION, ACCOUNT, ROLE)),
            START_URL_CREDENTIALS_KEY,
            "accountId, roleName and startUrl, key-sorted and compact"
        );
        assert_eq!(
            credentials_cache_key(&trading()),
            CREDENTIALS_KEY,
            "a sign-in that belongs to a session is keyed by the session's name in place of \
             the start URL, as botocore's SSOCredentialFetcher._create_cache_key does"
        );
        assert_eq!(
            credentials_cache_key(
                &Sso::new(START_URL, "us-east-1", ACCOUNT, ROLE)
                    .with_session_name(SESSION_NAME)
                    .with_scopes(["sso:account:access"])
            ),
            CREDENTIALS_KEY,
            "neither the region nor the scopes enter the key"
        );
    }
}
