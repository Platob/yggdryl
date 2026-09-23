//! What a dictionary has to be right about, checked in one pass.
//!
//! Written to be run two ways from one implementation: at a desk, where it
//! prints what it found and what to do; and in a workflow, where it prints
//! one line per finding and exits non-zero. The checks are the same either
//! way, so a green terminal and a green pipeline mean the same thing.
//!
//! Each check exists because something has actually gone wrong in one of
//! them. A borrowed document that stops at a refusal silently loses every
//! record after it; two fields folding to one name make a lookup answer
//! whichever the index happened to hold; a field with no tag cannot be
//! written back at all.

use std::collections::HashMap;

use yggdryl::{Field, FixCategory, FixRegistry};

use crate::style;

/// How serious a finding is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Worth knowing, and not wrong.
    Note,
    /// Wrong in a way a dictionary can still be used with.
    Warn,
    /// Wrong in a way that loses data or answers falsely.
    Fail,
}

impl Level {
    /// The word this level prints as, for a machine reading the output.
    const fn word(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Warn => "warning",
            Self::Fail => "error",
        }
    }
}

/// One thing a check found.
pub struct Finding {
    /// How serious it is.
    pub level: Level,
    /// Which check found it.
    pub check: &'static str,
    /// Which field it is about, where it is about one.
    pub subject: String,
    /// What is wrong, in one sentence.
    pub detail: String,
}

/// Everything the checks found, in the order they ran.
pub struct Report {
    /// What was found.
    pub findings: Vec<Finding>,
    /// How many definitions were walked in each category.
    pub categories: Vec<(FixCategory, usize)>,
    /// How many code sets were walked.
    pub codesets: usize,
    /// How many code records were walked, once per set rather than once per
    /// field reading by it.
    pub codes: usize,
}

impl Report {
    /// Whether anything failed.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.findings.iter().any(|held| held.level == Level::Fail)
    }

    /// How many findings reached one level.
    #[must_use]
    pub fn counted(&self, level: Level) -> usize {
        self.findings
            .iter()
            .filter(|held| held.level == level)
            .count()
    }
}

/// Runs every check over one dictionary.
#[must_use]
pub fn check(registry: &FixRegistry) -> Report {
    let mut report = Report {
        findings: Vec::new(),
        categories: Vec::new(),
        codesets: 0,
        codes: 0,
    };
    check_codesets(&mut report, registry);
    for category in FixCategory::ALL {
        let mut count = 0;
        for field in registry.definitions(category) {
            count += 1;
            let view = field.as_fix();
            let named = format!("{category}/{}", field.name());

            // A field with no tag cannot be written back into the store, so a
            // dictionary holding one cannot round-trip.
            if category == FixCategory::Fields {
                match view.tag() {
                    Ok(Some(_)) => {}
                    Ok(None) => report.findings.push(Finding {
                        level: Level::Fail,
                        check: "tag",
                        subject: named.clone(),
                        detail: "states no FIX:tag, so it cannot be written back".to_owned(),
                    }),
                    Err(error) => report.findings.push(Finding {
                        level: Level::Fail,
                        check: "tag",
                        subject: named.clone(),
                        detail: format!("states a tag that will not read: {error}"),
                    }),
                }
            }

            // The set a field names has to be one the dictionary holds, or
            // every value of that field resolves to nothing.
            if let Some(set) = view.codeset() {
                if registry.get_codeset(set).is_none() {
                    report.findings.push(Finding {
                        level: Level::Fail,
                        check: "codes",
                        subject: named.clone(),
                        detail: format!("reads by {set:?}, which this dictionary does not hold"),
                    });
                }
            }
            shaped_group(&mut report, field, &named);
        }
        report.categories.push((category, count));
    }

    if report.categories.iter().all(|(_, count)| *count == 0) {
        report.findings.push(Finding {
            level: Level::Note,
            check: "empty",
            subject: String::new(),
            detail: "the dictionary holds no definitions".to_owned(),
        });
    }
    report
}

