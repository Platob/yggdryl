#![allow(dead_code)]
//! `rust/tests/support/identity.rs`: the in-process fake of every AWS
//! identity endpoint the `aws` module speaks to.
//!
//! One loopback listener answers as all of them, so a session pointed at it
//! with `Session::with_endpoint_url` and `Session::with_metadata_endpoint`
//! walks its whole chain over a real socket: STS (`AssumeRole`,
//! `AssumeRoleWithWebIdentity`), the IAM Identity Center OIDC service
//! (`RegisterClient`, `StartDeviceAuthorization`, `CreateToken`) and access
//! portal (`GetRoleCredentials`), the AWS Sign-In service's refresh of an
//! `aws login` sign-in (`CreateOAuth2Token`, whose `DPoP` proof it verifies
//! itself), the container credential endpoint, and the instance metadata
//! service (IMDSv2 token, role listing, role keys, the identity document).
//! Every request is recorded so a test can count and
//! inspect what went on the wire, and every answer is scripted by the
//! setters below. It is a leaf file included with `#[path]`, so it names
//! nothing of the crate.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The bearer token the portal accepts until a test says otherwise.
pub const SSO_TOKEN: &str = "sso-access-token";
/// The refresh token a device sign-in hands back.
pub const SSO_REFRESH_TOKEN: &str = "sso-refresh-token";
/// The IMDSv2 session token the service issues.
pub const IMDS_TOKEN: &str = "imds-session-token";
/// The container endpoint's path.
pub const CONTAINER_PATH: &str = "/v2/credentials/task";
/// The set the Sign-In service answers a refresh with.
pub const LOGIN_ACCESS_KEY: &str = "ASIALOGINREFRESHED";
pub const LOGIN_SECRET_KEY: &str = "login-refreshed-secret";
pub const LOGIN_SESSION_TOKEN: &str = "login-refreshed-session-token";
/// The refresh token a refresh rotates to.
pub const LOGIN_REFRESH_TOKEN: &str = "login-rotated-refresh-token";
/// How long a refreshed set lasts, in seconds, as the service states it.
pub const LOGIN_EXPIRES_IN: i64 = 900;
/// The most a proof's `iat` may differ from the fake's clock, in seconds.
const DPOP_SKEW: i64 = 60;

/// One request the server handled, as a test inspects it.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    /// The path without the query.
    pub path: String,
    /// The query, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// Headers with lowercase names, in wire order.
    pub headers: Vec<(String, String)>,
    /// The body as text.
    pub body: String,
    /// The pairs of a form body - an STS exchange's parameters -
    /// percent-decoded, in wire order; empty where the body is no form.
    pub form: Vec<(String, String)>,
    pub status: u16,
}

impl Recorded {
    /// One parameter: of the URL's query, else of the form body the
    /// exchange carried it in.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .chain(self.form.iter())
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// One header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether this is an STS exchange for `action`: the action in the
    /// query, or in the form body the exchange carries it in.
    pub fn is_sts(&self, action: &str) -> bool {
        self.query("Action") == Some(action)
            || form_param(&self.body, self.header("content-type"), "Action").as_deref()
                == Some(action)
    }
}

/// One parameter of a form body - `application/x-www-form-urlencoded`, as
/// an STS exchange carries its parameters - percent-decoded; `None` where
/// the body is no form or holds no such name.
fn form_param(body: &str, content_type: Option<&str>, name: &str) -> Option<String> {
    form_pairs(body, content_type)
        .into_iter()
        .find(|(held, _)| held == name)
        .map(|(_, value)| value)
}

/// The pairs of a form body, percent-decoded, in wire order; empty where
/// the body is no form.
fn form_pairs(body: &str, content_type: Option<&str>) -> Vec<(String, String)> {
    if !content_type.is_some_and(|value| value.starts_with("application/x-www-form-urlencoded")) {
        return Vec::new();
    }
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (percent_decode(name), percent_decode(value)))
        .collect()
}

