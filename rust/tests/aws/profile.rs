//! `rust/src/aws/profile.rs`: the shared files `~/.aws/config` and
//! `~/.aws/credentials`, read the way the AWS tools read them.
//!
//! A machine's own files would exercise none of these edges reliably, and a
//! test may not read them, so every file here is text the test spells: handed
//! to a session through `with_config_text` and `with_credentials_text` for what
//! a caller reaches through `Profile`, and to `Files` and the legacy readers
//! through `yggdryl::internals::aws_profile` for what it cannot.

use std::path::{Path, PathBuf};
use std::time::Duration;

use yggdryl::aws::{CredentialSource, Credentials, Profile, Session, Sso};
use yggdryl::internals::aws_profile::{
    Files, boto_config, ec2_credential_file, expand_user, service_key, split_words,
};

/// The profile `name` two spelled files hold, read through a session that
/// consults nothing else.
fn read_profile(config: &str, credentials: &str, name: &str) -> Option<Profile> {
    Session::new()
        .with_environment(false)
        .with_config_text(config)
        .with_credentials_text(credentials)
        .with_profile(name)
        .profile()
}

/// The profile `name` the configuration file alone holds.
fn configured(config: &str, name: &str) -> Profile {
    read_profile(config, "", name).unwrap_or_else(|| panic!("the configuration file spells {name}"))
}

/// The text of a refusal, which is invalid data naming the profile.
fn refusal<T: std::fmt::Debug>(answer: yggdryl::Result<T>) -> String {
    let error = answer.expect_err("a refusal");
    assert!(
        matches!(&error, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidData),
        "a profile's refusal is invalid data: {error:?}"
    );
    error.to_string()
}

mod sections {
    use super::*;

