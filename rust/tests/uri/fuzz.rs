//! Randomized identifier invariants.
//!
//! The corpus in `encoding.rs` pins the cases a specification names. This pins
//! the ones nobody thought to name: a deterministic generator builds identifier
//! text out of the fragments that make percent-encoding hard - bare and short
//! escapes, escaped separators and dots, doubled percents, non-UTF-8 octets,
//! zone markers, drive and UNC prefixes - and every value that parses has to
//! satisfy the properties the crate promises about all of them.
//!
//! The generator is a seeded xorshift, so a failure names one seed and one
//! iteration and reproduces exactly. It is a property test rather than a
//! coverage-guided fuzzer: it runs in the ordinary suite and costs a second.

use std::collections::BTreeMap;

use yggdryl::{Authority, Error, Scheme, Uri, UriPath, Url, Urn};

/// Iterations per seed. Ten seeds cover roughly four hundred thousand values
/// against the release build; this size keeps the debug suite under a second.
const ITERATIONS: usize = 4_000;
const SEEDS: [u64; 10] = [
    0x2545_F491_4F6C_DD1D,
    0x0000_0000_0000_0001,
    0x9E37_79B9_7F4A_7C15,
    0xDEAD_BEEF_CAFE_F00D,
    0x1234_5678_9ABC_DEF0,
    0xFFFF_FFFF_FFFF_FFFF,
    0x0F0F_0F0F_0F0F_0F0F,
    0xA5A5_5A5A_A5A5_5A5A,
    0x7FFF_FFFF_FFFF_FFFF,
    0x1357_9BDF_2468_ACE0,
];

/// The fragments identifier text is built from.
///
/// Every entry is one thing a real implementation has been caught mishandling:
/// a bare or short escape, an escape standing for a separator or a dot, a
/// doubled percent, an octet that is not UTF-8, a zone marker, a drive or UNC
/// prefix, a delimiter, or a plain name byte to glue them together.
const ATOMS: &[&str] = &[
    "%",
    "%2",
    "%25",
    "%2F",
    "%2f",
    "%5C",
    "%00",
    "%41",
    "%2E",
    "%2e",
    "%7E",
    "%c3%a9",
    "%C0%AF",
    "%ED%A0%80",
    "%FF",
    "%7F",
    "%%",
    "%zz",
    "%2525",
    "%3A",
    "%3F",
    "%23",
    "%40",
    "%20",
    "a",
    "b",
    "0",
    "-",
    ".",
    "..",
    "/",
    "//",
    "///",
    ":",
    "@",
    "?",
    "#",
    "&",
    "=",
    "+",
    " ",
    "~",
    "!",
    "*",
    "'",
    "(",
    ")",
    ";",
    ",",
    "$",
    "\"",
    "|",
    "[",
    "]",
    "\\",
    "\\\\",
    "\t",
    "\u{7f}",
    "\u{e9}",
    "\u{30a2}",
    "file:",
    "https:",
    "urn:",
    "s3:",
    "C:",
    "c:",
    "localhost",
    "fe80::1",
    "%25eth0",
    "[fe80::1%25eth0]",
    "server",
    "share",
    "lake",
    "100%",
    ".csv",
];

/// Names that stand in for a real file name, joined through the platform door.
const NAMES: &[&str] = &[
    "100%.csv",
    "%20.txt",
    "a%b",
    "report %2F final.pdf",
    "50%25discount",
    "%",
    "%%",
    "%2",
    "caf\u{e9}.csv",
    "a b.csv",
    "a#b",
    "a?b",
    "a&b",
    "a=b",
    "a:b",
    "a@b",
    "~x",
    "A",
    "%41",
    "...",
    "..bar",
    ".hidden",
    "year=2024",
    "a+b",
    "a;b",
    "a,b",
    "a[b]",
    "a|b",
    "\u{30a2}",
];

struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.0 = state;
        state
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).unwrap_or(0)
    }

    fn pick<'a>(&mut self, values: &[&'a str]) -> &'a str {
        values[self.below(values.len())]
    }
}

/// Everything one iteration can find wrong, collected rather than panicked on
/// so one run reports the whole class instead of its first member.
#[derive(Default)]
struct Failures {
    entries: Vec<String>,
}

impl Failures {
    fn check(&mut self, holds: bool, message: impl FnOnce() -> String) {
        if !holds {
            self.entries.push(message());
        }
    }

    /// Assert a reported byte offset addresses the input it came from.
    ///
    /// An offset past the end, or one splitting a character, is a defect on its
    /// own: it is what a caller slices the input with to show the failure.
    fn located_in(&mut self, label: &str, input: &str, error: &Error) {
        let Error::Parse { position, .. } = error else {
            return;
        };
        self.check(*position <= input.len(), || {
            format!("{label}: byte {position} is past the end of {input:?}")
        });
        self.check(input.is_char_boundary((*position).min(input.len())), || {
            format!("{label}: byte {position} splits a character in {input:?}")
        });
    }
}