/// What the scripted answers say.
struct Script {
    /// `AssumeRole` answers sessions that have already lapsed.
    expired_sessions: bool,
    /// `AssumeRole` refuses unless `TokenCode` is this.
    token_code: Option<String>,
    /// `AssumeRoleWithWebIdentity` refuses unless the token is this.
    web_identity_token: Option<String>,
    /// The next STS exchanges answer 403 with this code.
    sts_refusals: Option<(String, usize)>,
    /// The bearer token the portal accepts, and the one a sign-in or a
    /// refresh hands back.
    sso_token: String,
    /// Device polls still to answer `authorization_pending`.
    sso_pending: usize,
    /// The refresh token a refresh must present, when checked.
    sso_refresh_token: Option<String>,
    /// The refresh token a Sign-In refresh must present, when checked.
    login_refresh_token: Option<String>,
    /// The next Sign-In refreshes answer this status and OAuth 2.0 error
    /// code, after their proof is verified.
    login_refusals: Option<(u16, String, usize)>,
    /// Every proof identifier the Sign-In service has accepted; one seen
    /// again is a replay, and refused.
    login_jtis: Vec<String>,
    /// The access key the container endpoint answers.
    container_access_key: String,
    /// When the container endpoint's set lapses.
    container_expiry: String,
    /// The `Authorization` the container endpoint requires.
    container_authorization: Option<String>,
    /// The role the instance carries; `None` is an instance without one.
    imds_role: Option<String>,
    /// Whether a read without the session token is refused.
    imds_v2_only: bool,
    /// The region the identity document states.
    imds_region: String,
    /// The expiry the instance role's credential document states, when
    /// pinned; else six hours from the answer.
    imds_expiry: Option<String>,
    /// Whether the portal's refusal names its error type in the
    /// `x-amzn-ErrorType` header alone, with no `error` in the body.
    portal_error_in_header: bool,
    /// The next requests, whichever endpoint, answer this status.
    failures: Option<(u16, usize)>,
    recorded: Vec<Recorded>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            expired_sessions: false,
            token_code: None,
            web_identity_token: None,
            sts_refusals: None,
            sso_token: SSO_TOKEN.to_owned(),
            sso_pending: 0,
            sso_refresh_token: None,
            login_refresh_token: None,
            login_refusals: None,
            login_jtis: Vec::new(),
            container_access_key: "ASIACONTAINER".to_owned(),
            container_expiry: iso8601(now_seconds() + 3600),
            container_authorization: None,
            imds_role: Some("instance-role".to_owned()),
            imds_v2_only: false,
            imds_region: "eu-west-3".to_owned(),
            imds_expiry: None,
            portal_error_in_header: false,
            failures: None,
            recorded: Vec::new(),
        }
    }
}

struct Inner {
    address: String,
    script: Mutex<Script>,
    stopping: AtomicBool,
}

