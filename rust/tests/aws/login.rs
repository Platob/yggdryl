//! `rust/src/aws/login.rs`: the AWS Console sign-in `aws login` files -
//! where the cache keeps it, what its document holds, the P-256 key its
//! refresh token is bound to, the `DPoP` proof that key signs - and its
//! refresh against the identity fake, which verifies every proof with its
//! own decoding and its own ES256 check.
//!
//! No caller names any of it until a profile's `login_session` reaches it
//! through `Session`, so the whole file reaches the crate through
//! `yggdryl::internals::aws_login`. What a refresh logs is read through the
//! crate's logging tree, installed as the `log` facade's backend.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use yggdryl::aws::Credentials;
use yggdryl::internals::aws_login::{
    REFRESH_WINDOW, cache_path, credentials, endpoint, jti, jwk, parse, proof,
};
use yggdryl::logging::{self, Handler, Level};

use crate::identity::{
    Identity, LOGIN_ACCESS_KEY, LOGIN_EXPIRES_IN, LOGIN_REFRESH_TOKEN, LOGIN_SECRET_KEY,
    LOGIN_SESSION_TOKEN, Recorded, iso8601,
};
use crate::logging::{Collect, Kept, serial};
use crate::mod_::scratch;

/// The sign-in most tests file: the session aws-sdk-rust's own cache tests
/// name.
const SESSION: &str = "arn:aws:iam::0123456789012:user/Admin";
/// `sha256(SESSION)`, the file `aws login` keeps it in: the name
/// aws-sdk-rust's `determine_correct_cache_filenames` pins, which Python's
/// `hashlib.sha256` answers too.
const SESSION_KEY: &str = "36db1d138ff460920374e4c3d8e01f53f9f73537e89c88d639f68393df0e2726";
/// The sign-in the tests reading the logging tree file, so no other test's
/// record is taken for theirs; its file name pinned the same way.
const OTHER_SESSION: &str = "arn:aws:iam::000000000000:user/PowerUser";
const OTHER_SESSION_KEY: &str = "d19c78f768c6a12874de5f41d7f22cbb834ba205704102da0db20d8496efecb5";
/// The profile a refusal's way out names.
const PROFILE: &str = "console";
const WAY_OUT: &str = "aws login --profile console";
/// The logger the crate's records from `aws::login` reach.
const LOGGER: &str = "yggdryl.aws.login";
/// The client `aws login` signs in as.
const CLIENT: &str = "arn:aws:signin:::devtools/same-device";
/// The account the cached set names, which a refresh carries over.
const ACCOUNT: &str = "012345678901";
/// What the cache holds before a refresh.
const CACHED_ACCESS_KEY: &str = "ASIACACHEDLOGIN";
const CACHED_SECRET: &str = "cached-login-secret";
const CACHED_SESSION_TOKEN: &str = "cached-login-session-token";
const CACHED_REFRESH_TOKEN: &str = "cached-login-refresh-token";
const CACHED_ID_TOKEN: &str = "cached-login-id-token";