/// The schemes the structured generator spells, one per shape it reaches.
const SCHEMES: &[&str] = &["https", "file", "s3", "urn", "postgres", "a"];

/// Build one piece of identifier text out of the hard fragments.
///
/// Flat soup finds what nobody would write; the structured shape finds what a
/// caller does write, and is what keeps the `urn` and `s3` paths reached often
/// enough for their properties to mean anything.
fn generate(rng: &mut Rng) -> String {
    if rng.below(2) == 0 {
        let mut text = String::new();
        for _ in 0..=rng.below(7) {
            text.push_str(rng.pick(ATOMS));
        }
        return text;
    }

    let scheme = rng.pick(SCHEMES);
    let mut text = String::from(scheme);
    text.push(':');
    if scheme == "urn" {
        // A URN is `nid:nss` with no authority, so spelling one at random
        // reaches the namespace rules; anything else never gets past them.
        text.push_str(rng.pick(&["example", "isbn", "uuid", "a-b", "x"]));
        text.push(':');
        for _ in 0..=rng.below(3) {
            text.push_str(rng.pick(ATOMS));
        }
        if rng.below(3) == 0 {
            text.push_str(rng.pick(&["?=", "?+"]));
            text.push_str(rng.pick(ATOMS));
        }
        if rng.below(4) == 0 {
            text.push('#');
            text.push_str(rng.pick(ATOMS));
        }
        return text;
    }
    if rng.below(3) != 0 {
        text.push_str("//");
        for _ in 0..rng.below(3) {
            text.push_str(rng.pick(ATOMS));
        }
    }
    for _ in 0..rng.below(4) {
        text.push('/');
        text.push_str(rng.pick(ATOMS));
    }
    if rng.below(3) == 0 {
        text.push('?');
        for _ in 0..=rng.below(3) {
            text.push_str(rng.pick(ATOMS));
            text.push('=');
            text.push_str(rng.pick(ATOMS));
            text.push('&');
        }
        text.pop();
    }
    if rng.below(4) == 0 {
        text.push('#');
        text.push_str(rng.pick(ATOMS));
    }
    text
}

/// Every property a parsed `Uri` owes, whatever text produced it.
fn check_uri(uri: &Uri, source: &str, failures: &mut Failures) {
    // Display is the canonical spelling, so re-parsing it is the identity and
    // the second spelling equals the first. This is the invariant the `url`
    // crate's own fuzzing found `file://` URLs breaking.
    let rendered = uri.to_string();
    match Uri::from_str(&rendered) {
        Ok(again) => {
            failures.check(&again == uri, || {
                format!("{source:?} -> {rendered:?} re-parsed as a different value")
            });
            failures.check(again.to_string() == rendered, || {
                format!("{source:?} -> {rendered:?} is not its own canonical spelling")
            });
        }
        Err(error) => failures.entries.push(format!(
            "{source:?} -> {rendered:?} does not re-parse: {error}"
        )),
    }

    // Canonical hex case: no escape survives with a lowercase digit.
    for component in [
        uri.path().as_str(),
        uri.authority().as_str(),
        uri.query(false)
            .unwrap_or_default()
            .unwrap_or_default()
            .as_ref(),
        uri.fragment(false)
            .unwrap_or_default()
            .unwrap_or_default()
            .as_ref(),
    ] {
        let bytes = component.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte != b'%' {
                continue;
            }
            let digits = bytes.get(index + 1..index + 3).unwrap_or_default();
            failures.check(!digits.iter().any(u8::is_ascii_lowercase), || {
                format!("{source:?} kept a lowercase escape in {component:?}")
            });
        }
    }

    // `file:` has one spelling for an absolute path, so the marker is not
    // optional: `file:/data` and `file:///data` cannot both be values.
    failures.check(
        uri.scheme() != &Scheme::FILE || uri.has_authority() || !uri.path().is_absolute(),
        || format!("{source:?} is an absolute file path without its authority marker"),
    );

    // Structure is read from the encoded text, so resolving `.` and `..` only
    // ever removes names and never invents one.
    failures.check(uri.parts().len() <= uri.path().segment_len(), || {
        format!("{source:?} resolved to more parts than it has segments")
    });

    // The structural JSON is the same value.
    if let Ok(json) = uri.clone().into_json() {
        match Uri::from_json(&json) {
            Ok(back) => failures.check(&back == uri, || {
                format!("{source:?} JSON is a different value")
            }),
            Err(error) => failures
                .entries
                .push(format!("{source:?} JSON does not read back: {error}")),
        }
    }

    // Decoding is text, not structure: it may reveal separators inside a
    // segment but must never be refused for a component that parsed clean.
    if let Ok(text) = uri.path_text(true) {
        failures.check(text.len() <= uri.path().as_str().len(), || {
            format!("{source:?} decoded to a longer path than it stores")
        });
    }
}