impl Inner {
    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The fake: a loopback listener, its accept thread, and the script every
/// handled request reads. Dropping it stops the listener.
pub struct Identity {
    inner: Arc<Inner>,
    accept: Option<JoinHandle<()>>,
}

impl Identity {
    /// Bind `127.0.0.1:0` and start answering.
    ///
    /// # Panics
    ///
    /// When the loopback listener cannot be bound.
    pub fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind a loopback listener");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        let inner = Arc::new(Inner {
            address: address.to_string(),
            script: Mutex::new(Script::default()),
            stopping: AtomicBool::new(false),
        });
        let shared = Arc::clone(&inner);
        let accept = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = connection {
                    let inner = Arc::clone(&shared);
                    std::thread::spawn(move || serve(&inner, stream));
                }
            }
        });
        Self {
            inner,
            accept: Some(accept),
        }
    }

    /// `http://127.0.0.1:PORT`.
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.inner.address)
    }

    /// The container endpoint's full URI.
    pub fn container_uri(&self) -> String {
        format!("{}{CONTAINER_PATH}", self.endpoint())
    }

    /// The requests handled so far, in arrival order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.script().recorded.clone()
    }

    pub fn request_count(&self) -> usize {
        self.inner.script().recorded.len()
    }

    pub fn clear_requests(&self) {
        self.inner.script().recorded.clear();
    }

    /// Answer every assumed-role session as already lapsed, or as lasting.
    pub fn expire_sessions(&self, expired: bool) {
        self.inner.script().expired_sessions = expired;
    }

    /// Refuse `AssumeRole` unless it presents `code` as `TokenCode`.
    pub fn require_token_code(&self, code: Option<&str>) {
        self.inner.script().token_code = code.map(str::to_owned);
    }

    /// Refuse `AssumeRoleWithWebIdentity` unless it presents `token`.
    pub fn require_web_identity_token(&self, token: Option<&str>) {
        self.inner.script().web_identity_token = token.map(str::to_owned);
    }

    /// Answer the next `times` STS exchanges with 403 and `code`.
    pub fn refuse_sts(&self, code: &str, times: usize) {
        self.inner.script().sts_refusals = (times > 0).then(|| (code.to_owned(), times));
    }

    /// The bearer token the portal accepts from now on, and the one a
    /// sign-in or a refresh hands back.
    pub fn set_sso_token(&self, token: &str) {
        self.inner.script().sso_token = token.to_owned();
    }

    /// Answer the next `count` device polls with `authorization_pending`.
    pub fn sso_pending_polls(&self, count: usize) {
        self.inner.script().sso_pending = count;
    }

    /// Refuse a refresh unless it presents `token`.
    pub fn require_refresh_token(&self, token: Option<&str>) {
        self.inner.script().sso_refresh_token = token.map(str::to_owned);
    }

    /// Refuse a Sign-In refresh unless it presents `token`.
    pub fn require_login_refresh_token(&self, token: Option<&str>) {
        self.inner.script().login_refresh_token = token.map(str::to_owned);
    }

    /// Answer the next `times` Sign-In refreshes with `status` and the
    /// OAuth 2.0 error `code`, once their proof is verified.
    pub fn refuse_login(&self, status: u16, code: &str, times: usize) {
        self.inner.script().login_refusals = (times > 0).then(|| (status, code.to_owned(), times));
    }

    /// What the container endpoint answers: `access_key`, lapsing at
    /// `expiry` (ISO 8601).
    pub fn set_container_credentials(&self, access_key: &str, expiry: &str) {
        let mut script = self.inner.script();
        script.container_access_key = access_key.to_owned();
        script.container_expiry = expiry.to_owned();
    }

    /// Refuse the container endpoint unless `Authorization` is `token`.
    pub fn require_container_authorization(&self, token: Option<&str>) {
        self.inner.script().container_authorization = token.map(str::to_owned);
    }

    /// The role the instance carries, or none.
    pub fn set_imds_role(&self, role: Option<&str>) {
        self.inner.script().imds_role = role.map(str::to_owned);
    }

    /// Refuse a metadata read without the IMDSv2 session token.
    pub fn imds_v2_only(&self, only: bool) {
        self.inner.script().imds_v2_only = only;
    }

    /// The region the identity document states.
    pub fn set_imds_region(&self, region: &str) {
        self.inner.script().imds_region = region.to_owned();
    }

    /// The `Expiration` the instance role's credential document states
    /// from now on, ISO 8601 text, in place of six hours from each answer.
    pub fn set_imds_expiry(&self, expiry: &str) {
        self.inner.script().imds_expiry = Some(expiry.to_owned());
    }

    /// Have the portal refuse with its error type in `x-amzn-ErrorType`
    /// alone - `UnauthorizedException:http://internal.amazon.com/` - and a
    /// body holding the message and no `error`, as the real portal answers.
    pub fn portal_error_in_header(&self, in_header: bool) {
        self.inner.script().portal_error_in_header = in_header;
    }

    /// Answer the next `times` requests, whichever endpoint, with `status`
    /// and an empty body.
    pub fn fail_next(&self, status: u16, times: usize) {
        self.inner.script().failures = (times > 0).then_some((status, times));
    }
}

