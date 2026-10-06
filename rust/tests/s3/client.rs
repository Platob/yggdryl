//! `rust/src/s3/client.rs`: the addressing and the retry schedule.
//!
//! Where a request is sent, how its path is spelled, which region signs it, and
//! how long the client waits before trying again are all settled before a
//! single request goes out - so they are pinned here, with no store to answer,
//! beside what the requests themselves look like on the wire.
//!
//! On Amazon S3 the region, the endpoint and the addressing style also come
//! from the [`Session`] the options carry - its profile, its environment, its
//! FIPS and dual-stack switches - so those readings are pinned here too, each
//! over a session whose files are text and whose environment is handed over,
//! so nothing of the machine's own decides.

use yggdryl::Url;
use yggdryl::aws::Session;
use yggdryl::internals::s3_client::{
    Client, DEFAULT_REGION, Endpoint, RETRY_BACKOFF, backoff, bucket_region_of,
    total_of_content_range,
};
use yggdryl::s3::S3Options;

fn url(text: &str) -> Url {
    Url::from_str(text).expect("a valid location")
}

/// Options that consult nothing outside the test.
fn sealed() -> S3Options {
    S3Options::default().with_environment(false)
}

/// A session that reads nothing but the configuration file `config`, and
/// consults no environment.
///
/// The options a client is built with seal a session they carry anyway when
/// they consult no environment themselves; text needs no environment, so the
/// profile it spells is read either way.
fn profiled(config: &str) -> Session {
    Session::new()
        .with_environment(false)
        .with_config_text(config)
}

/// Options that consult an environment, carrying a session whose environment
/// is exactly `variables` and whose shared files are `config` and nothing -
/// so what the AWS tools would read from the machine is what the test says.
///
/// The options sweep no prefix, so no variable of this process becomes a
/// knob; and they are anonymous, so the Google and Azure halves of the client
/// look for no identity of their own on the machine. Who signs plays no part
/// in where a request goes.
fn ambient(variables: &[(&str, &str)], config: &str) -> S3Options {
    let session = Session::new()
        .with_variables(variables.iter().copied())
        .with_config_text(config)
        .with_credentials_text("")
        .with_metadata_disabled(true);
    S3Options::default()
        .with_environment_prefixes(std::iter::empty::<String>())
        .with_anonymous(true)
        .with_session(session)
}

/// The client `location` and `options` describe.
fn client(location: &str, options: S3Options) -> Client {
    Client::new(&url(location), options).expect("a client")
}

#[test]
fn an_aws_location_is_addressed_virtual_hosted_and_signed_for_its_region() {
    let client = Client::new(
        &url("s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet"),
        sealed(),
    )
    .expect("a client");
    assert_eq!(client.region(), "eu-west-3");
    assert_eq!(
        client.host_header("trades"),
        "trades.s3.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        client.path("trades", "lake/part.parquet"),
        "/lake/part.parquet"
    );
}

#[test]
fn a_bucket_only_location_defaults_its_endpoint_from_the_region() {
    let client = Client::new(&url("s3://trades/lake/part.parquet"), sealed()).expect("a client");
    assert_eq!(client.region(), DEFAULT_REGION);
    assert_eq!(
        client.host_header("trades"),
        "trades.s3.us-east-1.amazonaws.com"
    );

    // A dot in the bucket would break a TLS wildcard, so it stays in the path.
    let dotted = Client::new(&url("s3://my.trades/part.parquet"), sealed()).expect("a client");
    assert_eq!(
        dotted.host_header("my.trades"),
        "s3.us-east-1.amazonaws.com"
    );
    assert_eq!(
        dotted.path("my.trades", "part.parquet"),
        "/my.trades/part.parquet"
    );
}

#[test]
fn a_local_endpoint_is_addressed_path_style_over_its_own_scheme_and_port() {
    let client = Client::new(
        &url("s3://localhost:9000/trades/lake/part.parquet"),
        sealed().with_endpoint("http://localhost:9000"),
    )
    .expect("a client");
    assert_eq!(client.scheme(), "http");
    assert_eq!(client.host_header("trades"), "localhost:9000");
    assert_eq!(
        client.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );
}

#[test]
fn a_key_is_encoded_once_and_its_separators_survive() {
    let endpoint = Endpoint::new("https", "s3.example.io", None, true, None, false);
    assert_eq!(
        endpoint.path("trades", "year=2026/a b/c+d.parquet"),
        "/trades/year%3D2026/a%20b/c%2Bd.parquet"
    );
    // The bucket root has no trailing separator to sign over.
    assert_eq!(endpoint.path("trades", ""), "/trades");
}

#[test]
fn explicit_credentials_beat_a_url_that_carries_its_own() {
    let carried = Client::url_credentials(&url("s3://key:s3cr3t@trades/part.parquet"))
        .expect("credentials off the URL");
    assert_eq!(carried.access_key_id(), "key");

    let explicit = Client::new(
        &url("s3://key:s3cr3t@trades/part.parquet"),
        sealed().with_credentials(yggdryl::s3::Credentials::new("other", "secret")),
    )
    .expect("a client");
    let signer = explicit
        .signer_access_key_id(std::time::SystemTime::UNIX_EPOCH)
        .expect("a signer")
        .expect("credentials");
    assert_eq!(signer, "other");
}