/// The SEC1 key aws-sdk-rust's login cache module documents as the one
/// `aws login` files.
const KEY: &str = "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIFDZHUzOG1Pzq+6F0mjMlOSp1syN9LRPBuHMoCFXTcXhoAoGCCqGSM49\nAwEHoUQDQgAE9qhj+KtcdHj1kVgwxWWWw++tqoh7H7UHs7oXh8jBbgF47rrYGC+t\ndjiIaHK3dBvvdE7MGj5HsepzLm3Kj91bqA==\n-----END EC PRIVATE KEY-----\n";
/// The JWK coordinates of `KEY`, computed outside this crate: OpenSSL 3.5.7
/// derived the point from the private scalar alone (`openssl ec -no_public`
/// dropped the stated point, `openssl pkey -pubout` derived it again), and
/// Python's `base64.urlsafe_b64encode`, padding stripped, spelled each
/// coordinate.
const KEY_X: &str = "9qhj-KtcdHj1kVgwxWWWw--tqoh7H7UHs7oXh8jBbgE";
const KEY_Y: &str = "eO662BgvrXY4iGhyt3Qb73ROzBo-R7Hqcy5tyo_dW6g";
/// `KEY` as PKCS#8 (`openssl pkcs8 -topk8 -nocrypt`).
const KEY_PKCS8: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgUNkdTM4bU/Or7oXS\naMyU5KnWzI30tE8G4cygIVdNxeGhRANCAAT2qGP4q1x0ePWRWDDFZZbD762qiHsf\ntQezuheHyMFuAXjuutgYL612OIhocrd0G+90TswaPkex6nMubcqP3Vuo\n-----END PRIVATE KEY-----\n";
/// `KEY` without its public point (`openssl ec -no_public`).
const KEY_WITHOUT_POINT: &str = "-----BEGIN EC PRIVATE KEY-----\nMDECAQEEIFDZHUzOG1Pzq+6F0mjMlOSp1syN9LRPBuHMoCFXTcXhoAoGCCqGSM49\nAwEH\n-----END EC PRIVATE KEY-----\n";
/// aws-sdk-rust's `DPoP` test key, and the coordinates its own test pins: a
/// second outside implementation, the `p256` crate, computed them.
const SDK_KEY: &str = "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIBMB/RwQERsVoqWRQG4zK8CnaAa5dfrpbm+9tFdBh3z4oAoGCCqGSM49\nAwEHoUQDQgAEWb1VLi1EA2hJaTz4yYuxSELvY+1GAfL+8rUTCAdiFid87Bf6GY+s\n2+1RpqDv0RpZiDIMCrZrsAh+RK9S3QCaGA==\n-----END EC PRIVATE KEY-----\n";
const SDK_X: &str = "Wb1VLi1EA2hJaTz4yYuxSELvY-1GAfL-8rUTCAdiFic";
const SDK_Y: &str = "fOwX-hmPrNvtUaag79EaWYgyDAq2a7AIfkSvUt0Amhg";
/// A secp256k1 key - P-256's sizes, only its curve named apart (`openssl
/// ecparam -name secp256k1 -genkey -noout`) - and the same key as PKCS#8.
const SECP256K1_KEY: &str = "-----BEGIN EC PRIVATE KEY-----\nMHQCAQEEIEs/OQsfBGY6DXoBKZFrtXYfbLZqrcJCWn+gDjvafKG0oAcGBSuBBAAK\noUQDQgAEgdy0vYdbj10boQju64dQTQQoL7E+XLtg7aO/zS5+I7H8msbmeivwiaFr\nm+0nlDQP7+NgZDygGU82SiQhL/Vfew==\n-----END EC PRIVATE KEY-----\n";
const SECP256K1_PKCS8: &str = "-----BEGIN PRIVATE KEY-----\nMIGEAgEAMBAGByqGSM49AgEGBSuBBAAKBG0wawIBAQQgSz85Cx8EZjoNegEpkWu1\ndh9stmqtwkJaf6AOO9p8obShRANCAASB3LS9h1uPXRuhCO7rh1BNBCgvsT5cu2Dt\no7/NLn4jsfyaxuZ6K/CJoWub7SeUNA/v42BkPKAZTzZKJCEv9V97\n-----END PRIVATE KEY-----\n";

/// Every secret a sign-in holds or a refresh hands back - the key's body
/// and its armour among them. No error, log line or `Debug` rendering
/// carries one.
const SECRETS: [&str; 9] = [
    CACHED_SECRET,
    CACHED_SESSION_TOKEN,
    CACHED_REFRESH_TOKEN,
    CACHED_ID_TOKEN,
    LOGIN_SECRET_KEY,
    LOGIN_SESSION_TOKEN,
    LOGIN_REFRESH_TOKEN,
    "MHcCAQEEIFDZHUzOG1Pzq",
    "-----BEGIN",
];

fn assert_no_secret(text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{secret} rendered in {text}");
    }
}

fn now_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_secs();
    i64::try_from(seconds).expect("a representable instant")
}

fn seconds_of(instant: SystemTime) -> i64 {
    let seconds = instant
        .duration_since(UNIX_EPOCH)
        .expect("an instant after the epoch")
        .as_secs();
    i64::try_from(seconds).expect("a representable instant")
}

/// A sign-in as `aws login` files it, its set lapsing `expires_in` seconds
/// from now, beside a field no CLI writes yet.
fn signed_in(expires_in: i64) -> Value {
    json!({
        "accessToken": {
            "accessKeyId": CACHED_ACCESS_KEY,
            "secretAccessKey": CACHED_SECRET,
            "sessionToken": CACHED_SESSION_TOKEN,
            "accountId": ACCOUNT,
            "expiresAt": iso8601(now_seconds() + expires_in),
        },
        "tokenType": "aws_sigv4",
        "refreshToken": CACHED_REFRESH_TOKEN,
        "idToken": CACHED_ID_TOKEN,
        "clientId": CLIENT,
        "dpopKey": KEY,
        "laterField": {"kept": true},
    })
}

/// A cache directory of the test's own, and the file `key` names in it -
/// spelled from the pinned hash, never by the crate.
fn cache(name: &str, key: &str) -> (PathBuf, PathBuf) {
    let directory = scratch(name).join("login").join("cache");
    let file = directory.join(format!("{key}.json"));
    (directory, file)
}

/// `document` filed under `key` in a cache directory of the test's own.
fn filed(name: &str, key: &str, document: &Value) -> (PathBuf, PathBuf) {
    let (directory, file) = cache(name, key);
    std::fs::create_dir_all(&directory).expect("a cache directory");
    std::fs::write(&file, document.to_string()).expect("a filed sign-in");
    (directory, file)
}

fn read(file: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(file).expect("a filed sign-in")).expect("JSON")
}