impl Drop for Identity {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(&self.inner.address);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

/// One parsed request.
struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: String,
}

impl Request {
    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// One parameter of the request: in the query, else in its form body.
    fn param(&self, name: &str) -> Option<String> {
        self.query(name)
            .map(str::to_owned)
            .or_else(|| form_param(&self.body, self.header("content-type"), name))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    /// One field of a JSON body.
    fn json(&self, name: &str) -> Option<String> {
        let document: serde_json::Value = serde_json::from_str(&self.body).ok()?;
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }
}

/// One answer.
struct Response {
    status: u16,
    content_type: &'static str,
    body: String,
    /// Headers beside the content type and length, as sent.
    headers: Vec<(&'static str, String)>,
}

impl Response {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
            headers: Vec::new(),
        }
    }

    fn xml(status: u16, body: String) -> Self {
        Self {
            status,
            content_type: "text/xml",
            body,
            headers: Vec::new(),
        }
    }

    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    /// The same answer carrying one more header.
    fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_owned()));
        self
    }

    fn sts_error(status: u16, code: &str, message: &str) -> Self {
        Self::xml(
            status,
            format!(
                "<ErrorResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>fake</RequestId></ErrorResponse>"
            ),
        )
    }
}

/// Serve one connection: one request, one answer, close.
fn serve(inner: &Inner, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let response = answer(inner, &request);
    let mut script = inner.script();
    script.recorded.push(Recorded {
        method: request.method.clone(),
        path: request.path.clone(),
        query: request.query.clone(),
        headers: request.headers.clone(),
        body: request.body.clone(),
        form: form_pairs(&request.body, request.header("content-type")),
        status: response.status,
    });
    drop(script);
    let extra: String = response
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
    let _ = stream.flush();
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    if length > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), parse_query(query)),
        None => (target, Vec::new()),
    };
    Some(Request {
        method,
        path,
        query,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(name), percent_decode(value))
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    None => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Route one request to the endpoint it addresses.
fn answer(inner: &Inner, request: &Request) -> Response {
    let mut script = inner.script();
    if let Some((status, times)) = script.failures {
        script.failures = (times > 1).then_some((status, times - 1));
        return Response::text(status, "");
    }
    if let Some(action) = request.param("Action") {
        return sts(&mut script, request, &action);
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/client/register") => Response::json(
            200,
            serde_json::json!({
                "clientId": "client-1",
                "clientSecret": "client-secret-1",
                "clientSecretExpiresAt": now_seconds() + 90 * 86_400,
            }),
        ),
        ("POST", "/device_authorization") => Response::json(
            200,
            serde_json::json!({
                "deviceCode": "device-1",
                "userCode": "ABCD-EFGH",
                "verificationUri": "https://device.sso.test/",
                "verificationUriComplete": "https://device.sso.test/?user_code=ABCD-EFGH",
                "expiresIn": 600,
                "interval": 0,
            }),
        ),
        ("POST", "/token") => oidc_token(&mut script, request),
        ("POST", "/v1/token") => signin_token(&mut script, &inner.address, request),
        ("GET", "/federation/credentials") => portal(&script, request),
        ("GET", CONTAINER_PATH) => container(&script, request),
        ("PUT", "/latest/api/token") => Response::text(200, IMDS_TOKEN),
        ("GET", path) if path.starts_with("/latest/") => imds(&script, request),
        _ => Response::text(404, "no such endpoint"),
    }
}