#[test]
fn a_profile_that_asks_for_path_style_addresses_an_aws_location_in_the_path() {
    let session =
        profiled("[profile trading]\nregion = eu-west-3\ns3 =\n  addressing_style = path\n")
            .with_profile("trading");
    let path_style = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(path_style.region(), "eu-west-3");
    assert_eq!(
        path_style.host_header("trades"),
        "s3.eu-west-3.amazonaws.com",
        "the bucket leaves the host"
    );
    assert_eq!(
        path_style.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );

    // The options' own choice wins over the profile's.
    let explicit = client(
        "s3://trades/lake/part.parquet",
        sealed().with_path_style(false).with_session(session),
    );
    assert_eq!(
        explicit.host_header("trades"),
        "trades.s3.eu-west-3.amazonaws.com"
    );

    // And `virtual` is a choice too: it holds even for the dotted bucket that
    // is otherwise kept in the path.
    let virtual_hosted = client(
        "s3://my.trades/part.parquet",
        sealed().with_session(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  addressing_style = virtual\n",
        )),
    );
    assert_eq!(
        virtual_hosted.host_header("my.trades"),
        "my.trades.s3.eu-west-3.amazonaws.com"
    );
}

#[test]
fn a_session_asking_for_fips_or_dual_stack_reaches_that_published_host() {
    let host = |session: Session| {
        client(
            "s3://trades/lake/part.parquet",
            sealed().with_session(session),
        )
        .host_header("trades")
    };
    let stated = Session::new()
        .with_environment(false)
        .with_region("eu-west-3");
    assert_eq!(
        host(stated.with_use_fips_endpoint(true)),
        "trades.s3-fips.eu-west-3.amazonaws.com",
        "virtual hosted, on the FIPS host"
    );
    assert_eq!(
        host(stated.with_use_dualstack_endpoint(true)),
        "trades.s3.dualstack.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host(
            stated
                .with_use_fips_endpoint(true)
                .with_use_dualstack_endpoint(true)
        ),
        "trades.s3-fips.dualstack.eu-west-3.amazonaws.com"
    );

    // The profile's own switches reach the same hosts: `use_fips_endpoint`
    // at its top level, and the `s3` table's dual-stack and accelerate ones.
    assert_eq!(
        host(profiled(
            "[default]\nregion = eu-west-3\nuse_fips_endpoint = true\n"
        )),
        "trades.s3-fips.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  use_dualstack_endpoint = true\n"
        )),
        "trades.s3.dualstack.eu-west-3.amazonaws.com"
    );
    let accelerated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(profiled(
            "[default]\nregion = eu-west-3\ns3 =\n  use_accelerate_endpoint = true\n",
        )),
    );
    assert_eq!(
        accelerated.host_header("trades"),
        "trades.s3-accelerate.amazonaws.com",
        "acceleration has one host for every region"
    );
    assert_eq!(
        accelerated.region(),
        "eu-west-3",
        "and still signs for the bucket's"
    );
}

#[test]
fn a_china_region_is_addressed_on_the_china_partition() {
    let china = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            Session::new()
                .with_environment(false)
                .with_region("cn-north-1"),
        ),
    );
    assert_eq!(china.region(), "cn-north-1");
    assert_eq!(
        china.host_header("trades"),
        "trades.s3.cn-north-1.amazonaws.com.cn",
        "virtual hosted, because the partition's host is AWS's own"
    );
    assert_eq!(
        china.path("trades", "lake/part.parquet"),
        "/lake/part.parquet"
    );
}

#[test]
fn the_region_comes_from_the_sessions_profile_when_the_url_and_options_name_none() {
    let session = profiled("[profile trading]\nregion = ap-southeast-2\n").with_profile("trading");
    let from_profile = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(from_profile.region(), "ap-southeast-2");
    assert_eq!(
        from_profile.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );

    // A region the session states outright beats its profile's.
    let stated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session.with_region("eu-central-1")),
    );
    assert_eq!(stated.region(), "eu-central-1");

    // The location's own region comes ahead of the session's, and an explicit
    // one ahead of both.
    let from_url = client(
        "s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet",
        sealed().with_session(session.clone()),
    );
    assert_eq!(from_url.region(), "eu-west-3");
    let explicit = client(
        "s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet",
        sealed().with_region("us-west-2").with_session(session),
    );
    assert_eq!(explicit.region(), "us-west-2");

    // A profile the session names that neither file holds names no region,
    // and the default stands.
    let unwritten = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            profiled("[profile other]\nregion = eu-west-1\n").with_profile("trading"),
        ),
    );
    assert_eq!(unwritten.region(), DEFAULT_REGION);
}

#[test]
fn options_that_consult_no_environment_seal_the_session_they_carry() {
    // The session would read its region and its S3 endpoint out of the
    // environment it was handed; the options say no environment decides, so
    // neither does.
    let session = Session::new()
        .with_variables([
            ("AWS_REGION", "eu-west-3"),
            ("AWS_ENDPOINT_URL_S3", "http://localhost:9000"),
        ])
        .with_config_text("")
        .with_credentials_text("");
    let sealed_client = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(session),
    );
    assert_eq!(sealed_client.region(), DEFAULT_REGION);
    assert_eq!(
        sealed_client.host_header("trades"),
        "trades.s3.us-east-1.amazonaws.com"
    );
    assert_eq!(sealed_client.scheme(), "https");
}