/// The set the sign-in `session` filed under `directory` answers now, a
/// refresh sent to `identity`.
fn sign(identity: &Identity, directory: &Path, session: &str) -> yggdryl::Result<Credentials> {
    credentials(
        &identity.endpoint(),
        directory,
        session,
        PROFILE,
        SystemTime::now(),
    )
}

fn decode(part: &str) -> Value {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).expect("base64url")).expect("JSON")
}

/// The header and the claims of the proof a recorded refresh carried.
fn proof_of(request: &Recorded) -> (Value, Value) {
    let proof = request.header("dpop").expect("a DPoP header");
    let parts: Vec<&str> = proof.split('.').collect();
    let [header, claims, _] = parts[..] else {
        panic!("a compact JWS, got {proof}");
    };
    (decode(header), decode(claims))
}

/// The header the proofs `KEY` signs carry.
fn key_header() -> Value {
    json!({
        "typ": "dpop+jwt",
        "alg": "ES256",
        "jwk": {"kty": "EC", "x": KEY_X, "y": KEY_Y, "crv": "P-256"},
    })
}

/// The records the crate logs under `yggdryl.aws.login`, at `level` and
/// above, while this is held. The tree is the process's, so this holds it.
struct Logged {
    collect: Arc<Collect>,
    _tree: MutexGuard<'static, ()>,
}

impl Logged {
    fn at(level: Level) -> Self {
        let tree = serial();
        // Nothing else in this binary configures the tree: a record nobody
        // collects is dropped, as it was before the tree was installed,
        // rather than written to standard error by the last resort.
        logging::set_last_resort(None);
        logging::install().expect("the tree is the facade's backend");
        let logger = logging::get_logger(LOGGER);
        logger.set_level(level);
        let collect = Collect::shared();
        logger.add_handler(collect.clone());
        Self {
            collect,
            _tree: tree,
        }
    }

    /// The records kept so far about the sign-in `session`.
    fn about(&self, session: &str) -> Vec<Kept> {
        self.collect
            .take()
            .into_iter()
            .filter(|kept| kept.line.contains(session))
            .collect()
    }
}

impl Drop for Logged {
    fn drop(&mut self) {
        let logger = logging::get_logger(LOGGER);
        let handler: Arc<dyn Handler> = self.collect.clone();
        logger.remove_handler(&handler);
        logger.set_level(Level::NOTSET);
    }
}

/// An endpoint that accepts `times` connections and closes each unanswered.
fn hanging_up(times: usize) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a loopback listener");
    let address = listener.local_addr().expect("a bound address");
    std::thread::spawn(move || {
        for connection in listener.incoming().take(times) {
            drop(connection);
        }
    });
    format!("http://{address}")
}

#[test]
fn the_cache_file_is_the_sha256_of_the_session() {
    let directory = Path::new("cache");
    let filed = directory.join(format!("{SESSION_KEY}.json"));
    assert_eq!(cache_path(directory, SESSION), filed);
    // The AWS tools trim a profile's value; the file is the trimmed session's.
    assert_eq!(cache_path(directory, &format!("  {SESSION}\t")), filed);
    assert_eq!(
        cache_path(directory, OTHER_SESSION),
        directory.join(format!("{OTHER_SESSION_KEY}.json"))
    );
}

#[test]
fn the_service_is_the_regional_one_of_the_partition() {
    // `signin-rules.json`'s regional, single-stack form for an operation
    // that is not the control plane's.
    for (region, expected) in [
        ("eu-west-3", "https://eu-west-3.signin.aws.amazon.com"),
        ("us-east-1", "https://us-east-1.signin.aws.amazon.com"),
        ("cn-north-1", "https://cn-north-1.signin.amazonaws.cn"),
        (
            "us-gov-west-1",
            "https://us-gov-west-1.signin.amazonaws-us-gov.com",
        ),
        (
            "us-iso-east-1",
            "https://us-iso-east-1.signin.c2shome.ic.gov",
        ),
        (
            "us-isob-east-1",
            "https://us-isob-east-1.signin.sc2shome.sgov.gov",
        ),
        (
            "us-isof-south-1",
            "https://us-isof-south-1.signin.csphome.hci.ic.gov",
        ),
        (
            "eu-isoe-west-1",
            "https://eu-isoe-west-1.signin.csphome.adc-e.uk",
        ),
        (
            "eusc-de-east-1",
            "https://eusc-de-east-1.signin.amazonaws-eusc.eu",
        ),
        // A region no other partition claims is in `aws`.
        ("mars-north-1", "https://mars-north-1.signin.aws.amazon.com"),
        (" eu-west-1 ", "https://eu-west-1.signin.aws.amazon.com"),
    ] {
        assert_eq!(
            endpoint(region, false, false).as_deref(),
            Ok(expected),
            "{region}"
        );
    }
}