fn sts(script: &mut Script, request: &Request, action: &str) -> Response {
    if let Some((code, times)) = script.sts_refusals.clone() {
        script.sts_refusals = (times > 1).then(|| (code.clone(), times - 1));
        return Response::sts_error(403, &code, "refused as scripted");
    }
    let Some(role) = request.param("RoleArn").filter(|arn| !arn.is_empty()) else {
        return Response::sts_error(400, "ValidationError", "RoleArn must not be null");
    };
    let session = request.param("RoleSessionName").unwrap_or_default();
    let short = role.rsplit('/').next().unwrap_or(&role);
    match action {
        "AssumeRole" => {
            if request.header("authorization").is_none() {
                return Response::sts_error(403, "MissingAuthenticationToken", "unsigned");
            }
            if let Some(code) = &script.token_code
                && request.param("TokenCode").as_deref() != Some(code.as_str())
            {
                return Response::sts_error(
                    403,
                    "AccessDenied",
                    "MultiFactorAuthentication failed with invalid MFA one time pass code.",
                );
            }
        }
        "AssumeRoleWithWebIdentity" => {
            if request.header("authorization").is_some() {
                return Response::sts_error(400, "InvalidAction", "a signed web identity exchange");
            }
            if let Some(token) = &script.web_identity_token
                && request.param("WebIdentityToken").as_deref() != Some(token.as_str())
            {
                return Response::sts_error(
                    400,
                    "InvalidIdentityToken",
                    "the token is not the one the role trusts",
                );
            }
        }
        other => return Response::sts_error(400, "InvalidAction", other),
    }
    let expiry = if script.expired_sessions {
        "2000-01-01T00:00:00Z".to_owned()
    } else {
        iso8601(now_seconds() + 3600)
    };
    Response::xml(
        200,
        format!(
            "<{action}Response xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><{action}Result><Credentials><AccessKeyId>ASIA{short}</AccessKeyId><SecretAccessKey>secret-of-{short}</SecretAccessKey><SessionToken>token-{short}-{session}</SessionToken><Expiration>{expiry}</Expiration></Credentials><AssumedRoleUser><Arn>arn:aws:sts::123456789012:assumed-role/{short}/{session}</Arn><AssumedRoleId>AROA:{session}</AssumedRoleId></AssumedRoleUser></{action}Result><ResponseMetadata><RequestId>fake</RequestId></ResponseMetadata></{action}Response>"
        ),
    )
}

fn oidc_token(script: &mut Script, request: &Request) -> Response {
    match request.json("grantType").as_deref() {
        Some("urn:ietf:params:oauth:grant-type:device_code") => {
            if request.json("deviceCode").as_deref() != Some("device-1") {
                return Response::json(400, serde_json::json!({"error": "invalid_grant"}));
            }
            if script.sso_pending > 0 {
                script.sso_pending -= 1;
                return Response::json(
                    400,
                    serde_json::json!({"error": "authorization_pending", "error_description": "not yet"}),
                );
            }
            Response::json(
                200,
                serde_json::json!({
                    "accessToken": script.sso_token,
                    "tokenType": "Bearer",
                    "expiresIn": 3600,
                    "refreshToken": SSO_REFRESH_TOKEN,
                }),
            )
        }
        Some("refresh_token") => {
            let presented = request.json("refreshToken");
            if let Some(required) = &script.sso_refresh_token
                && presented.as_deref() != Some(required)
            {
                return Response::json(
                    400,
                    serde_json::json!({"error": "invalid_grant", "error_description": "unknown refresh token"}),
                );
            }
            if request.json("clientId").is_none() || request.json("clientSecret").is_none() {
                return Response::json(400, serde_json::json!({"error": "invalid_client"}));
            }
            Response::json(
                200,
                serde_json::json!({
                    "accessToken": script.sso_token,
                    "tokenType": "Bearer",
                    "expiresIn": 3600,
                    "refreshToken": SSO_REFRESH_TOKEN,
                }),
            )
        }
        _ => Response::json(400, serde_json::json!({"error": "unsupported_grant_type"})),
    }
}