#[test]
fn the_s3_endpoint_the_sessions_environment_names_is_the_one_addressed() {
    // The service's own variable beats the generic one, as the AWS tools
    // read them, and an endpoint that is not AWS's is addressed path style.
    let named = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[
                ("AWS_ENDPOINT_URL_S3", "http://localhost:9000"),
                ("AWS_ENDPOINT_URL", "http://localhost:9999"),
                ("AWS_REGION", "eu-west-3"),
            ],
            "",
        ),
    );
    assert_eq!(named.scheme(), "http");
    assert_eq!(named.host_header("trades"), "localhost:9000");
    assert_eq!(
        named.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );
    assert_eq!(named.region(), "eu-west-3");

    // The generic one, when the service names none of its own.
    let generic = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL", "http://localhost:9999")], ""),
    );
    assert_eq!(generic.host_header("trades"), "localhost:9999");

    // `AWS_DEFAULT_REGION` when `AWS_REGION` is unset, and
    // `AWS_S3_FORCE_PATH_STYLE` out of the same environment.
    let regional = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[
                ("AWS_DEFAULT_REGION", "ap-south-1"),
                ("AWS_S3_FORCE_PATH_STYLE", "true"),
            ],
            "",
        ),
    );
    assert_eq!(regional.region(), "ap-south-1");
    assert_eq!(
        regional.host_header("trades"),
        "s3.ap-south-1.amazonaws.com"
    );
    assert_eq!(
        regional.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet"
    );

    // An explicit endpoint on the options wins over the environment's.
    let explicit = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL_S3", "http://localhost:9000")], "")
            .with_endpoint("http://localhost:9500"),
    );
    assert_eq!(explicit.host_header("trades"), "localhost:9500");
}

#[test]
fn an_endpoint_stated_on_a_session_is_honoured_by_options_that_consult_no_environment() {
    let stated = client(
        "s3://trades/lake/part.parquet",
        sealed().with_session(
            Session::new()
                .with_environment(false)
                .with_service_endpoint_url("s3", "http://localhost:9300/"),
        ),
    );
    assert_eq!(stated.host_header("trades"), "localhost:9300");
    assert_eq!(
        stated.path("trades", "lake/part.parquet"),
        "/trades/lake/part.parquet",
        "a bare host is addressed path style"
    );
}

#[test]
fn the_path_an_s3_endpoint_carries_is_the_prefix_every_request_is_sent_under() {
    // A gateway mounting S3 below a path is reached there, as botocore
    // reaches it: path style puts the bucket after the prefix...
    let gateway = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[("AWS_ENDPOINT_URL_S3", "http://localhost:9000/gateway/s3/")],
            "",
        ),
    );
    assert_eq!(gateway.host_header("trades"), "localhost:9000");
    assert_eq!(
        gateway.path("trades", "lake/part.parquet"),
        "/gateway/s3/trades/lake/part.parquet"
    );
    assert_eq!(gateway.path("trades", ""), "/gateway/s3/trades");

    // ... and virtual-hosted style the bucket in the host, the prefix still
    // ahead of the key.
    let hosted = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[
                ("AWS_ENDPOINT_URL_S3", "http://localhost:9000/gateway"),
                ("AWS_S3_FORCE_PATH_STYLE", "false"),
            ],
            "",
        ),
    );
    assert_eq!(hosted.host_header("trades"), "trades.localhost:9000");
    assert_eq!(
        hosted.path("trades", "lake/part.parquet"),
        "/gateway/lake/part.parquet"
    );
    assert_eq!(hosted.path("trades", ""), "/gateway/");

    // An endpoint stated on the options keeps its path the same way.
    let explicit = client(
        "s3://trades/lake/part.parquet",
        sealed().with_endpoint("http://localhost:9000/minio"),
    );
    assert_eq!(
        explicit.path("trades", "lake/part.parquet"),
        "/minio/trades/lake/part.parquet"
    );
}

// --- where a request arrives -------------------------------------------------
//
// Each test below reaches the fake store through one source of the endpoint,
// and the source a wrong reading would take instead points at a loopback port
// nothing listens on. The session is in a region no AWS partition publishes
// a host for, so a client that read no configured endpoint at all would ask
// for `s3.zz-nowhere-1.amazonaws.com` - a name that resolves to nothing - and
// fail on this machine rather than send a fixture-signed request to Amazon S3.

/// A region no AWS partition publishes a host for.
const NOWHERE_REGION: &str = "zz-nowhere-1";

/// A loopback URL nothing listens on: where a decoy source points, so a
/// client that read the wrong source fails on this machine rather than
/// reaching anything.
fn nowhere() -> String {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
    let address = listener.local_addr().expect("a bound address");
    drop(listener);
    format!("http://{address}")
}

/// Options carrying a session in [`NOWHERE_REGION`] whose whole environment
/// is `variables` and whose configuration file is `config`, signing with a
/// pair the fake store accepts and sweeping no prefix of this process.
fn named_by(variables: &[(&str, &str)], config: &str) -> S3Options {
    let session = Session::new()
        .with_variables(variables.iter().copied())
        .with_config_text(config)
        .with_credentials_text("")
        .with_metadata_disabled(true)
        .with_region(NOWHERE_REGION);
    S3Options::default()
        .with_environment_prefixes(std::iter::empty::<String>())
        .with_credentials(yggdryl::s3::Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI",
        ))
        .with_session(session)
}