#[test]
fn the_fips_and_dual_stack_hosts_are_the_rules_own_and_a_region_chooses_no_other_host() {
    // `signin-rules.json` for an operation that is not the control plane's.
    for (region, fips, dualstack, expected) in [
        (
            "eu-west-3",
            true,
            false,
            "https://signin-fips.eu-west-3.amazonaws.com",
        ),
        ("eu-west-3", false, true, "https://signin.eu-west-3.api.aws"),
        (
            "eu-west-3",
            true,
            true,
            "https://signin-fips.eu-west-3.api.aws",
        ),
        (
            "us-gov-west-1",
            true,
            false,
            "https://signin-fips.amazonaws-us-gov.com",
        ),
        (
            "us-gov-east-1",
            true,
            false,
            "https://us-gov-east-1.signin-fips.amazonaws-us-gov.com",
        ),
        (
            "us-gov-east-1",
            false,
            true,
            "https://signin.us-gov-east-1.api.aws",
        ),
        (
            "cn-north-1",
            true,
            false,
            "https://signin-fips.cn-north-1.amazonaws.com.cn",
        ),
        (
            "cn-north-1",
            false,
            true,
            "https://signin.cn-north-1.api.amazonwebservices.com.cn",
        ),
    ] {
        assert_eq!(
            endpoint(region, fips, dualstack).as_deref(),
            Ok(expected),
            "{region} fips={fips} dualstack={dualstack}"
        );
    }
    assert_eq!(
        endpoint("us-iso-east-1", true, false).as_deref(),
        Ok("https://signin-fips.us-iso-east-1.c2s.ic.gov"),
        "every partition states a FIPS suffix"
    );
    // The one region rule every AWS host is built under, naming the session
    // as where the region came from: digits alone and a label past 63 bytes
    // are refused here as everywhere else.
    let long = "a".repeat(64);
    for region in [
        "x@evil.example#",
        "eu-west-3.evil.example",
        "",
        "  ",
        "-eu",
        "eu-",
        "a/b",
        "123",
        long.as_str(),
    ] {
        let refused = endpoint(region, false, false).expect_err("no host label");
        assert!(
            refused.contains("no Sign-In service")
                && refused.contains("one host label")
                && refused.contains("the session"),
            "{region:?}: {refused}"
        );
    }
    // 63 bytes is one label still.
    let longest = format!("a{}", "-b".repeat(31));
    assert_eq!(longest.len(), 63);
    assert_eq!(
        endpoint(&longest, false, false).as_deref(),
        Ok(format!("https://{longest}.signin.aws.amazon.com").as_str())
    );
}

#[test]
fn a_set_that_lasts_is_answered_with_no_request() {
    let identity = Identity::start();
    let document = signed_in(3600);
    let (directory, file) = filed("login-lasts", SESSION_KEY, &document);

    let found = sign(&identity, &directory, SESSION).expect("the cached set");
    assert_eq!(found.access_key_id(), CACHED_ACCESS_KEY);
    assert_eq!(found.session_token(), Some(CACHED_SESSION_TOKEN));
    assert_eq!(found.account_id(), Some(ACCOUNT));
    let lapses = found.expires_at().expect("a sign-in's set lapses");
    assert_eq!(
        iso8601(seconds_of(lapses)),
        document["accessToken"]["expiresAt"]
    );
    assert_eq!(identity.request_count(), 0);
    assert_eq!(read(&file), document);
}

#[test]
fn the_window_is_five_minutes() {
    assert_eq!(REFRESH_WINDOW, Duration::from_secs(5 * 60));
    let identity = Identity::start();

    // A minute beyond the window: the cached set, and nothing asked.
    let (directory, _) = filed("login-window-beyond", SESSION_KEY, &signed_in(360));
    let found = sign(&identity, &directory, SESSION).expect("the cached set");
    assert_eq!(found.access_key_id(), CACHED_ACCESS_KEY);
    assert_eq!(identity.request_count(), 0);

    // A minute inside it: one refresh, though the set still stands.
    let (directory, _) = filed("login-window-inside", SESSION_KEY, &signed_in(240));
    let found = sign(&identity, &directory, SESSION).expect("a refreshed set");
    assert_eq!(found.access_key_id(), LOGIN_ACCESS_KEY);
    assert_eq!(identity.request_count(), 1);
}