    #[test]
    fn the_configuration_file_prefixes_every_profile_but_default_and_a_bare_name_is_no_profile() {
        const CONFIG: &str = "\
[default]
region = us-east-1

[profile trading]
region = eu-west-3

[trading]
region = ap-south-1

[plugins]
cli_legacy_plugin_path = /opt/plugins
";
        let default = configured(CONFIG, "default");
        assert_eq!(default.name(), "default");
        assert_eq!(
            default.region(),
            Some("us-east-1"),
            "[default] is the default profile"
        );

        let trading = configured(CONFIG, "trading");
        assert_eq!(trading.name(), "trading");
        assert_eq!(
            trading.region(),
            Some("eu-west-3"),
            "a bare [trading] in the configuration file is not the profile, and its keys stay out of it"
        );
        assert_eq!(
            trading.get("cli_legacy_plugin_path"),
            None,
            "a section nothing reads keeps its own keys"
        );
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["default", "trading"],
            "neither the bare section nor [plugins] is a profile"
        );
    }

    #[test]
    fn profile_default_and_default_name_one_profile_and_the_later_value_wins() {
        const CONFIG: &str = "\
[default]
region = us-east-1
output = json

[profile default]
region = eu-west-3
";
        let default = configured(CONFIG, "default");
        assert_eq!(
            default.region(),
            Some("eu-west-3"),
            "the later spelling's value wins"
        );
        assert_eq!(
            default.output(),
            Some("json"),
            "a key only the first spells is kept"
        );
        assert_eq!(Files::parse(Some(CONFIG), None).names(), ["default"]);
    }

    #[test]
    fn a_quoted_profile_name_keeps_its_space_and_a_header_of_three_words_is_no_profile() {
        const CONFIG: &str = "\
[profile \"my name\"]
region = eu-west-3

[profile 'single quoted']
output = text

[ profile   spaced ]
region = us-west-2

[profile two words]
region = us-east-1
";
        let quoted = configured(CONFIG, "my name");
        assert_eq!(quoted.name(), "my name");
        assert_eq!(quoted.region(), Some("eu-west-3"));
        assert_eq!(configured(CONFIG, "single quoted").output(), Some("text"));
        assert_eq!(
            configured(CONFIG, "spaced").region(),
            Some("us-west-2"),
            "whitespace around and inside a header separates words"
        );
        assert!(
            read_profile(CONFIG, "", "two words").is_none(),
            "an unquoted name of two words is no profile"
        );
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["my name", "single quoted", "spaced"]
        );
    }

    #[test]
    fn the_credentials_file_names_every_section_bare() {
        const CREDENTIALS: &str = "\
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = default-secret

[trading]
aws_access_key_id = AKIATRADING
aws_secret_access_key = trading-secret

[profile desk]
aws_access_key_id = AKIADESK
aws_secret_access_key = desk-secret
";
        let trading =
            read_profile("", CREDENTIALS, "trading").expect("a bare section is a profile");
        assert_eq!(
            trading.credentials(),
            Some(Credentials::new("AKIATRADING", "trading-secret"))
        );
        assert_eq!(
            read_profile("", CREDENTIALS, "default").and_then(|default| default.credentials()),
            Some(Credentials::new("AKIADEFAULT", "default-secret"))
        );
        assert!(
            read_profile("", CREDENTIALS, "desk").is_none(),
            "the credentials file has no profile prefix"
        );
        assert_eq!(
            Files::parse(None, Some(CREDENTIALS)).names(),
            ["default", "profile desk", "trading"],
            "[profile desk] in the credentials file is a profile of that whole name"
        );
    }

    #[test]
    fn sso_session_and_services_sections_are_no_profiles() {
        const CONFIG: &str = "\
[profile dev]
region = eu-west-3

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1

[services local]
s3 =
  endpoint_url = http://localhost:9000
";
        assert_eq!(Files::parse(Some(CONFIG), None).names(), ["dev"]);
        assert!(read_profile(CONFIG, "", "corp").is_none());
        assert!(read_profile(CONFIG, "", "local").is_none());
    }

    #[test]
    fn files_list_every_profile_either_file_holds_in_name_order() {
        const CONFIG: &str = "[default]\nregion = us-east-1\n[profile b]\nregion = eu-west-3\n";
        const CREDENTIALS: &str =
            "[a]\naws_access_key_id = AKIAA\naws_secret_access_key = s\n[default]\noutput = json\n";
        let files = Files::parse(Some(CONFIG), Some(CREDENTIALS));
        assert_eq!(files.names(), ["a", "b", "default"]);
        let session = Session::new()
            .with_environment(false)
            .with_config_text(CONFIG)
            .with_credentials_text(CREDENTIALS);
        assert_eq!(
            session.available_profiles(),
            files.names(),
            "a session lists what the files hold"
        );
        assert_eq!(
            files
                .profile("default")
                .and_then(|default| default.output().map(str::to_owned)),
            Some("json".to_owned()),
            "the default profile is laid over from the credentials file"
        );

        let empty = Files::parse(None, None);
        assert!(empty.names().is_empty(), "no file, no profile");
        assert!(
            empty.profile("default").is_none(),
            "no file, not even the default"
        );
        assert!(
            read_profile(CONFIG, CREDENTIALS, "nobody").is_none(),
            "a profile nobody wrote is absent, not an error"
        );
    }
}

mod lines {
    use super::*;