/// A fake store holding `trades/lake/part.parquet`.
fn holding_a_part() -> crate::server::FakeS3 {
    let store = crate::server::FakeS3::start();
    store.create_bucket("trades");
    store.put("trades", "lake/part.parquet", b"PAR1");
    store
}

/// The part read under `options`: one request, at `path` on `store`.
fn read_at(store: &crate::server::FakeS3, options: S3Options, path: &str) {
    use yggdryl::IOBase;

    store.clear_requests();
    let part = yggdryl::s3::file_with("s3://trades/lake/part.parquet", options).expect("a handle");
    assert_eq!(part.read_all_bytes().expect("the object"), b"PAR1");
    let sent = store.requests();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].path, path);
    assert!(
        sent[0]
            .headers
            .iter()
            .any(|(name, value)| name == "authorization"
                && value.contains(&format!("/{NOWHERE_REGION}/s3/aws4_request"))),
        "signed for the session's region: {:?}",
        sent[0]
    );
}

#[test]
fn a_request_reaches_the_store_aws_endpoint_url_s3_alone_names_and_the_path_it_names() {
    let store = holding_a_part();
    let decoy = nowhere();

    // The variable alone: the generic one beside it points nowhere.
    let named = store.endpoint();
    read_at(
        &store,
        named_by(
            &[
                ("AWS_ENDPOINT_URL_S3", named.as_str()),
                ("AWS_ENDPOINT_URL", decoy.as_str()),
            ],
            "",
        ),
        "/trades/lake/part.parquet",
    );

    // Below a gateway's path, which every request is sent under.
    store.mount_at("/gateway/s3");
    let mounted = format!("{}/gateway/s3/", store.endpoint());
    read_at(
        &store,
        named_by(
            &[
                ("AWS_ENDPOINT_URL_S3", mounted.as_str()),
                ("AWS_ENDPOINT_URL", decoy.as_str()),
            ],
            "",
        ),
        "/gateway/s3/trades/lake/part.parquet",
    );
}

#[test]
fn a_request_reaches_the_store_aws_endpoint_url_alone_names() {
    let store = holding_a_part();
    let (named, decoy) = (store.endpoint(), nowhere());
    // The profile's `[services]` entry and its own endpoint, which the
    // variable outranks, point nowhere.
    read_at(
        &store,
        named_by(
            &[("AWS_ENDPOINT_URL", named.as_str())],
            &format!(
                "[default]\nservices = local\nendpoint_url = {decoy}\n\n\
                 [services local]\ns3 =\n  endpoint_url = {decoy}\n"
            ),
        ),
        "/trades/lake/part.parquet",
    );
}

#[test]
fn a_request_reaches_the_store_the_services_section_s_s3_entry_names() {
    let store = holding_a_part();
    let (named, decoy) = (store.endpoint(), nowhere());
    // The profile's own endpoint, which the entry outranks, and another
    // service's entry point nowhere.
    read_at(
        &store,
        named_by(
            &[],
            &format!(
                "[default]\nservices = local\nendpoint_url = {decoy}\n\n\
                 [services local]\nsts =\n  endpoint_url = {decoy}\n\
                 s3 =\n  endpoint_url = {named}\n"
            ),
        ),
        "/trades/lake/part.parquet",
    );
}

#[test]
fn the_profiles_services_section_names_the_s3_endpoint_unless_configured_ones_are_ignored() {
    const CONFIG: &str = "[default]\nregion = ap-southeast-2\nendpoint_url = http://localhost:9100\n\
                          services = lake\n\n[services lake]\ns3 =\n  endpoint_url = http://localhost:9200\n";

    // The `[services]` entry for S3 beats the profile's `endpoint_url`.
    let serviced = client("s3://trades/lake/part.parquet", ambient(&[], CONFIG));
    assert_eq!(serviced.host_header("trades"), "localhost:9200");
    assert_eq!(serviced.region(), "ap-southeast-2");

    // Without one, the profile's `endpoint_url` serves every service.
    let profiled_endpoint = client(
        "s3://trades/lake/part.parquet",
        ambient(&[], "[default]\nendpoint_url = http://localhost:9100\n"),
    );
    assert_eq!(profiled_endpoint.host_header("trades"), "localhost:9100");

    // The environment's variable beats both files.
    let variable = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_ENDPOINT_URL_S3", "http://localhost:9000")], CONFIG),
    );
    assert_eq!(variable.host_header("trades"), "localhost:9000");

    // And a process that ignores configured endpoints reaches the published
    // host, whichever of the two says so.
    let ignored = client(
        "s3://trades/lake/part.parquet",
        ambient(&[("AWS_IGNORE_CONFIGURED_ENDPOINT_URLS", "true")], CONFIG),
    );
    assert_eq!(
        ignored.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );
    let ignored_by_profile = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[],
            "[default]\nregion = ap-southeast-2\nendpoint_url = http://localhost:9100\n\
             ignore_configured_endpoint_urls = true\n",
        ),
    );
    assert_eq!(
        ignored_by_profile.host_header("trades"),
        "trades.s3.ap-southeast-2.amazonaws.com"
    );
}