#[test]
fn a_set_inside_the_window_is_refreshed_by_one_proven_request() {
    let identity = Identity::start();
    identity.require_login_refresh_token(Some(CACHED_REFRESH_TOKEN));
    let document = signed_in(240);
    let (directory, file) = filed("login-refresh", SESSION_KEY, &document);

    let asked = now_seconds();
    let found = sign(&identity, &directory, SESSION).expect("a refreshed set");
    let answered = now_seconds();
    assert_eq!(found.access_key_id(), LOGIN_ACCESS_KEY);
    assert_eq!(found.session_token(), Some(LOGIN_SESSION_TOKEN));
    // A refresh states no account: the sign-in's is carried over.
    assert_eq!(found.account_id(), Some(ACCOUNT));
    let lapses = seconds_of(found.expires_at().expect("a sign-in's set lapses"));
    assert!(
        (asked + LOGIN_EXPIRES_IN..=answered + LOGIN_EXPIRES_IN).contains(&lapses),
        "{lapses} is not {LOGIN_EXPIRES_IN} seconds after the refresh"
    );

    let [request] = identity.requests().try_into().expect("one request");
    assert_eq!(
        (
            request.method.as_str(),
            request.path.as_str(),
            request.status
        ),
        ("POST", "/v1/token", 200),
        "the fake verified the proof, or said why not"
    );
    // Not signed: the proof stands in for a signature.
    assert_eq!(request.header("authorization"), None);
    assert_eq!(request.header("content-type"), Some("application/json"));
    let body: Value = serde_json::from_str(&request.body).expect("a JSON body");
    assert_eq!(
        body,
        json!({
            "clientId": CLIENT,
            "grantType": "refresh_token",
            "refreshToken": CACHED_REFRESH_TOKEN,
        })
    );
    let (header, claims) = proof_of(&request);
    assert_eq!(header, key_header());
    assert_eq!(claims["htm"], "POST");
    assert_eq!(claims["htu"], format!("{}/v1/token", identity.endpoint()));

    // The new set and the rotated token replace the old; every other field
    // - the key, the client, the identity token, the token type, the field
    // no CLI writes yet - is filed back as it was read.
    let mut expected = document.clone();
    expected["accessToken"] = json!({
        "accessKeyId": LOGIN_ACCESS_KEY,
        "secretAccessKey": LOGIN_SECRET_KEY,
        "sessionToken": LOGIN_SESSION_TOKEN,
        "accountId": ACCOUNT,
        "expiresAt": iso8601(lapses),
    });
    expected["refreshToken"] = json!(LOGIN_REFRESH_TOKEN);
    assert_eq!(read(&file), expected);

    // Read again, the refreshed document is the set: nothing more is asked.
    let again = sign(&identity, &directory, SESSION).expect("the refreshed set, filed");
    assert_eq!(again, found);
    assert_eq!(identity.request_count(), 1);
}

#[test]
fn an_ended_sign_in_is_a_refusal_naming_aws_login() {
    for code in ["TOKEN_EXPIRED", "USER_CREDENTIALS_CHANGED"] {
        let identity = Identity::start();
        identity.refuse_login(401, code, 1);
        let document = signed_in(-60);
        let (directory, file) = filed(&format!("login-ended-{code}"), SESSION_KEY, &document);

        let error = sign(&identity, &directory, SESSION).expect_err("a lapsed set, refused");
        let message = error.to_string();
        assert!(message.contains(&format!("401 {code}")), "{message}");
        assert!(message.contains(SESSION), "{message}");
        assert!(message.contains(WAY_OUT), "{message}");
        assert!(message.contains("refused as scripted"), "{message}");
        assert_no_secret(&message);
        // The service's verdict is not tried again, and the CLI's document
        // is left as it was.
        assert_eq!(identity.request_count(), 1);
        assert_eq!(read(&file), document);
    }
}

#[test]
fn a_missing_permission_is_named() {
    let identity = Identity::start();
    identity.refuse_login(403, "INSUFFICIENT_PERMISSIONS", 1);
    let (directory, _) = filed("login-permission", SESSION_KEY, &signed_in(-60));

    let error = sign(&identity, &directory, SESSION).expect_err("a refused refresh");
    let message = error.to_string();
    assert!(
        message.contains("403 INSUFFICIENT_PERMISSIONS"),
        "{message}"
    );
    assert!(message.contains("signin:CreateOAuth2Token"), "{message}");
    assert!(message.contains(SESSION), "{message}");
    assert!(message.contains(WAY_OUT), "{message}");
    assert_no_secret(&message);
}

#[test]
fn a_throttle_or_a_server_failure_is_tried_again_with_a_fresh_proof() {
    let identity = Identity::start();
    identity.fail_next(500, 2);
    let (directory, _) = filed("login-server-failure", SESSION_KEY, &signed_in(-60));
    let found = sign(&identity, &directory, SESSION).expect("the third attempt's set");
    assert_eq!(found.access_key_id(), LOGIN_ACCESS_KEY);
    let statuses: Vec<u16> = identity
        .requests()
        .iter()
        .map(|request| request.status)
        .collect();
    assert_eq!(statuses, [500, 500, 200]);

    let identity = Identity::start();
    identity.refuse_login(429, "INVALID_REQUEST", 1);
    let (directory, _) = filed("login-throttle", SESSION_KEY, &signed_in(-60));
    let found = sign(&identity, &directory, SESSION).expect("the second attempt's set");
    assert_eq!(found.access_key_id(), LOGIN_ACCESS_KEY);
    let requests = identity.requests();
    let statuses: Vec<u16> = requests.iter().map(|request| request.status).collect();
    assert_eq!(statuses, [429, 200]);
    // The fake refuses an identifier it has seen, and each attempt was
    // accepted: each carried its own.
    let identifiers: Vec<Value> = requests
        .iter()
        .map(|request| proof_of(request).1["jti"].clone())
        .collect();
    assert_ne!(identifiers[0], identifiers[1]);
}