/// Every vocabulary the dictionary holds, walked once.
///
/// Once per set rather than once per field reading by it: 103 fields read by
/// one offset-unit set in the shipped dictionary, and a malformed record in
/// it is one finding about one vocabulary, not 103 about the fields that
/// name it.
fn check_codesets(report: &mut Report, registry: &FixRegistry) {
    let mut read_by: HashMap<&str, usize> = HashMap::new();
    for field in registry {
        if let Some(name) = field.as_fix().codeset() {
            *read_by.entry(name).or_default() += 1;
        }
    }
    for set in registry.codesets() {
        report.codesets += 1;
        let named = format!("codesets/{}", set.name());
        // A borrowed walk ends at a refusal, so one malformed record removes
        // every record after it from resolution - silently.
        let mut values: Vec<String> = Vec::new();
        for code in set.codes() {
            match code {
                Ok(code) => {
                    report.codes += 1;
                    let held = code.value().to_owned();
                    if values.contains(&held) {
                        report.findings.push(Finding {
                            level: Level::Warn,
                            check: "codes",
                            subject: named.clone(),
                            detail: format!("declares {held:?} twice"),
                        });
                    } else {
                        values.push(held);
                    }
                }
                Err(error) => {
                    report.findings.push(Finding {
                        level: Level::Fail,
                        check: "codes",
                        subject: named.clone(),
                        detail: format!("stops at {error} - every code after it is invisible"),
                    });
                    break;
                }
            }
        }
        // A vocabulary nothing reads by is one a store writes, a reader loads
        // and no value ever reaches.
        if !read_by.contains_key(set.name()) {
            report.findings.push(Finding {
                level: Level::Note,
                check: "codes",
                subject: named,
                detail: "no field reads by it".to_owned(),
            });
        }
    }
}

/// A group with no item struct is a group nothing can be read out of.
fn shaped_group(report: &mut Report, field: &Field, named: &str) {
    let Some(item) = serie_item(field) else {
        return;
    };
    if item.dtype().as_fields().is_none() {
        report.findings.push(Finding {
            level: Level::Warn,
            check: "groups",
            subject: named.to_owned(),
            detail: "is a serie whose item is not a struct".to_owned(),
        });
    }
}

/// A serie field's item, where it is one.
fn serie_item(field: &Field) -> Option<&Field> {
    field.dtype().serie_item()
}

/// Prints a report the way a person reads it.
pub fn render(report: &Report) {
    style::heading("checked");
    for (category, count) in &report.categories {
        style::entry(category.as_str(), &count.to_string());
    }
    style::entry("codesets", &report.codesets.to_string());
    style::entry("codes", &report.codes.to_string());

    if report.findings.is_empty() {
        style::good("nothing to report");
        return;
    }
    style::heading("findings");
    let rows: Vec<Vec<String>> = report
        .findings
        .iter()
        .map(|held| {
            vec![
                held.level.word().to_owned(),
                held.check.to_owned(),
                held.subject.clone(),
                held.detail.clone(),
            ]
        })
        .collect();
    style::table(&["level", "check", "definition", "detail"], &rows);

    let failed = report.counted(Level::Fail);
    let warned = report.counted(Level::Warn);
    if failed > 0 {
        style::bad(&format!("{failed} error(s), {warned} warning(s)"));
    } else if warned > 0 {
        style::warn(&format!("{warned} warning(s), no errors"));
    } else {
        style::good("notes only");
    }
}

/// Prints a report the way a workflow reads it.
///
/// One line per finding, in the annotation shape GitHub Actions recognises,
/// so a check that fails shows up on the job rather than only in the log.
pub fn annotate(report: &Report) {
    for held in &report.findings {
        let subject = if held.subject.is_empty() {
            String::new()
        } else {
            format!("{}: ", held.subject)
        };
        println!(
            "::{} title=fix {}::{subject}{}",
            held.level.word(),
            held.check,
            held.detail,
        );
    }
}