#[test]
fn a_profile_naming_a_services_section_nobody_wrote_refuses_the_client_rather_than_the_published_host()
 {
    const MISSPELT: &str = "[default]\nregion = eu-west-3\nservices = locl\n\n\
                            [services local]\ns3 =\n  endpoint_url = http://localhost:9200\n";
    let refused = Client::new(
        &url("s3://trades/lake/part.parquet"),
        ambient(&[], MISSPELT),
    )
    .err()
    .expect("a refusal")
    .to_string();
    assert!(
        refused.contains("names services locl, which no [services locl] section defines"),
        "{refused}"
    );

    // The lookup that ends before the `[services]` step never reaches it:
    // the service's own variable, or an endpoint the options state.
    let variable = client(
        "s3://trades/lake/part.parquet",
        ambient(
            &[("AWS_ENDPOINT_URL_S3", "http://localhost:9000")],
            MISSPELT,
        ),
    );
    assert_eq!(variable.host_header("trades"), "localhost:9000");
    let explicit = client(
        "s3://trades/lake/part.parquet",
        ambient(&[], MISSPELT).with_endpoint("http://localhost:9500"),
    );
    assert_eq!(explicit.host_header("trades"), "localhost:9500");
}

#[test]
fn a_content_range_states_the_total_and_a_redirect_states_the_region() {
    assert_eq!(total_of_content_range(Some("bytes 0-9/1024")), Some(1024));
    assert_eq!(total_of_content_range(Some("bytes */1024")), Some(1024));
    assert_eq!(total_of_content_range(Some("bytes 0-9/*")), None);
    assert_eq!(total_of_content_range(None), None);

    let region = [("x-amz-bucket-region".to_owned(), "eu-west-3".to_owned())];
    assert_eq!(
        bucket_region_of(301, &region, b""),
        Some(Some("eu-west-3".to_owned()))
    );

    // A 200 is never a redirect, whatever headers it carries.
    assert_eq!(bucket_region_of(200, &region, b""), None);
}

/// Amazon S3's refusal of a request signed for `us-east-1` that reached a
/// bucket in `eu-west-3`, with `extra` inside the `<Error>`.
fn malformed(extra: &str) -> String {
    format!(
        "<Error><Code>AuthorizationHeaderMalformed</Code><Message>The authorization header \
         is malformed; the region 'us-east-1' is wrong; expecting 'eu-west-3'</Message>\
         {extra}</Error>"
    )
}

#[test]
fn a_malformed_authorization_answer_states_the_region_it_expects() {
    // With no header, the message's region is the one it expects - the last
    // it quotes - and never the one the request was signed for.
    assert_eq!(
        bucket_region_of(400, &[], malformed("").as_bytes()),
        Some(Some("eu-west-3".to_owned()))
    );
    // The document's own `<Region>` is read before the message; a region
    // that differs from the message's shows which was read.
    assert_eq!(
        bucket_region_of(
            400,
            &[],
            malformed("<Region>eu-central-1</Region>").as_bytes()
        ),
        Some(Some("eu-central-1".to_owned()))
    );
    // And the header before both.
    let header = [("x-amz-bucket-region".to_owned(), "ap-south-1".to_owned())];
    assert_eq!(
        bucket_region_of(
            400,
            &header,
            malformed("<Region>eu-central-1</Region>").as_bytes()
        ),
        Some(Some("ap-south-1".to_owned()))
    );
}

#[test]
fn a_redirect_naming_no_region_asks_the_bucket_and_a_refusal_that_is_none_asks_nothing() {
    let refusal = |code: &str| format!("<Error><Code>{code}</Code><Message>m</Message></Error>");
    // A refusal that is no redirect, and the bodyless `400` a lapsed key
    // answers a `HEAD` with, are no redirect at all.
    assert_eq!(
        bucket_region_of(403, &[], refusal("AccessDenied").as_bytes()),
        None
    );
    assert_eq!(bucket_region_of(400, &[], b""), None);
    // A redirect status, a `PermanentRedirect` and a bucket in an opt-in
    // region reached through another region's host each say the request went
    // to the wrong region without saying which is right.
    assert_eq!(bucket_region_of(301, &[], b""), Some(None));
    assert_eq!(bucket_region_of(302, &[], b""), Some(None));
    assert_eq!(bucket_region_of(307, &[], b""), Some(None));
    assert_eq!(
        bucket_region_of(301, &[], refusal("PermanentRedirect").as_bytes()),
        Some(None)
    );
    assert_eq!(
        bucket_region_of(
            400,
            &[],
            refusal("IllegalLocationConstraintException").as_bytes()
        ),
        Some(None)
    );
}