#[test]
fn a_refresh_failing_while_the_set_stands_keeps_it_and_says_so() {
    let logged = Logged::at(Level::WARNING);
    let identity = Identity::start();
    identity.fail_next(503, 3);
    let document = signed_in(120);
    let (directory, file) = filed("login-kept", OTHER_SESSION_KEY, &document);

    let found = sign(&identity, &directory, OTHER_SESSION).expect("the set in hand");
    assert_eq!(found.access_key_id(), CACHED_ACCESS_KEY);
    assert_eq!(found.account_id(), Some(ACCOUNT));
    assert_eq!(identity.request_count(), 3);
    assert_eq!(read(&file), document);

    let [warning] = logged
        .about(OTHER_SESSION)
        .try_into()
        .expect("one warning about the sign-in");
    assert_eq!(warning.level, Level::WARNING);
    assert_eq!(warning.name, LOGGER);
    assert_eq!(warning.target, "yggdryl::aws::login");
    assert!(
        warning.line.starts_with(&format!(
            "keeping the AWS Console sign-in {OTHER_SESSION} in hand, which stands until {}",
            document["accessToken"]["expiresAt"].as_str().expect("text")
        )),
        "{}",
        warning.line
    );
    assert!(warning.line.contains("503"), "{}", warning.line);
    assert_no_secret(&warning.line);
}

#[test]
fn a_refresh_says_when_the_new_set_lapses_and_nothing_secret() {
    let logged = Logged::at(Level::DEBUG);
    let identity = Identity::start();
    let (directory, _) = filed("login-logged", OTHER_SESSION_KEY, &signed_in(60));

    let found = sign(&identity, &directory, OTHER_SESSION).expect("a refreshed set");
    let lapses = seconds_of(found.expires_at().expect("a sign-in's set lapses"));
    let [record] = logged
        .about(OTHER_SESSION)
        .try_into()
        .expect("one record about the sign-in");
    assert_eq!(record.level, Level::DEBUG);
    assert_eq!(
        record.line,
        format!(
            "refreshed the AWS Console sign-in {OTHER_SESSION}, which now stands until {}",
            iso8601(lapses)
        )
    );
    assert_no_secret(&record.line);
}

#[test]
fn a_lapsed_set_whose_refresh_fails_is_a_refusal() {
    let identity = Identity::start();
    identity.fail_next(503, 3);
    let (directory, _) = filed("login-lapsed", SESSION_KEY, &signed_in(-60));

    let error = sign(&identity, &directory, SESSION).expect_err("nothing stands");
    let message = error.to_string();
    assert!(message.contains("503"), "{message}");
    assert!(message.contains(SESSION), "{message}");
    assert!(message.contains(WAY_OUT), "{message}");
    assert_no_secret(&message);
    assert_eq!(identity.request_count(), 3);
}

#[test]
fn a_service_that_cannot_be_reached_is_named() {
    let (directory, _) = filed("login-unreached", SESSION_KEY, &signed_in(-60));
    let unreached = hanging_up(3);

    let error = credentials(&unreached, &directory, SESSION, PROFILE, SystemTime::now())
        .expect_err("no answer");
    let message = error.to_string();
    assert!(
        message.contains(&format!(
            "could not reach the AWS Sign-In service at {unreached}"
        )),
        "{message}"
    );
    assert!(message.contains(SESSION), "{message}");
    assert!(message.contains(WAY_OUT), "{message}");
    assert_no_secret(&message);
}

