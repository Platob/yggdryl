//! How an Azure request says who is asking.
//!
//! Azure has four answers and they are not variations of one: a signature over
//! the request built from the account's shared key, a token already in the
//! query that nothing signs, a bearer token from Entra ID, or nothing at all
//! for a public container. The choice belongs to the client, is made once, and
//! never changes for the life of a handle.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use super::options::AzureOptions;
use super::sign::SharedKey;
use crate::{Error, Result, Scalar};

/// The scope a storage token is asked for.
const SCOPE: &str = "https://storage.azure.com/.default";
/// The v1 resource the metadata service takes, which is the scope without its
/// `/.default` suffix.
const RESOURCE: &str = "https://storage.azure.com";
/// Where an instance's own token is asked for.
const IMDS_ENDPOINT: &str = "http://169.254.169.254/metadata/identity/oauth2/token";
/// The version the metadata service answers under.
const IMDS_VERSION: &str = "2018-02-01";
/// The version the hosted identity endpoints answer under.
const IDENTITY_VERSION: &str = "2019-08-01";
/// How long before a token lapses it is obtained again.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Off an Azure host, reaching the link-local address hangs; a token is not
/// worth waiting on that long.
const IMDS_TIMEOUT: Duration = Duration::from_secs(2);
/// The largest token document read from any of these endpoints.
const MAX_ANSWER: u64 = 256 * 1024;

/// What authorizes this client's requests.
pub(crate) enum Authorization {
    /// A signature over each request, from the account's own key.
    SharedKey(SharedKey),
    /// A signature already computed, carried in the query.
    Sas(String),
    /// A bearer token, obtained once and refreshed before it lapses.
    Bearer(TokenCache),
    /// Nothing, which a public container allows.
    Anonymous,
}

impl Authorization {
    /// Decide how requests will be authorized, reading no network.
    ///
    /// The order is the one every Azure tool uses: an explicit token, then a
    /// shared access signature, then the account key, then an Entra ID
    /// application, then the instance's own identity, then nothing.
    ///
    /// # Errors
    ///
    /// Returns a refusal when an account key is not base64, or when a key is
    /// given with no account to sign for.
    pub(crate) fn new(
        options: &AzureOptions,
        account: Option<&str>,
        account_id: Option<&str>,
        key: Option<&str>,
        anonymous: bool,
    ) -> Result<Self> {
        if anonymous {
            return Ok(Self::Anonymous);
        }
        if let Some(token) = options.bearer_token() {
            return Ok(Self::Bearer(TokenCache::fixed(token)));
        }
        if let Some(token) = options.sas_token() {
            return Ok(Self::Sas(token.to_owned()));
        }
        // A generic credential pair is Azure's shared key only when it names
        // this account: a caller who set keys for another store and then named
        // an account of their own meant the account, and signing with those
        // keys would refuse every request over a value never meant for here.
        let paired = key.filter(|_| {
            account
                .is_some_and(|account| account.eq_ignore_ascii_case(account_id.unwrap_or_default()))
        });
        if let (Some(account), Some(key)) = (account, options.account_key().or(paired)) {
            return Ok(Self::SharedKey(SharedKey::new(account, key)?));
        }
        if let (Some(tenant), Some(client), Some(secret)) = (
            options.tenant_id(),
            options.client_id(),
            options.client_secret(),
        ) {
            return Ok(Self::Bearer(TokenCache::client_secret(
                options.authority_host(),
                tenant,
                client,
                secret,
            )));
        }
        if let (Some(tenant), Some(client), Some(path)) = (
            options.tenant_id(),
            options.client_id(),
            options.federated_token_file(),
        ) {
            return Ok(Self::Bearer(TokenCache::federated(
                options.authority_host(),
                tenant,
                client,
                path,
            )));
        }
        if options.managed_identity() {
            return Ok(Self::Bearer(TokenCache::managed(options.client_id())));
        }
        Ok(Self::Anonymous)
    }

    /// The query a request carries beyond its own, which only a shared access
    /// signature has.
    pub(crate) fn query_suffix(&self) -> Option<&str> {
        match self {
            Self::Sas(token) => Some(token),
            _ => None,
        }
    }