    #[test]
    fn keys_fold_to_lower_case_values_keep_theirs_and_either_delimiter_splits() {
        const CONFIG: &str = "\
[profile desk]
Region: eu-west-3
OUTPUT = json
endpoint_url: http://localhost:4566
role_session_name = PowerDesk
Credential_Process = /opt/bin/keys --profile desk
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.get("region"), Some("eu-west-3"), "a colon delimits");
        assert_eq!(desk.get("REGION"), Some("eu-west-3"), "a lookup folds too");
        assert_eq!(desk.region(), Some("eu-west-3"));
        assert_eq!(desk.output(), Some("json"));
        assert_eq!(
            desk.endpoint_url(),
            Some("http://localhost:4566"),
            "the first delimiter splits, so a URL keeps its own colon"
        );
        assert_eq!(
            desk.get("role_session_name"),
            Some("PowerDesk"),
            "a value keeps its case"
        );
        assert_eq!(
            desk.credential_process(),
            Some("/opt/bin/keys --profile desk")
        );
    }

    #[test]
    fn comments_and_lines_before_any_section_are_skipped() {
        const CONFIG: &str = "\
region = before-any-section
# a comment
; another comment
[profile desk]
  # an indented comment
region = eu-west-3
; region = us-east-1
# output = json
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(
            desk.region(),
            Some("eu-west-3"),
            "a commented value is no value"
        );
        assert_eq!(desk.output(), None);
        assert_eq!(
            Files::parse(Some(CONFIG), None).names(),
            ["desk"],
            "a line before any header belongs to no profile"
        );
    }

    #[test]
    fn a_later_duplicate_key_wins_and_a_repeated_section_extends_the_first() {
        const CONFIG: &str = "\
[profile desk]
region = us-east-1
region = eu-west-3

[profile desk]
output = json
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.region(), Some("eu-west-3"), "the later value wins");
        assert_eq!(
            desk.output(),
            Some("json"),
            "the section spelled again adds to it"
        );
    }

    #[test]
    fn an_indented_block_is_a_table_under_its_key_and_an_unindented_key_closes_it() {
        const CONFIG: &str = "\
[profile desk]
region = eu-west-3
s3 =
  addressing_style = path
  Use_Accelerate_Endpoint: true
\tpayload_signing_enabled = false
output = json
  stray = belongs to nothing
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(desk.s3("addressing_style"), Some("path"));
        assert_eq!(
            desk.s3("ADDRESSING_STYLE"),
            Some("path"),
            "a nested lookup folds"
        );
        assert_eq!(
            desk.nested("S3", "use_accelerate_endpoint"),
            Some("true"),
            "a nested key folds and takes a colon"
        );
        assert_eq!(
            desk.s3("payload_signing_enabled"),
            Some("false"),
            "a tab indents"
        );
        assert_eq!(desk.get("s3"), None, "a table is no top-level value");
        assert_eq!(
            desk.get("addressing_style"),
            None,
            "a nested key stays nested"
        );
        assert_eq!(
            desk.output(),
            Some("json"),
            "an unindented key closes the table"
        );
        assert_eq!(
            desk.get("stray"),
            None,
            "an indented line under a value is skipped"
        );
        assert_eq!(desk.s3("stray"), None, "and never reopens the closed table");
        assert_eq!(
            desk.nested("region", "anything"),
            None,
            "a value is no table"
        );
        assert_eq!(desk.nested("dynamodb", "anything"), None, "an absent table");
    }

    #[test]
    fn an_indented_key_right_after_a_header_is_a_key_as_the_aws_parser_reads_it() {
        let profile = Session::new()
            .with_environment(false)
            .with_config_text("[profile desk]\n  region = eu-west-3\n  output = json\n")
            .with_profile("desk")
            .profile()
            .expect("the profile");
        assert_eq!(profile.region(), Some("eu-west-3"));
        assert_eq!(profile.output(), Some("json"));
    }

    #[test]
    fn a_profile_iterates_its_values_in_key_order_and_reads_its_flags() {
        const CONFIG: &str = "\
[profile desk]
use_fips_endpoint = True
use_dualstack_endpoint = false
region = eu-west-3
s3 =
  addressing_style = path
output = json
experimental = maybe
nothing =
";
        let desk = configured(CONFIG, "desk");
        assert_eq!(
            desk.iter().collect::<Vec<_>>(),
            [
                ("experimental", "maybe"),
                ("output", "json"),
                ("region", "eu-west-3"),
                ("use_dualstack_endpoint", "false"),
                ("use_fips_endpoint", "True"),
            ],
            "tables and empty values are no values"
        );
        assert_eq!(desk.get("nothing"), None, "an empty value is absent");
        assert_eq!(
            desk.flag("use_fips_endpoint"),
            Some(true),
            "true in any case"
        );
        assert_eq!(desk.flag("USE_DUALSTACK_ENDPOINT"), Some(false));
        assert_eq!(
            desk.flag("experimental"),
            Some(false),
            "an unknown spelling is false"
        );
        assert_eq!(desk.flag("missing"), None, "an absent flag is no answer");
    }
}

mod services {
    use super::*;