/// The Sign-In service's `CreateOAuth2Token` refresh. The proof is verified
/// here, by this file's own decoding and its own ES256 check, against the URL
/// the request reached: a client that signed the wrong bytes, named another
/// request or replayed a proof is refused as the service would refuse it.
fn signin_token(script: &mut Script, address: &str, request: &Request) -> Response {
    let invalid = |message: &str| {
        Response::json(
            400,
            serde_json::json!({"error": "INVALID_REQUEST", "message": message}),
        )
    };
    let htu = format!("http://{address}{}", request.path);
    let jti = match verify_dpop(request.header("dpop"), &htu, &script.login_jtis) {
        Ok(jti) => jti,
        Err(defect) => return invalid(&format!("invalid DPoP proof: {defect}")),
    };
    script.login_jtis.push(jti);
    if request.header("authorization").is_some() {
        return invalid("a refresh is not signed");
    }
    if request.header("content-type") != Some("application/json") {
        return invalid("expected an application/json body");
    }
    if request.json("grantType").as_deref() != Some("refresh_token") {
        return invalid("expected grantType refresh_token");
    }
    if request.json("clientId").is_none() {
        return invalid("expected a clientId");
    }
    let presented = request.json("refreshToken");
    if presented.is_none()
        || script
            .login_refresh_token
            .as_ref()
            .is_some_and(|required| presented.as_ref() != Some(required))
    {
        return invalid("expected the refresh token the sign-in holds");
    }
    if let Some((status, code, times)) = script.login_refusals.clone() {
        script.login_refusals = (times > 1).then(|| (status, code.clone(), times - 1));
        return Response::json(
            status,
            serde_json::json!({"error": code, "message": "refused as scripted"}),
        );
    }
    Response::json(
        200,
        serde_json::json!({
            "accessToken": {
                "accessKeyId": LOGIN_ACCESS_KEY,
                "secretAccessKey": LOGIN_SECRET_KEY,
                "sessionToken": LOGIN_SESSION_TOKEN,
            },
            "tokenType": "aws_sigv4",
            "expiresIn": LOGIN_EXPIRES_IN,
            "refreshToken": LOGIN_REFRESH_TOKEN,
        }),
    )
}

/// The `jti` of `proof` when it is a `DPoP` proof (RFC 9449) for `POST htu`,
/// issued within `DPOP_SKEW` of now under an identifier not in `seen`, and
/// signed by the P-256 key its own header carries; else what is wrong with it.
fn verify_dpop(proof: Option<&str>, htu: &str, seen: &[String]) -> Result<String, String> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let proof = proof.ok_or("no DPoP header")?;
    let parts: Vec<&str> = proof.split('.').collect();
    let [header, claims, signature] = parts[..] else {
        return Err(format!("{} parts, not a compact JWS's three", parts.len()));
    };
    let decode = |part: &str| -> Result<serde_json::Value, String> {
        let bytes = engine.decode(part).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    };
    let header_document = decode(header)?;
    if header_document["typ"] != "dpop+jwt" || header_document["alg"] != "ES256" {
        return Err(format!("header {header_document}"));
    }
    let jwk = &header_document["jwk"];
    if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
        return Err(format!("jwk {jwk}"));
    }
    let coordinate = |name: &str| {
        jwk[name]
            .as_str()
            .and_then(|text| engine.decode(text).ok())
            .filter(|bytes| bytes.len() == 32)
            .ok_or_else(|| format!("jwk.{name} is not 32 bytes"))
    };
    let mut point = vec![0x04];
    point.extend(coordinate("x")?);
    point.extend(coordinate("y")?);
    let claims_document = decode(claims)?;
    if claims_document["htm"] != "POST" {
        return Err(format!("htm {}", claims_document["htm"]));
    }
    if claims_document["htu"] != htu {
        return Err(format!("htu {}, not {htu}", claims_document["htu"]));
    }
    let issued = claims_document["iat"].as_i64().ok_or("no iat")?;
    if (now_seconds() - issued).abs() > DPOP_SKEW {
        return Err(format!("iat {issued} is not now"));
    }
    let jti = claims_document["jti"].as_str().ok_or("no jti")?;
    if !is_uuid_v4(jti) {
        return Err(format!("jti {jti} is not a version 4 UUID"));
    }
    if seen.iter().any(|held| held == jti) {
        return Err(format!("jti {jti} was seen before"));
    }
    let signature = engine
        .decode(signature)
        .map_err(|error| error.to_string())?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, &point)
        .verify(format!("{header}.{claims}").as_bytes(), &signature)
        .map_err(|_| "the ES256 signature does not verify".to_owned())?;
    Ok(jti.to_owned())
}