/// Every property the platform-path conversions owe each other.
///
/// A URI is not required to be the *only* spelling of the path it converts to -
/// `file:///a/%41` and `file:///a/A` name one file - so the round trip is stated
/// as a fixed point: whatever URI a converted path spells, converting that one
/// again lands on itself. The exact-text direction, path to URI and back, is
/// asserted where a path is the input.
fn check_path_conversions(uri: &Uri, source: &str, failures: &mut Failures) {
    let Ok(path) = uri.clone().into_path() else {
        return;
    };
    let text = path.to_string_lossy().into_owned();

    // A converted path never carries structure the URI's own path did not
    // spell: no dot segment an escape created, and no leading `//` naming a
    // host the URI has no authority for.
    failures.check(
        !text.starts_with("//") || !uri.authority().is_empty(),
        || format!("{source:?} -> {text:?} names a UNC server without an authority"),
    );
    // The authority is spelled back at the front as `//host`, so the path is
    // whatever follows it; a host is the authority's business, not the guard's.
    let body = if uri.authority().is_empty() {
        text.as_str()
    } else {
        text.get(2..)
            .and_then(|rest| rest.find('/').map(|at| &rest[at..]))
            .unwrap_or_default()
    };
    let dot_segments = |value: &str| {
        value
            .split('/')
            .filter(|segment| matches!(*segment, "." | ".."))
            .count()
    };
    failures.check(
        dot_segments(body)
            <= uri
                .path()
                .segments()
                .filter(|segment| matches!(*segment, "." | ".."))
                .count(),
        || format!("{source:?} -> {text:?} gained a dot segment the URI path does not spell"),
    );

    // Converting back and forth again is the identity on whatever it landed on.
    let Ok(again) = Uri::from_path(&text) else {
        failures
            .entries
            .push(format!("{source:?} -> {text:?} cannot be converted back"));
        return;
    };
    match again.clone().into_path().map(Uri::from_path) {
        Ok(Ok(third)) => failures.check(third == again, || {
            format!("{source:?} -> {again} -> {third} is not a fixed point")
        }),
        Ok(Err(error)) | Err(error) => failures.entries.push(format!(
            "{source:?} -> {again} does not convert again: {error}"
        )),
    }
}