    /// The headers that say who is asking.
    ///
    /// # Errors
    ///
    /// Returns the refusal of whichever token endpoint was asked.
    pub(crate) fn headers(
        &self,
        agent: &ureq::Agent,
        method: &str,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        now: SystemTime,
    ) -> Result<Vec<(String, String)>> {
        match self {
            Self::SharedKey(key) => Ok(key.sign(method, path, query, headers, now)),
            // The token is the signature, and a signed request carries no date
            // of its own beyond what the token already pins.
            Self::Sas(_) | Self::Anonymous => Ok(Vec::new()),
            Self::Bearer(cache) => {
                let token = cache.resolve(agent, now)?;
                Ok(vec![(
                    "authorization".to_owned(),
                    format!("Bearer {token}"),
                )])
            }
        }
    }
}

/// Where a bearer token comes from, and the one currently held.
pub(crate) struct TokenCache {
    source: Source,
    held: Mutex<Option<(String, Option<SystemTime>)>>,
}

/// The identity a token is obtained for.
enum Source {
    /// A token the caller already holds; nothing is obtained or refreshed.
    Fixed(String),
    /// An Entra ID application and its secret.
    ClientSecret {
        endpoint: String,
        client_id: String,
        secret: String,
    },
    /// An Entra ID application and a signed assertion a platform projects.
    Federated {
        endpoint: String,
        client_id: String,
        path: std::path::PathBuf,
    },
    /// The instance's own identity, asked of the metadata service.
    Managed { client_id: Option<String> },
}

impl TokenCache {
    /// A token the caller supplied.
    fn fixed(token: &str) -> Self {
        Self::of(Source::Fixed(token.to_owned()))
    }

    /// An Entra ID application authenticating with a secret.
    fn client_secret(authority: &str, tenant: &str, client_id: &str, secret: &str) -> Self {
        Self::of(Source::ClientSecret {
            endpoint: token_endpoint(authority, tenant),
            client_id: client_id.to_owned(),
            secret: secret.to_owned(),
        })
    }

    /// The same, authenticating with a projected federated token.
    fn federated(authority: &str, tenant: &str, client_id: &str, path: &std::path::Path) -> Self {
        Self::of(Source::Federated {
            endpoint: token_endpoint(authority, tenant),
            client_id: client_id.to_owned(),
            path: path.to_path_buf(),
        })
    }

    /// The instance's own identity.
    fn managed(client_id: Option<&str>) -> Self {
        Self::of(Source::Managed {
            client_id: client_id.map(str::to_owned),
        })
    }

    /// Hold `source`, having obtained nothing yet.
    fn of(source: Source) -> Self {
        Self {
            source,
            held: Mutex::new(None),
        }
    }

    /// The token to authorize with now, obtaining one if what is held lapses.
    fn resolve(&self, agent: &ureq::Agent, now: SystemTime) -> Result<String> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((token, expiry)) = held.as_ref() {
            if expiry.is_none_or(|expiry| expiry > now + REFRESH_MARGIN) {
                return Ok(token.clone());
            }
        }
        let (token, expiry) = self.obtain(agent, now)?;
        *held = Some((token.clone(), expiry));
        Ok(token)
    }

    /// Ask this cache's source for a token.
    fn obtain(&self, agent: &ureq::Agent, now: SystemTime) -> Result<(String, Option<SystemTime>)> {
        match &self.source {
            Source::Fixed(token) => Ok((token.clone(), None)),
            Source::ClientSecret {
                endpoint,
                client_id,
                secret,
            } => exchange(
                agent,
                endpoint,
                &form(&[
                    ("client_id", client_id),
                    ("client_secret", secret),
                    ("scope", SCOPE),
                    ("grant_type", "client_credentials"),
                ]),
                now,
            ),
            Source::Federated {
                endpoint,
                client_id,
                path,
            } => {
                let assertion = std::fs::read_to_string(path).map_err(|error| {
                    refusal(&format!(
                        "could not read the federated token file {}: {error}",
                        path.display()
                    ))
                })?;
                exchange(
                    agent,
                    endpoint,
                    &form(&[
                        ("client_id", client_id),
                        (
                            "client_assertion_type",
                            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                        ),
                        ("client_assertion", assertion.trim()),
                        ("scope", SCOPE),
                        ("grant_type", "client_credentials"),
                    ]),
                    now,
                )
            }
            Source::Managed { client_id } => from_metadata(agent, client_id.as_deref(), now),
        }
    }
}