#[test]
fn a_redirect_moves_a_published_host_to_the_region_and_leaves_a_stated_endpoint_alone() {
    let signing = |region: &str| Session::new().with_environment(false).with_region(region);
    let published = client(
        "s3://lake/part.parquet",
        sealed().with_session(signing("us-east-1")),
    );
    // A store answering a region that is no host label moves nothing: the
    // region would be spliced into the host the next request is sent to.
    assert!(published.adopt_region("eu-west-3.example.org#").is_err());
    assert_eq!(published.region(), "us-east-1");
    assert_eq!(
        published.host_header("lake"),
        "lake.s3.us-east-1.amazonaws.com"
    );

    published.adopt_region("eu-west-3").expect("a region");
    assert_eq!(published.region(), "eu-west-3");
    assert_eq!(
        published.host_header("lake"),
        "lake.s3.eu-west-3.amazonaws.com",
        "the regional host answers the bucket's region with another redirect"
    );
    published.adopt_region("cn-north-1").expect("a region");
    assert_eq!(
        published.host_header("lake"),
        "lake.s3.cn-north-1.amazonaws.com.cn",
        "on the partition the new region belongs to"
    );

    // The switches the host was chosen by stay switched.
    let fips = client(
        "s3://lake/part.parquet",
        sealed().with_session(signing("us-east-1").with_use_fips_endpoint(true)),
    );
    fips.adopt_region("eu-west-3").expect("a region");
    assert_eq!(
        fips.host_header("lake"),
        "lake.s3-fips.eu-west-3.amazonaws.com"
    );

    // A stated endpoint is where the store is whatever region signs.
    let stated = client(
        "s3://lake/part.parquet",
        sealed()
            .with_endpoint("http://localhost:9000")
            .with_region("us-east-1"),
    );
    stated.adopt_region("eu-west-3").expect("a region");
    assert_eq!(stated.region(), "eu-west-3");
    assert_eq!(stated.host_header("lake"), "localhost:9000");
}

#[test]
fn a_region_that_is_no_host_label_is_refused_naming_where_it_came_from_before_a_host_is_built() {
    let refusal = |options: S3Options| {
        Client::new(&url("s3://lake/part.parquet"), options)
            .err()
            .expect("a refusal")
            .to_string()
    };
    let stated = refusal(sealed().with_region("eu-west-3/"));
    assert!(stated.contains("eu-west-3/"), "{stated}");
    assert!(stated.contains("S3 options"), "{stated}");
    assert!(stated.contains("host label"), "{stated}");

    let ambient = refusal(
        sealed().with_session(
            Session::new()
                .with_environment(false)
                .with_region("x.example.org#"),
        ),
    );
    assert!(ambient.contains("x.example.org#"), "{ambient}");
    assert!(ambient.contains("AWS_REGION"), "{ambient}");
}

#[test]
fn the_backoff_doubles_and_stops_doubling() {
    assert_eq!(backoff(1), RETRY_BACKOFF);
    assert_eq!(backoff(2), RETRY_BACKOFF * 2);
    assert_eq!(backoff(3), RETRY_BACKOFF * 4);
    assert_eq!(backoff(20), RETRY_BACKOFF * 64);
}

#[test]
fn an_endpoint_splits_into_scheme_host_and_port() {
    assert_eq!(
        Client::split_endpoint("http://localhost:9000").expect("a split endpoint"),
        (
            "http".to_owned(),
            "localhost".to_owned(),
            Some(9000),
            String::new()
        )
    );
    assert_eq!(
        Client::split_endpoint("s3.example.io").expect("a split endpoint"),
        (
            "https".to_owned(),
            "s3.example.io".to_owned(),
            None,
            String::new()
        )
    );
    assert_eq!(
        Client::split_endpoint("https://[::1]:9000").expect("a split endpoint"),
        (
            "https".to_owned(),
            "[::1]".to_owned(),
            Some(9000),
            String::new()
        )
    );
    Client::split_endpoint("https://host:notaport").expect_err("a refused port");
}

#[test]
fn an_endpoint_is_read_once_as_the_url_it_is_and_only_where_the_store_is_is_kept() {
    for (endpoint, scheme, host, port, path) in [
        ("localhost:9000", "https", "localhost", Some(9000), ""),
        (
            "HTTP://minio.example.io:9000/",
            "http",
            "minio.example.io",
            Some(9000),
            "",
        ),
        ("[::1]:9000", "https", "[::1]", Some(9000), ""),
        ("http://[::1]", "http", "[::1]", None, ""),
        // User information and a query say nothing about where the store is,
        // so neither reaches the `Host` header or the signature.
        ("https://u:p@host:9000", "https", "host", Some(9000), ""),
        // A path is answered apart, its trailing `/` dropped: the prefix a
        // gateway mounts Amazon S3 below...
        ("https://host/prefix", "https", "host", None, "/prefix"),
        ("http://host:9000/p/?x=1", "http", "host", Some(9000), "/p"),
        // ... or the account the emulator spelling an Azure connection string
        // states names, which the client adds itself.
        (
            "http://127.0.0.1:10000/devstoreaccount1",
            "http",
            "127.0.0.1",
            Some(10000),
            "/devstoreaccount1",
        ),
    ] {
        assert_eq!(
            Client::split_endpoint(endpoint).expect(endpoint),
            (scheme.to_owned(), host.to_owned(), port, path.to_owned()),
            "{endpoint:?}"
        );
    }
}