#[test]
fn generated_identifiers_hold_every_component_invariant() {
    let mut failures = Failures::default();
    let mut parsed: BTreeMap<&str, usize> = BTreeMap::new();

    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for _ in 0..ITERATIONS {
            let input = generate(&mut rng);

            match Uri::from_str(&input) {
                Ok(uri) => {
                    *parsed.entry("uri").or_default() += 1;
                    check_uri(&uri, &input, &mut failures);
                    check_path_conversions(&uri, &input, &mut failures);
                }
                Err(error) => failures.located_in("Uri::from_str", &input, &error),
            }

            match Uri::from_path(&input) {
                Ok(uri) => {
                    *parsed.entry("path").or_default() += 1;
                    check_uri(&uri, &input, &mut failures);
                    check_path_conversions(&uri, &input, &mut failures);

                    // A path is data, so the URI it produces gives it back byte
                    // for byte. Only the spellings the conversion is documented
                    // to canonicalize differ: separators, repeated roots, and a
                    // Windows drive, which is why they are excluded here rather
                    // than weakened into a looser comparison.
                    let canonicalized =
                        input.contains('\\') || input.starts_with("//") || input.starts_with("///");
                    if let Ok(back) = uri.clone().into_path() {
                        let text = back.to_string_lossy().into_owned();
                        let drive_shaped = text.as_bytes().get(1) == Some(&b':');
                        failures.check(canonicalized || drive_shaped || text == input, || {
                            format!("{input:?} -> {uri} -> {text:?} is not the path it started as")
                        });
                    }
                }
                Err(error) => failures.located_in("Uri::from_path", &input, &error),
            }

            match UriPath::from_str(&input) {
                Ok(path) => {
                    *parsed.entry("uri-path").or_default() += 1;
                    let walked = path.parts().len();
                    failures.check(walked <= path.segment_len(), || {
                        format!("{input:?} resolved to more parts than segments")
                    });
                    if let Ok(normalized) = path.normalize() {
                        failures.check(normalized.is_absolute() == path.is_absolute(), || {
                            format!("{input:?} changed rootedness under normalize")
                        });
                    }
                }
                Err(error) => failures.located_in("UriPath::from_str", &input, &error),
            }

            match Authority::from_str(&input) {
                Ok(authority) => {
                    *parsed.entry("authority").or_default() += 1;
                    let host = authority.host();
                    failures.check(authority.as_str().contains(host), || {
                        format!("{input:?} answered a host it does not contain")
                    });
                }
                Err(error) => failures.located_in("Authority::from_str", &input, &error),
            }

            if let Ok(url) = Url::from_str(&input) {
                *parsed.entry("url").or_default() += 1;

                // A fragment is set from text, so reading it back answers that
                // text however many escapes the spelling needed.
                let mut edited = url.clone();
                if edited.set_fragment(Some(&input)).is_ok() {
                    let read = edited.fragment(true).unwrap_or_default();
                    failures.check(read.as_deref() == Some(input.as_str()), || {
                        format!("{input:?} did not read back out of a fragment: {read:?}")
                    });
                }

                // Query pairs are a fixed point through one write.
                if let Ok(pairs) = url.parameters(true) {
                    let before: Vec<(String, String)> = pairs
                        .iter()
                        .map(|(key, value)| (key.to_owned(), value.to_owned()))
                        .collect();
                    let mut written = url.clone();
                    if written.set_parameters(&pairs.into_owned()).is_ok() {
                        let after: Vec<(String, String)> = written
                            .parameters(true)
                            .map(|read| {
                                read.iter()
                                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                                    .collect()
                            })
                            .unwrap_or_default();
                        failures.check(before == after, || {
                            format!("{input:?} query pairs are not a fixed point: {before:?} -> {after:?}")
                        });
                    }
                }
            }

            if let Ok(urn) = Urn::from_str(&input) {
                *parsed.entry("urn").or_default() += 1;
                let namespace = urn.namespace();
                failures.check(
                    !namespace.is_empty() && !urn.namespace_specific().is_empty(),
                    || format!("{input:?} parsed as a URN with an empty half"),
                );
            }
        }
    }

    // The generator has to keep reaching every entry point often enough for
    // the properties above to mean something, so the floor is asserted rather
    // than assumed: a shape that stops parsing takes its properties with it.
    for (entry, floor) in [
        ("uri", 5_000),
        ("path", 5_000),
        ("uri-path", 2_000),
        ("authority", 500),
        ("url", 500),
        ("urn", 500),
    ] {
        assert!(
            parsed.get(entry).copied().unwrap_or_default() >= floor,
            "the generator reached too few valid {entry} values: {parsed:?}"
        );
    }

    let shown: Vec<&String> = failures.entries.iter().take(20).collect();
    assert!(
        failures.entries.is_empty(),
        "{} failed properties, first {}:\n{shown:#?}",
        failures.entries.len(),
        shown.len()
    );
}

/// A name joined through the platform door comes back as that name.
///
/// `join_path` takes an operating-system component, so whatever a file system
/// accepts has to survive the encoding: this is the property .NET's
/// `System.Uri` and Node's `new URL(name, 'file:')` break on a literal `%`.
#[test]
fn every_generated_file_name_survives_a_platform_join() {
    let mut failures = Failures::default();
    let lake = Url::from_str("file:///lake").unwrap();

    let mut rng = Rng::new(0xBADC_0FFE_E0DD_F00D);
    for iteration in 0..20_000 {
        let name = if iteration % 2 == 0 {
            rng.pick(NAMES).to_owned()
        } else {
            let mut built = String::new();
            for _ in 0..=rng.below(3) {
                built.push_str(rng.pick(ATOMS));
            }
            built
        };
        // A component a platform path cannot carry is not a name to begin with.
        if name.is_empty()
            || name.contains(['/', '\\', '\0'])
            || name.chars().any(char::is_control)
            || matches!(name.as_str(), "." | "..")
            || name.starts_with(':')
        {
            continue;
        }

        let joined = match lake.join_path(&name) {
            Ok(joined) => joined,
            Err(error) => {
                failures
                    .entries
                    .push(format!("{name:?} could not be joined: {error}"));
                continue;
            }
        };

        failures.check(joined.path().segment_len() == 2, || {
            format!(
                "{name:?} became {} segments in {joined}",
                joined.path().segment_len()
            )
        });
        failures.check(joined.file_name().is_some(), || {
            format!("{name:?} lost its file name in {joined}")
        });
        match joined.clone().into_path() {
            Ok(path) => failures.check(path == *format!("/lake/{name}"), || {
                format!("{name:?} came back as {path:?} through {joined}")
            }),
            Err(error) => failures.entries.push(format!(
                "{name:?} joined to {joined} but does not convert back: {error}"
            )),
        }
    }

    let shown: Vec<&String> = failures.entries.iter().take(20).collect();
    assert!(
        failures.entries.is_empty(),
        "{} names did not survive, first {}:\n{shown:#?}",
        failures.entries.len(),
        shown.len()
    );
}
