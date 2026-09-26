//! `rust/src/http/netrc.rs`: the credentials a user keeps per host.
//!
//! The grammar and the lookup read text in hand; the file lookup reads a
//! path a stand-in `NETRC` names, so no test reads the home directory of the
//! machine it runs on.

use yggdryl::http::Authorization;
use yggdryl::internals::http_netrc::{authorization, environment_authorization};

const NETRC: &str = r#"
# Comments run to the end of their line.
machine api.example.com login alice password s3cret
machine other.example.com
    login bob
    account ignored
    password "with space \" and quote"
macdef init
    cd /pub
    get file

default login anonymous password guest@
"#;

#[test]
fn a_machine_entry_answers_its_host_as_a_basic_credential() {
    assert_eq!(
        authorization(NETRC, "api.example.com"),
        Some(Authorization::basic("alice", "s3cret"))
    );
    assert_eq!(
        authorization(NETRC, "API.Example.COM"),
        Some(Authorization::basic("alice", "s3cret"))
    );
    assert_eq!(
        authorization(NETRC, "other.example.com"),
        Some(Authorization::basic("bob", "with space \" and quote"))
    );
}

#[test]
fn any_other_host_takes_default_and_without_it_nothing() {
    assert_eq!(
        authorization(NETRC, "unknown.example.org"),
        Some(Authorization::basic("anonymous", "guest@"))
    );
    let no_default = "machine api.example.com login alice password s3cret";
    assert_eq!(authorization(no_default, "unknown.example.org"), None);
    // A subdomain is another host: no suffix matching.
    assert_eq!(authorization(no_default, "v2.api.example.com"), None);
    assert_eq!(authorization("", "api.example.com"), None);
    // An entry naming neither a login nor a password is no credential.
    assert_eq!(authorization("machine bare.example.com", "bare.example.com"), None);
}

#[test]
fn the_file_netrc_names_is_read_and_reread_when_it_changes() {
    let directory = std::env::temp_dir().join(format!("yggdryl-netrc-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("credentials");
    std::fs::write(&path, "machine h.example login one password first").unwrap();
    let variables = [("NETRC", path.to_str().unwrap())];
    assert_eq!(
        environment_authorization("h.example", &variables),
        Some(Authorization::basic("one", "first"))
    );
    // A longer file is another version of it, read again.
    std::fs::write(&path, "machine h.example login two password second-longer").unwrap();
    assert_eq!(
        environment_authorization("h.example", &variables),
        Some(Authorization::basic("two", "second-longer"))
    );
    let missing = directory.join("absent");
    assert_eq!(
        environment_authorization("h.example", &[("NETRC", missing.to_str().unwrap())]),
        None
    );
    std::fs::remove_dir_all(&directory).unwrap();
}