#[test]
fn no_sign_in_filed_is_a_refusal_naming_the_file_and_aws_login() {
    let identity = Identity::start();
    let (directory, file) = cache("login-missing", SESSION_KEY);

    let error = sign(&identity, &directory, SESSION).expect_err("nothing filed");
    let message = error.to_string();
    assert!(message.contains(SESSION), "{message}");
    assert!(
        message.contains(&format!("is not filed at {}", file.display())),
        "{message}"
    );
    assert!(message.contains(WAY_OUT), "{message}");
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn a_document_that_is_not_one_is_named() {
    let identity = Identity::start();
    for (name, text, expected) in [
        ("login-truncated", "{\"accessToken\": {", "is not JSON"),
        ("login-array", "[]", "is not a JSON object"),
    ] {
        let (directory, file) = cache(name, SESSION_KEY);
        std::fs::create_dir_all(&directory).expect("a cache directory");
        std::fs::write(&file, text).expect("a filed document");
        let error = sign(&identity, &directory, SESSION).expect_err(expected);
        let message = error.to_string();
        assert!(message.contains(expected), "{message}");
        assert!(message.contains(&file.display().to_string()), "{message}");
        assert!(message.contains(WAY_OUT), "{message}");
    }
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn each_field_a_refresh_needs_is_required_and_named() {
    let path = Path::new("cache").join(format!("{SESSION_KEY}.json"));
    let refused = |document: &Value, field: &str| {
        let error = parse(document.to_string().as_bytes(), SESSION, PROFILE, &path)
            .expect_err("a document lacking a field");
        let message = error.to_string();
        assert!(message.contains(&format!("lacks {field}")), "{message}");
        assert!(message.contains(SESSION), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains(WAY_OUT), "{message}");
        assert_no_secret(&message);
    };
    for field in ["accessToken", "refreshToken", "dpopKey", "clientId"] {
        let mut document = signed_in(3600);
        document.as_object_mut().expect("an object").remove(field);
        refused(&document, field);
    }
    for field in [
        "accessKeyId",
        "secretAccessKey",
        "sessionToken",
        "expiresAt",
    ] {
        let mut document = signed_in(3600);
        document["accessToken"]
            .as_object_mut()
            .expect("an object")
            .remove(field);
        refused(&document, &format!("accessToken.{field}"));
    }
    // An empty value is no value.
    let mut document = signed_in(3600);
    document["refreshToken"] = json!("");
    refused(&document, "refreshToken");

    // An expiry naming no instant is named as such.
    let mut document = signed_in(3600);
    document["accessToken"]["expiresAt"] = json!("soon");
    let error = parse(document.to_string().as_bytes(), SESSION, PROFILE, &path)
        .expect_err("an expiry that is no instant");
    assert!(
        error.to_string().contains("accessToken.expiresAt"),
        "{error}"
    );

    // The account is the one field a set signs without.
    let mut document = signed_in(3600);
    document["accessToken"]
        .as_object_mut()
        .expect("an object")
        .remove("accountId");
    let token = parse(document.to_string().as_bytes(), SESSION, PROFILE, &path)
        .expect("a set with no account");
    assert_eq!(token.credentials().account_id(), None);
}

#[test]
fn a_document_reads_and_renders_back_whole() {
    let path = Path::new("cache").join(format!("{SESSION_KEY}.json"));
    let document = signed_in(3600);
    let token =
        parse(document.to_string().as_bytes(), SESSION, PROFILE, &path).expect("a filed sign-in");
    assert_eq!(token.client_id(), CLIENT);
    assert_eq!(token.credentials().access_key_id(), CACHED_ACCESS_KEY);
    let rendered: Value = serde_json::from_str(&token.render()).expect("JSON");
    assert_eq!(rendered, document);

    // aws-sdk-rust's own sample document, whose expiry its test pins at
    // 1640467800; the key is not read until a refresh needs it.
    let sample = r#"{
        "accessToken": {
            "accessKeyId": "AKIAIOSFODNN7EXAMPLE",
            "secretAccessKey": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "sessionToken": "session-token",
            "accountId": "012345678901",
            "expiresAt": "2021-12-25T21:30:00Z"
        },
        "tokenType": "aws_sigv4",
        "refreshToken": "refresh-token-value",
        "idToken": "identity-token-value",
        "clientId": "arn:aws:signin:::devtools/same-device",
        "dpopKey": "-----BEGIN EC PRIVATE KEY-----\ntest\n-----END EC PRIVATE KEY-----\n"
    }"#;
    let token = parse(sample.as_bytes(), SESSION, PROFILE, &path).expect("the SDK's sample");
    assert_eq!(
        token.credentials().expires_at(),
        Some(UNIX_EPOCH + Duration::from_secs(1_640_467_800))
    );
    assert_eq!(token.credentials().account_id(), Some("012345678901"));
}

#[test]
fn debug_renders_no_secret() {
    let path = Path::new("cache").join(format!("{SESSION_KEY}.json"));
    let token = parse(
        signed_in(3600).to_string().as_bytes(),
        SESSION,
        PROFILE,
        &path,
    )
    .expect("a filed sign-in");
    let rendered = format!("{token:?}");
    assert_no_secret(&rendered);
    assert!(rendered.contains(CACHED_ACCESS_KEY), "{rendered}");
    assert!(rendered.contains(CLIENT), "{rendered}");

    let identity = Identity::start();
    let (directory, _) = filed("login-debug", SESSION_KEY, &signed_in(-60));
    let found = sign(&identity, &directory, SESSION).expect("a refreshed set");
    let rendered = format!("{found:?}");
    assert_no_secret(&rendered);
    assert!(rendered.contains(LOGIN_ACCESS_KEY), "{rendered}");
}

#[test]
fn a_key_the_proof_cannot_be_signed_with_is_a_refusal_before_any_request() {
    let identity = Identity::start();
    let mut document = signed_in(-60);
    document["dpopKey"] = json!(KEY_WITHOUT_POINT);
    let (directory, _) = filed("login-pointless", SESSION_KEY, &document);

    let error = sign(&identity, &directory, SESSION).expect_err("no proof can be signed");
    let message = error.to_string();
    assert!(
        message.contains("holds a DPoP key that states no public point"),
        "{message}"
    );
    assert!(message.contains(WAY_OUT), "{message}");
    assert_no_secret(&message);
    assert_eq!(identity.request_count(), 0);
}

#[test]
fn the_jwk_is_the_point_outside_implementations_derive() {
    let expected = (KEY_X.to_owned(), KEY_Y.to_owned());
    assert_eq!(jwk(KEY), Ok(expected.clone()));
    assert_eq!(jwk(SDK_KEY), Ok((SDK_X.to_owned(), SDK_Y.to_owned())));
    // PKCS#8 holds the same pair.
    assert_eq!(jwk(KEY_PKCS8), Ok(expected.clone()));
    // A document escaped twice leaves a literal `\n`, read as the break it
    // meant.
    assert_eq!(jwk(&KEY.replace('\n', "\\n")), Ok(expected));
}

#[test]
fn a_key_without_its_point_is_refused() {
    let cause = jwk(KEY_WITHOUT_POINT).expect_err("signing needs the point");
    assert_eq!(cause, "states no public point");
}

#[test]
fn a_key_on_another_curve_is_refused() {
    let cause = jwk(SECP256K1_KEY).expect_err("a secp256k1 key");
    assert_eq!(
        cause,
        "is on the curve 1.3.132.0.10, not on P-256 (1.2.840.10045.3.1.7)"
    );
    let cause = jwk(SECP256K1_PKCS8).expect_err("a secp256k1 PKCS#8 key");
    assert!(
        cause.starts_with("is not a PKCS#8 P-256 key pair"),
        "{cause}"
    );
}

#[test]
fn a_key_that_is_not_one_is_refused_by_name() {
    for (pem, expected) in [
        ("not a key", "is not PEM: it has no BEGIN line"),
        (
            "-----BEGIN PUBLIC KEY-----\nMFkw\n-----END PUBLIC KEY-----\n",
            "is a PEM PUBLIC KEY, where an EC PRIVATE KEY or a PRIVATE KEY is expected",
        ),
        (
            "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEE\n",
            "is a PEM EC PRIVATE KEY with no END line",
        ),
        (
            "-----BEGIN EC PRIVATE KEY-----\n!!!!\n-----END EC PRIVATE KEY-----\n",
            "is a PEM EC PRIVATE KEY whose body is not base64",
        ),
        // A sequence that states more bytes than follow it.
        (
            "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIFDZ\n-----END EC PRIVATE KEY-----\n",
            "is not a DER ECPrivateKey",
        ),
    ] {
        assert_eq!(jwk(pem).expect_err(expected), expected);
    }
}

#[test]
fn a_proof_is_a_compact_jws_over_es256_naming_its_request() {
    let url = "https://eu-west-3.signin.aws.amazon.com/v1/token";
    let identifier = "0b0f2f27-4a54-4b21-8a5c-6f1e0d2c9a10";
    let proof = proof(KEY, url, 1_800_000_000, identifier).expect("a proof");
    // base64url as JWS spells it: no padding anywhere.
    assert!(!proof.contains('='), "{proof}");
    let parts: Vec<&str> = proof.split('.').collect();
    let [header, claims, signature] = parts[..] else {
        panic!("a compact JWS, got {proof}");
    };
    assert_eq!(decode(header), key_header());
    assert_eq!(
        decode(claims),
        json!({"htm": "POST", "htu": url, "iat": 1_800_000_000, "jti": identifier})
    );
    // ES256 is r || s, 32 bytes each, verified here under the point the
    // pinned coordinates state rather than the one the proof carries.
    let signature = URL_SAFE_NO_PAD.decode(signature).expect("base64url");
    assert_eq!(signature.len(), 64);
    let point = [
        vec![0x04],
        URL_SAFE_NO_PAD.decode(KEY_X).expect("x"),
        URL_SAFE_NO_PAD.decode(KEY_Y).expect("y"),
    ]
    .concat();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, &point)
        .verify(format!("{header}.{claims}").as_bytes(), &signature)
        .expect("the signature verifies");
}

#[test]
fn a_proof_identifier_is_a_fresh_version_4_uuid() {
    let first = jti().expect("an identifier");
    let second = jti().expect("an identifier");
    assert_ne!(first, second);
    for identifier in [first, second] {
        let parsed = yggdryl::Uuid::from_bytes(identifier.as_bytes()).expect("a UUID");
        assert_eq!(parsed.version(), 4);
        assert_eq!((parsed.get() >> 62) & 0b11, 0b10, "the RFC 9562 variant");
        assert_eq!(parsed.to_string(), identifier, "the canonical spelling");
    }
}