    #[test]
    fn the_services_section_a_profile_names_answers_each_service_endpoint() {
        const CONFIG: &str = "\
[profile local]
services = local-stack
endpoint_url = http://localhost:4566

[services local-stack]
s3 =
  endpoint_url = http://localhost:9000
secrets_manager =
  endpoint_url = http://localhost:9001
sso_oidc =
  endpoint_url = http://localhost:9002
dynamodb =
  region = us-west-2

[profile elsewhere]
services = nowhere

[profile plain]
region = eu-west-3
";
        let local = configured(CONFIG, "local");
        assert_eq!(
            local.endpoint_url(),
            Some("http://localhost:4566"),
            "every service's"
        );
        assert_eq!(
            local.service_endpoint_url("s3"),
            Some("http://localhost:9000")
        );
        assert_eq!(
            local.service_endpoint_url("S3"),
            Some("http://localhost:9000"),
            "a service id folds"
        );
        assert_eq!(
            local.service_endpoint_url("Secrets Manager"),
            Some("http://localhost:9001"),
            "a space is spelled as an underscore"
        );
        assert_eq!(
            local.service_endpoint_url("sso-oidc"),
            Some("http://localhost:9002"),
            "a hyphen is spelled as an underscore"
        );
        assert_eq!(
            local.service_endpoint_url("dynamodb"),
            None,
            "a service entry without endpoint_url states none"
        );
        assert_eq!(
            local.service_endpoint_url("sts"),
            None,
            "a service with no entry"
        );
        assert_eq!(
            configured(CONFIG, "elsewhere").service_endpoint_url("s3"),
            None,
            "a services name no section defines"
        );
        assert_eq!(configured(CONFIG, "plain").service_endpoint_url("s3"), None);
    }

    #[test]
    fn a_service_key_is_the_service_id_folded_with_underscores() {
        assert_eq!(service_key("S3"), "s3");
        assert_eq!(service_key("Secrets Manager"), "secrets_manager");
        assert_eq!(service_key("sso-oidc"), "sso_oidc");
        assert_eq!(service_key(" DynamoDB "), "dynamodb", "trimmed first");
    }
}

mod credentials {
    use super::*;

    #[test]
    fn the_credentials_file_wins_over_the_configuration_file_for_one_profile() {
        const CONFIG: &str = "\
[profile desk]
region = us-east-1
output = json
aws_access_key_id = AKIACONFIG
aws_secret_access_key = config-secret
";
        const CREDENTIALS: &str = "\
[desk]
region = eu-west-3
aws_access_key_id = AKIACREDENTIALS
aws_secret_access_key = credentials-secret
";
        let desk = read_profile(CONFIG, CREDENTIALS, "desk").expect("both files spell desk");
        assert_eq!(
            desk.region(),
            Some("eu-west-3"),
            "the credentials file's value wins"
        );
        assert_eq!(
            desk.output(),
            Some("json"),
            "a key only the configuration file spells stays"
        );
        assert_eq!(
            desk.credentials(),
            Some(Credentials::new("AKIACREDENTIALS", "credentials-secret")),
            "the credentials file's pair wins"
        );
        assert_eq!(
            configured(CONFIG, "desk").credentials(),
            Some(Credentials::new("AKIACONFIG", "config-secret")),
            "the configuration file's pair stands alone"
        );
    }

    #[test]
    fn a_pair_carries_its_session_token_under_either_name_and_its_account() {
        const CREDENTIALS: &str = "\
[modern]
aws_access_key_id = AKIAMODERN
aws_secret_access_key = modern-secret
aws_session_token = modern-token
aws_account_id = 123456789012

[legacy]
aws_access_key_id = AKIALEGACY
aws_secret_access_key = legacy-secret
aws_security_token = legacy-token

[both]
aws_access_key_id = AKIABOTH
aws_secret_access_key = both-secret
aws_security_token = security-token
aws_session_token = session-token

[half]
aws_access_key_id = AKIAHALF
region = eu-west-3
";
        let modern =
            read_profile("", CREDENTIALS, "modern").and_then(|modern| modern.credentials());
        assert_eq!(
            modern,
            Some(
                Credentials::new("AKIAMODERN", "modern-secret")
                    .with_session_token("modern-token")
                    .with_account_id("123456789012")
            )
        );
        assert!(
            modern.is_some_and(|keys| keys.is_temporary()),
            "a token is a temporary set"
        );

        let legacy =
            read_profile("", CREDENTIALS, "legacy").and_then(|legacy| legacy.credentials());
        assert_eq!(
            legacy.as_ref().and_then(Credentials::session_token),
            Some("legacy-token"),
            "aws_security_token is the session token's older name"
        );

        let both = read_profile("", CREDENTIALS, "both").and_then(|both| both.credentials());
        assert_eq!(
            both.as_ref().and_then(Credentials::session_token),
            Some("session-token"),
            "aws_session_token wins over its older name"
        );

        let half = read_profile("", CREDENTIALS, "half").expect("a profile with half a pair");
        assert_eq!(
            half.credentials(),
            None,
            "a key id without its secret is no pair"
        );
        assert_eq!(half.region(), Some("eu-west-3"));
    }
}

