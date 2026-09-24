//! `rust/src/auth/mod.rs`: the four shared pieces are reachable together.

#[test]
fn the_shared_pieces_are_one_vocabulary() {
    use yggdryl::internals::auth_environment::Environment;
    use yggdryl::internals::auth_lease::{Expiring, Lease};
    use yggdryl::internals::auth_report::Report;
    use yggdryl::internals::auth_secret::Secret;

    #[derive(Clone)]
    struct Token(Secret);
    impl Expiring for Token {
        fn expires_at(&self) -> Option<std::time::SystemTime> {
            None
        }
    }

    let token = Token(Secret::new("s3cr3t"));
    assert_eq!(token.0.expose(), "s3cr3t");
    assert_eq!(token.expires_at(), None);
    let lease: Lease<Token> = Lease::new(
        "token",
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(1),
    );
    assert!(lease.peek().is_none());
    let report = Report::new("credentials");
    assert!(
        report
            .conclude::<Token>()
            .expect("nothing failed")
            .is_none()
    );
    assert_eq!(Environment::Given(Default::default()).get("PATH"), None);
}