/// Where an Entra ID tenant answers a token request.
fn token_endpoint(authority: &str, tenant: &str) -> String {
    format!("{authority}/{tenant}/oauth2/v2.0/token")
}

/// The instance's own token, from whichever identity endpoint this host has.
fn from_metadata(
    agent: &ureq::Agent,
    client_id: Option<&str>,
    now: SystemTime,
) -> Result<(String, Option<SystemTime>)> {
    // A hosted platform projects its own endpoint and a header only it knows,
    // and that is preferred over the link-local address when both are there.
    let hosted = (variable("IDENTITY_ENDPOINT"), variable("IDENTITY_HEADER"));
    let (url, version, secret) = match hosted {
        (Some(endpoint), Some(secret)) => (endpoint, IDENTITY_VERSION, Some(secret)),
        _ => (IMDS_ENDPOINT.to_owned(), IMDS_VERSION, None),
    };
    let mut request = agent
        .get(&url)
        .query("api-version", version)
        // The metadata service takes the resource an older protocol names,
        // which is the scope without its `/.default` suffix.
        .query("resource", RESOURCE)
        .header("metadata", "true");
    if let Some(secret) = &secret {
        request = request.header("x-identity-header", secret);
    }
    if let Some(client_id) = client_id {
        request = request.query("client_id", client_id);
    }
    let mut response = request
        .config()
        .timeout_global(Some(IMDS_TIMEOUT))
        .build()
        .call()
        .map_err(|error| {
            refusal(&format!(
                "the Azure identity endpoint at {url} answered no token: {error}"
            ))
        })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| refusal(&format!("could not read the identity answer: {error}")))?;
    read_token(&body, now)
}

/// Exchange a form body at a token endpoint.
fn exchange(
    agent: &ureq::Agent,
    url: &str,
    body: &str,
    now: SystemTime,
) -> Result<(String, Option<SystemTime>)> {
    let mut response = agent
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .send(body.as_bytes())
        .map_err(|error| refusal(&format!("the token endpoint {url} refused: {error}")))?;
    let status = response.status().as_u16();
    let answer = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(|error| refusal(&format!("could not read the token answer: {error}")))?;
    if status >= 300 {
        let described = String::from_utf8_lossy(&answer);
        let described = described.lines().next().unwrap_or_default();
        return Err(Error::remote(
            "az",
            "GetAccessToken",
            status,
            "TokenError",
            described,
            url,
        ));
    }
    read_token(&answer, now)
}

/// Read `{"access_token": ..., "expires_in": ...}`.
fn read_token(body: &[u8], now: SystemTime) -> Result<(String, Option<SystemTime>)> {
    let value = crate::text::json::from_bytes(body)
        .map_err(|error| refusal(&format!("expected an access token: {error}")))?;
    let token = value
        .get_key_str("access_token")
        .and_then(Scalar::as_str)
        .ok_or_else(|| refusal("expected access_token in the token answer"))?;
    // A hosted endpoint states a lifetime as a string and the rest as a number.
    let lifetime = value.get_key_str("expires_in").and_then(|value| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .or_else(|| value.as_u128().and_then(|value| u64::try_from(value).ok()))
    });
    Ok((
        token.to_owned(),
        lifetime.map(|seconds| now + Duration::from_secs(seconds)),
    ))
}

/// A form body, percent-encoded as one.
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                super::super::sigv4::encode_query_component(name),
                super::super::sigv4::encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// One environment variable, empty read as unset.
fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Refuse something about obtaining a token.
fn refusal(message: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}