mod roles {
    use super::*;

    const ARN: &str = "arn:aws:iam::123456789012:role/lake-reader";

    #[test]
    fn a_role_profile_names_its_source_and_every_parameter_of_the_exchange() {
        let config = format!(
            "\
[profile chained]
role_arn = {ARN}
source_profile = base
role_session_name = power-desk
external_id = desk-7
mfa_serial = arn:aws:iam::123456789012:mfa/trader
duration_seconds = 7200

[profile instance]
role_arn = {ARN}
credential_source = Ec2InstanceMetadata

[profile container]
role_arn = {ARN}
credential_source = ecscontainer

[profile environment]
role_arn = {ARN}
credential_source = Environment

[profile federated]
role_arn = {ARN}
web_identity_token_file = /var/run/secrets/token

[profile plain]
region = eu-west-3
"
        );
        let chained = configured(&config, "chained")
            .assumed_role()
            .expect("a well-formed role")
            .expect("a role");
        assert_eq!(chained.role_arn(), ARN);
        assert_eq!(chained.source_profile(), Some("base"));
        assert_eq!(chained.credential_source(), None);
        assert_eq!(chained.web_identity_token_file(), None);
        assert_eq!(chained.session_name(), Some("power-desk"));
        assert_eq!(chained.external_id(), Some("desk-7"));
        assert_eq!(
            chained.mfa_serial(),
            Some("arn:aws:iam::123456789012:mfa/trader")
        );
        assert_eq!(chained.duration(), Duration::from_secs(7200));

        let source = |name: &str| {
            configured(&config, name)
                .assumed_role()
                .expect("a well-formed role")
                .and_then(|role| role.credential_source())
        };
        assert_eq!(
            source("instance"),
            Some(CredentialSource::Ec2InstanceMetadata)
        );
        assert_eq!(
            source("container"),
            Some(CredentialSource::EcsContainer),
            "a source is read in any case"
        );
        assert_eq!(source("environment"), Some(CredentialSource::Environment));

        let federated = configured(&config, "federated")
            .assumed_role()
            .expect("a well-formed role")
            .expect("a role");
        assert_eq!(
            federated.web_identity_token_file(),
            Some(Path::new("/var/run/secrets/token")),
            "a web identity token is a source of its own"
        );
        assert_eq!(federated.source_profile(), None);

        assert!(
            configured(&config, "plain")
                .assumed_role()
                .expect("no role is no refusal")
                .is_none(),
            "a profile without role_arn assumes nothing"
        );
    }

    #[test]
    fn a_role_profile_is_refused_with_two_sources_with_none_or_with_a_bad_duration() {
        let config = format!(
            "\
[profile both]
role_arn = {ARN}
source_profile = base
credential_source = Environment

[profile sourceless]
role_arn = {ARN}

[profile hourly]
role_arn = {ARN}
source_profile = base
duration_seconds = an hour

[profile laptop]
role_arn = {ARN}
credential_source = Laptop
"
        );
        let both = refusal(configured(&config, "both").assumed_role());
        assert!(both.contains("the profile both"), "{both}");
        assert!(
            both.contains("names both source_profile and credential_source"),
            "{both}"
        );

        let sourceless = refusal(configured(&config, "sourceless").assumed_role());
        assert!(
            sourceless.contains("the profile sourceless"),
            "{sourceless}"
        );
        assert!(
            sourceless
                .contains("without source_profile, credential_source or web_identity_token_file"),
            "{sourceless}"
        );

        let hourly = refusal(configured(&config, "hourly").assumed_role());
        assert!(hourly.contains("the profile hourly"), "{hourly}");
        assert!(hourly.contains("duration_seconds"), "{hourly}");
        assert!(
            hourly.contains("\"an hour\""),
            "the value is quoted: {hourly}"
        );

        let laptop = refusal(configured(&config, "laptop").assumed_role());
        assert!(laptop.contains("the profile laptop"), "{laptop}");
        assert!(laptop.contains("credential_source"), "{laptop}");
        assert!(
            laptop.contains("Laptop"),
            "the unknown source is named: {laptop}"
        );
    }
}