#[test]
fn an_endpoint_that_names_no_host_or_a_port_that_is_no_number_is_refused_quoting_it() {
    for endpoint in [
        "https://host:notaport",
        "https://host:99999",
        "https://host:+80",
        "https://host:",
        "https://",
        "https://a@b@c",
        "",
    ] {
        let refused = Client::split_endpoint(endpoint)
            .expect_err(endpoint)
            .to_string();
        assert!(
            refused.contains("expected a host and an optional port in the S3 endpoint"),
            "{endpoint:?}: {refused}"
        );
        assert!(refused.contains(&format!("{endpoint:?}")), "{refused}");
    }
}

#[test]
fn the_path_style_variable_reads_the_one_boolean_table_and_text_it_does_not_spell_defers() {
    let host = |spelling: &str, config: &str| {
        client(
            "s3://trades/lake/part.parquet",
            ambient(
                &[
                    ("AWS_REGION", "eu-west-3"),
                    ("AWS_S3_FORCE_PATH_STYLE", spelling),
                ],
                config,
            ),
        )
        .host_header("trades")
    };
    let path = "s3.eu-west-3.amazonaws.com";
    let virtual_hosted = "trades.s3.eu-west-3.amazonaws.com";
    for spelling in ["true", "t", "tr", "yes", "y", "on", "1"] {
        assert_eq!(host(spelling, ""), path, "{spelling:?} forces path style");
    }
    for spelling in ["false", "f", "no", "n", "off", "0"] {
        assert_eq!(
            host(spelling, ""),
            virtual_hosted,
            "{spelling:?} forces virtual hosting"
        );
    }
    // Text no spelling reads states nothing: the profile's addressing style
    // answers, and without one the host's own default.
    assert_eq!(
        host("maybe", "[default]\ns3 =\n  addressing_style = path\n"),
        path
    );
    assert_eq!(host("maybe", ""), virtual_hosted);
}

#[test]
fn the_s3_table_s_switches_read_the_one_boolean_table_and_text_it_does_not_spell_is_off() {
    let host = |table: &str| {
        client(
            "s3://trades/lake/part.parquet",
            sealed().with_session(profiled(&format!(
                "[default]\nregion = eu-west-3\ns3 =\n  {table}\n"
            ))),
        )
        .host_header("trades")
    };
    assert_eq!(
        host("use_dualstack_endpoint = y"),
        "trades.s3.dualstack.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host("use_accelerate_endpoint = on"),
        "trades.s3-accelerate.amazonaws.com"
    );
    assert_eq!(
        host("use_dualstack_endpoint = maybe"),
        "trades.s3.eu-west-3.amazonaws.com"
    );
    assert_eq!(
        host("use_accelerate_endpoint = no"),
        "trades.s3.eu-west-3.amazonaws.com"
    );
}

#[test]
fn the_profiles_payload_signing_flag_reads_the_one_boolean_table_and_text_it_does_not_spell_states_nothing()
 {
    let signs = |endpoint: &str, spelling: &str| {
        client(
            "s3://trades/lake/part.parquet",
            ambient(
                &[("AWS_ENDPOINT_URL_S3", endpoint)],
                &format!(
                    "[default]\nregion = eu-west-3\ns3 =\n  payload_signing_enabled = {spelling}\n"
                ),
            ),
        )
        .signs_payload()
    };
    // Over plain HTTP the body is signed unless the profile says otherwise.
    for spelling in ["false", "f", "no", "n", "off", "0"] {
        assert!(!signs("http://localhost:9000", spelling), "{spelling:?}");
    }
    assert!(
        signs("http://localhost:9000", "maybe"),
        "text no spelling reads leaves the default, which signs over HTTP"
    );
    // Over TLS it is not, unless the profile says so.
    for spelling in ["true", "t", "tr", "yes", "y", "ye", "on", "1"] {
        assert!(signs("https://localhost:9000", spelling), "{spelling:?}");
    }
    assert!(
        !signs("https://localhost:9000", "maybe"),
        "and the default over TLS does not"
    );
}

mod accounting {
    use yggdryl::IOBase;

    use crate::mod_::{BUCKET, folder, payload, store};

    #[test]
    fn a_scan_that_reads_a_header_out_of_each_object_keeps_one_connection() {
        let store = store();
        for part in 0..6 {
            store.put(
                BUCKET,
                &format!("lake/{part:03}.parquet"),
                &payload(64 * 1024),
            );
        }
        let lake = folder(&store, "lake/");
        for leaf in lake.ls(false, false) {
            let leaf = leaf.expect("an entry");
            let mut stream = leaf.pstream_bytes(0, 4096).expect("a stream");
            let first = stream.next().expect("a chunk").expect("bytes");
            assert_eq!(first.len(), 4096);
            // The rest of the body is abandoned, which is what a header read does.
            drop(stream);
        }
        assert_eq!(
            store.connection_count(),
            1,
            "an abandoned body gives its connection back rather than burning it"
        );
    }
}

mod wire {

    use yggdryl::{Error, IOBase};

    use crate::mod_::{BUCKET, file, folder, options, path, payload, store};

