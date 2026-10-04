//! Pins `rust/src/hostname.rs`: the machine's name, read once and spelled as
//! a URL host.

use yggdryl::{Authority, HOSTNAME, Url};

#[test]
fn the_host_is_read_once_and_spells_a_url_host() -> yggdryl::Result<()> {
    assert!(!HOSTNAME.is_empty());
    assert!(std::ptr::eq(HOSTNAME.as_str(), HOSTNAME.as_str()));
    assert_eq!(HOSTNAME.as_str(), HOSTNAME.to_ascii_lowercase());
    assert!(!HOSTNAME.starts_with(['.', '-']) && !HOSTNAME.ends_with(['.', '-']));
    assert_eq!(
        Authority::from_str(HOSTNAME.as_str())?.host(),
        HOSTNAME.as_str()
    );
    let url = Url::from_str(&format!(
        "file://{}/bucket/trades.parquet",
        HOSTNAME.as_str()
    ))?;
    assert_eq!(url.hostname(), Some(HOSTNAME.as_str()));
    // Read as this machine beside `localhost`.
    assert!(Authority::from_str(HOSTNAME.as_str())?.is_this_machine());
    Ok(())
}

#[test]
fn the_host_is_the_operating_systems_own_name() {
    // What `hostname(1)` prints, spelled as the constant spells it.
    let Ok(output) = std::process::Command::new("hostname").output() else {
        return;
    };
    let reported = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    if reported.is_empty()
        || !reported
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return;
    }
    assert_eq!(HOSTNAME.as_str(), reported);
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::hostname::host;

    #[test]
    fn a_name_no_host_spells_is_spelled_as_one() {
        assert_eq!(host("Build-Agent.example.COM"), "build-agent.example.com");
        assert_eq!(host("  trader_01 \n"), "trader_01");
        assert_eq!(host("Trading Desk's Mac"), "trading-desk-s-mac");
        // A byte of a character outside ASCII is a hyphen of its own.
        assert_eq!(host("poste-été"), "poste---t");
        assert_eq!(host(".-host-."), "host");
        assert_eq!(host("a:b@c/d"), "a-b-c-d");
        assert_eq!(host(&"x".repeat(300)).len(), 253);
    }

    #[test]
    fn nothing_a_host_spells_is_localhost() {
        assert_eq!(host(""), "localhost");
        assert_eq!(host("   "), "localhost");
        assert_eq!(host("---"), "localhost");
        assert_eq!(host("..."), "localhost");
    }
}