mod sso {
    use super::*;

    #[test]
    fn a_profile_signs_in_through_its_sso_session_or_the_legacy_keys() {
        const CONFIG: &str = "\
[profile dev]
sso_session = corp
sso_account_id = 123456789012
sso_role_name = LakeReader
region = eu-west-3

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1
sso_registration_scopes = sso:account:access, codewhisperer:completions

[profile legacy]
sso_start_url = https://legacy.awsapps.com/start
sso_region = us-east-1
sso_account_id = 210987654321
sso_role_name = Auditor

[profile plain]
region = eu-west-3
";
        let dev = configured(CONFIG, "dev");
        assert_eq!(dev.sso_session_name(), Some("corp"));
        assert_eq!(
            dev.sso().expect("a complete sign-in"),
            Some(
                Sso::new(
                    "https://corp.awsapps.com/start",
                    "eu-west-1",
                    "123456789012",
                    "LakeReader"
                )
                .with_session_name("corp")
                .with_scopes(["sso:account:access", "codewhisperer:completions"])
            ),
            "the start URL, region and scopes come from the session, the account and role from the profile"
        );
        assert_eq!(
            dev.region(),
            Some("eu-west-3"),
            "the profile's region is its own"
        );

        let legacy = configured(CONFIG, "legacy");
        assert_eq!(legacy.sso_session_name(), None);
        let sign_in = legacy
            .sso()
            .expect("a complete sign-in")
            .expect("a sign-in");
        assert_eq!(
            sign_in,
            Sso::new(
                "https://legacy.awsapps.com/start",
                "us-east-1",
                "210987654321",
                "Auditor"
            )
        );
        assert_eq!(
            sign_in.session_name(),
            None,
            "the legacy shape names no session"
        );
        assert!(
            sign_in.scopes().is_empty(),
            "the legacy shape registers no scopes"
        );

        assert_eq!(
            configured(CONFIG, "plain")
                .sso()
                .expect("no sign-in is no refusal"),
            None
        );
    }

    #[test]
    fn a_sign_in_is_refused_naming_each_missing_key_and_an_undefined_session() {
        const CONFIG: &str = "\
[profile ghost]
sso_session = ghost
sso_account_id = 123456789012
sso_role_name = LakeReader

[profile roleless]
sso_session = corp
sso_account_id = 123456789012

[profile unregioned]
sso_session = regionless
sso_account_id = 123456789012
sso_role_name = LakeReader

[profile started]
sso_start_url = https://legacy.awsapps.com/start

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = eu-west-1

[sso-session regionless]
sso_start_url = https://corp.awsapps.com/start
";
        let ghost = configured(CONFIG, "ghost");
        assert_eq!(ghost.sso_session_name(), None, "no section, no session");
        let message = refusal(ghost.sso());
        assert!(
            message.contains(
                "the profile ghost names sso_session ghost, which no [sso-session ghost] section defines"
            ),
            "{message}"
        );

        let message = refusal(configured(CONFIG, "roleless").sso());
        assert!(message.contains("the profile roleless"), "{message}");
        assert!(message.contains("without sso_role_name"), "{message}");
        assert!(
            !message.contains("sso_account_id"),
            "only what is missing: {message}"
        );

        let message = refusal(configured(CONFIG, "unregioned").sso());
        assert!(message.contains("without sso_region"), "{message}");

        let message = refusal(configured(CONFIG, "started").sso());
        assert!(
            message.contains("without sso_region, sso_account_id, sso_role_name"),
            "every missing key, in order: {message}"
        );
    }
}

mod words {
    use super::*;

    #[test]
    fn whitespace_separates_words_and_nothing_is_no_word() {
        assert_eq!(
            split_words("aws  sso\tlogin\n --profile desk"),
            ["aws", "sso", "login", "--profile", "desk"]
        );
        assert!(split_words("").is_empty());
        assert!(
            split_words(" \t ").is_empty(),
            "whitespace alone is no word"
        );
    }