/// Whether `text` is a version 4 RFC 9562 UUID, spelled 8-4-4-4-12 in
/// lowercase hex.
fn is_uuid_v4(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

fn portal(script: &Script, request: &Request) -> Response {
    if request.header("x-amz-sso_bearer_token") != Some(script.sso_token.as_str()) {
        if script.portal_error_in_header {
            return Response::json(
                401,
                serde_json::json!({"message": "Session token not found or invalid"}),
            )
            .with_header(
                "x-amzn-ErrorType",
                "UnauthorizedException:http://internal.amazon.com/",
            );
        }
        return Response::json(
            401,
            serde_json::json!({"error": "UnauthorizedException", "message": "Session token not found or invalid"}),
        );
    }
    let role = request.query("role_name").unwrap_or("Role");
    let account = request.query("account_id").unwrap_or("000000000000");
    Response::json(
        200,
        serde_json::json!({
            "roleCredentials": {
                "accessKeyId": format!("ASIA{role}"),
                "secretAccessKey": format!("secret-of-{role}-in-{account}"),
                "sessionToken": format!("sso-session-token-{role}"),
                "expiration": (now_seconds() + 3600) * 1000,
            }
        }),
    )
}

fn container(script: &Script, request: &Request) -> Response {
    if let Some(required) = &script.container_authorization
        && request.header("authorization") != Some(required.as_str())
    {
        return Response::json(401, serde_json::json!({"Code": "Unauthorized"}));
    }
    Response::json(
        200,
        serde_json::json!({
            "AccessKeyId": script.container_access_key,
            "SecretAccessKey": "container-secret",
            "Token": "container-token",
            "Expiration": script.container_expiry,
            "RoleArn": "arn:aws:iam::123456789012:role/task",
        }),
    )
}

fn imds(script: &Script, request: &Request) -> Response {
    if script.imds_v2_only && request.header("x-aws-ec2-metadata-token") != Some(IMDS_TOKEN) {
        return Response::text(401, "");
    }
    match request.path.as_str() {
        "/latest/dynamic/instance-identity/document" => Response::json(
            200,
            serde_json::json!({
                "region": script.imds_region,
                "instanceId": "i-0123456789abcdef0",
                "accountId": "123456789012",
            }),
        ),
        "/latest/meta-data/iam/security-credentials/" => match &script.imds_role {
            Some(role) => Response::text(200, role),
            None => Response::text(404, ""),
        },
        path if path.starts_with("/latest/meta-data/iam/security-credentials/") => {
            let role = path.rsplit('/').next().unwrap_or_default();
            if script.imds_role.as_deref() != Some(role) {
                return Response::text(404, "");
            }
            Response::json(
                200,
                serde_json::json!({
                    "Code": "Success",
                    "Type": "AWS-HMAC",
                    "AccessKeyId": format!("ASIA{}", role.to_ascii_uppercase().replace('-', "")),
                    "SecretAccessKey": format!("secret-of-{role}"),
                    "Token": format!("token-of-{role}"),
                    "Expiration": script
                        .imds_expiry
                        .clone()
                        .unwrap_or_else(|| iso8601(now_seconds() + 6 * 3600)),
                }),
            )
        }
        _ => Response::text(404, ""),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Answer",
    }
}

fn now_seconds() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(0)
}

/// `seconds` since the epoch as `YYYY-MM-DDThh:mm:ssZ`.
pub fn iso8601(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}