    #[test]
    fn a_refused_signature_is_the_stores_own_verdict() {
        let store = store();
        store.require_access_key(Some("SOMEONEELSE"));
        let handle = file(&store, "lake/part.parquet");

        let error = handle.read_all_bytes().expect_err("a refusal");
        match &error {
            Error::Remote {
                service,
                operation,
                status,
                code,
                path,
                ..
            } => {
                assert_eq!(*service, "s3");
                assert_eq!(*operation, "GetObject");
                assert_eq!(*status, 403);
                assert_eq!(code.as_str(), "InvalidAccessKeyId");
                assert_eq!(path.as_str(), "s3://trades/lake/part.parquet");
            }
            other => panic!("expected a remote refusal, got {other:?}"),
        }
        // A refusal is neither an absence nor a conflict, so a caller repairing
        // one of those is never sent down the wrong branch.
        assert!(!error.is_absent());
        assert!(!error.is_conflict());
    }

    #[test]
    fn a_throttled_request_is_retried_and_a_refusal_is_not() {
        let store = store();
        store.put(BUCKET, "lake/part.parquet", b"PAR1");
        let handle = file(&store, "lake/part.parquet");

        // Two throttles, then the real answer: the default budget is three tries.
        store.fail_next(503, "SlowDown", 2);
        store.clear_requests();
        assert_eq!(handle.read_all_bytes().expect("the object"), b"PAR1");
        assert_eq!(store.request_count(), 3, "two retries and the answer");

        // A refusal is the store's verdict, not a hiccup, so it is not retried.
        store.fail_next(403, "AccessDenied", 1);
        store.clear_requests();
        handle.read_all_bytes().expect_err("a refusal");
        assert_eq!(
            store.request_count(),
            1,
            "a refusal stands on the first answer"
        );
    }

    #[test]
    fn many_reads_share_one_connection_rather_than_reconnecting() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", &payload(4096));
        let handle = file(&store, "lake/part.bin");

        // Ranged reads, whole reads, and streams in turn: every one of them has
        // to leave its connection reusable.
        for _ in 0..10 {
            assert_eq!(handle.read_range_bytes(0, 64).expect("a range").len(), 64);
            assert_eq!(handle.read_all_bytes().expect("the object").len(), 4096);
            assert_eq!(handle.pstream_bytes(0, 1024).expect("a stream").count(), 4);
        }

        assert_eq!(store.request_count(), 30);
        // The number that matters: on a real store each extra connection is a TCP
        // and TLS handshake, which would dwarf the transfer a footer read makes.
        assert_eq!(
            store.connection_count(),
            1,
            "thirty requests went down one connection"
        );
    }

    #[test]
    fn a_refusal_is_never_read_as_an_empty_prefix_or_an_absent_object() {
        let store = store();
        store.put(BUCKET, "lake/part.bin", b"AAPL,187.23");

        // A listing nobody was allowed to see says nothing about what is there,
        // so a non-recursive removal must refuse rather than report success.
        let mut lake = folder(&store, "lake/");
        store.fail_next(403, "AccessDenied", 1);
        let refused = lake.remove(false).expect_err("a refusal");
        assert!(!refused.is_absent(), "{refused}");
        assert_eq!(
            store.keys(BUCKET),
            vec!["lake/part.bin".to_owned()],
            "and nothing was deleted"
        );

        // The same for a location: absence reads as emptiness, a refusal does not.
        let handle = path(&store, "lake/part.bin");
        store.fail_next(403, "AccessDenied", 1);
        let refused = handle.read_all_bytes().expect_err("a refusal");
        assert!(
            matches!(refused, Error::Remote { status: 403, .. }),
            "{refused}"
        );

        let missing = path(&store, "lake/absent.bin");
        assert_eq!(
            missing.read_all_bytes().expect("absence reads empty"),
            Vec::<u8>::new()
        );
    }

    /// A `409` or `412` is a conflict only where the store's code says the
    /// thing being created is there: a write that asked nothing of the key is
    /// told what the store said, and a bucket create reads the bucket codes.
    #[test]
    fn a_409_or_412_is_a_conflict_only_by_the_code_that_says_so() {
        let store = store();
        let mut handle = file(&store, "lake/part.parquet");
        for (status, code) in [(412, "PreconditionFailed"), (409, "OperationAborted")] {
            store.fail_next(status, code, 1);
            store.clear_requests();
            let error = handle.write_all_bytes(b"PAR1").expect_err("a refusal");
            assert!(
                matches!(
                    &error,
                    Error::Remote { status: answered, code: named, operation: "PutObject", .. }
                        if *answered == status && named == code
                ),
                "{error:?}"
            );
            assert!(!error.is_conflict(), "{error}");
            assert_eq!(store.request_count(), 1, "a verdict is not retried");
        }

        // Another account's bucket of the name is the bucket a create found;
        // the bucket this account owns already is the create done; a `409`
        // naming anything else is the store's own refusal.
        let root =
            yggdryl::s3::folder_with("s3://fresh/", options(&store)).expect("a bucket handle");
        store.fail_next(409, "BucketAlreadyExists", 1);
        let taken = root.create().expect_err("a name another account holds");
        assert!(matches!(taken, Error::Conflict { .. }), "{taken}");
        assert!(taken.to_string().contains("s3://fresh/"), "{taken}");
        store.fail_next(409, "BucketAlreadyOwnedByYou", 1);
        root.create()
            .expect("owning the bucket already is the create done");
        store.fail_next(409, "OperationAborted", 1);
        let aborted = root.create().expect_err("a conflicting operation");
        assert!(
            matches!(&aborted, Error::Remote { status: 409, code, .. } if code == "OperationAborted"),
            "{aborted:?}"
        );
    }
}