    #[test]
    fn quotes_keep_whitespace_and_join_what_touches_them() {
        assert_eq!(
            split_words("'single quoted' \"double quoted\""),
            ["single quoted", "double quoted"]
        );
        assert_eq!(
            split_words(r#"pre'fix'ed "con"cat"#),
            ["prefixed", "concat"],
            "a quoted part joins the word it touches"
        );
        assert_eq!(
            split_words("'' x \"\""),
            ["", "x", ""],
            "an empty quote is a word"
        );
        assert_eq!(
            split_words("\"unterminated quote"),
            ["unterminated quote"],
            "an unclosed quote runs to the end"
        );
        assert_eq!(
            split_words(r#""$HOME" '$HOME'"#),
            ["$HOME", "$HOME"],
            "nothing is expanded"
        );
    }

    #[test]
    fn a_backslash_escapes_outside_quotes_a_few_inside_double_quotes_and_none_inside_single() {
        assert_eq!(
            split_words(r"a\ b c\\d"),
            ["a b", r"c\d"],
            "outside quotes a backslash keeps the next character"
        );
        assert_eq!(
            split_words(r#"'keeps \ and "'"#),
            [r#"keeps \ and ""#],
            "single quotes keep everything"
        );
        assert_eq!(
            split_words(r#""escapes \" \\ but not \$ \` \n""#),
            [r#"escapes " \ but not \$ \` \n"#],
            "double quotes escape only a quote and a backslash, as Python's shlex does"
        );
        assert_eq!(
            split_words(r"trailing\"),
            ["trailing"],
            "a final backslash escapes nothing"
        );
    }
}

mod legacy {
    use super::*;

    #[test]
    fn a_leading_tilde_is_the_home_directory_and_nothing_else_is_expanded() {
        let home = Path::new("/home/trader");
        assert_eq!(
            expand_user("~/.aws/config", Some(home)),
            PathBuf::from("/home/trader/.aws/config")
        );
        assert_eq!(expand_user("~", Some(home)), PathBuf::from("/home/trader"));
        assert_eq!(
            expand_user("~trader/.aws/config", Some(home)),
            PathBuf::from("~trader/.aws/config"),
            "another user's home is not guessed"
        );
        assert_eq!(
            expand_user("/etc/aws/config", Some(home)),
            PathBuf::from("/etc/aws/config")
        );
        assert_eq!(
            expand_user("config/~/x", Some(home)),
            PathBuf::from("config/~/x"),
            "only a leading tilde"
        );
        assert_eq!(
            expand_user("~/.aws/config", None),
            PathBuf::from("~/.aws/config"),
            "no home, no expansion"
        );
    }

    #[test]
    fn the_ec2_credential_file_spells_one_pair_one_key_per_line() {
        let text =
            "# the EC2 tools' file\nAWSAccessKeyId=AKIAEC2\nAWSSecretKey = ec2/secret+key=\n";
        assert_eq!(
            ec2_credential_file(text),
            Some(Credentials::new("AKIAEC2", "ec2/secret+key=")),
            "values are trimmed and split at the first equals sign"
        );
        assert_eq!(
            ec2_credential_file("AWSAccessKeyId=AKIAEC2\n"),
            None,
            "a key id without its secret"
        );
        assert_eq!(
            ec2_credential_file("AWSAccessKeyId=AKIAEC2\nAWSSecretKey=\n"),
            None,
            "an empty secret is no secret"
        );
        assert_eq!(
            ec2_credential_file("awsaccesskeyid=AKIAEC2\nawssecretkey=s\n"),
            None,
            "the two names are spelled exactly"
        );
        assert_eq!(ec2_credential_file(""), None);
    }

    #[test]
    fn a_boto_configuration_reads_its_credentials_section_alone() {
        let text = "\
[Boto]
debug = 0

[Credentials]
aws_access_key_id = AKIABOTO
aws_secret_access_key = boto-secret
";
        assert_eq!(
            boto_config(text),
            Some(Credentials::new("AKIABOTO", "boto-secret"))
        );
        assert_eq!(
            boto_config("[credentials]\nAWS_ACCESS_KEY_ID = AKIALOWER\naws_secret_access_key: s\n"),
            Some(Credentials::new("AKIALOWER", "s")),
            "the section name in any case, keys folded, either delimiter"
        );
        assert_eq!(
            boto_config("[Boto]\naws_access_key_id = AKIABOTO\naws_secret_access_key = s\n"),
            None,
            "a pair in another section is not the boto pair"
        );
        assert_eq!(boto_config(""), None);
    }
}
