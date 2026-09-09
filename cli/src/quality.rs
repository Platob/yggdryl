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
    /// How many code records were walked.
    pub codes: usize,
    /// How many lineage entries were walked.
    pub entries: usize,
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
        codes: 0,
        entries: 0,
    };
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
                        detail: "states no fix:tag, so it cannot be written back".to_owned(),
                    }),
                    Err(error) => report.findings.push(Finding {
                        level: Level::Fail,
                        check: "tag",
                        subject: named.clone(),
                        detail: format!("states a tag that will not read: {error}"),
                    }),
                }
            }

            // A borrowed walk ends at a refusal, so one malformed record removes
            // every record after it from resolution - silently.
            for code in view.codes() {
                match code {
                    Ok(_) => report.codes += 1,
                    Err(error) => {
                        report.findings.push(Finding {
                            level: Level::Fail,
                            check: "codes",
                            subject: named.clone(),
                            detail: format!(
                                "its code set stops at {error} - every code after it is invisible"
                            ),
                        });
                        break;
                    }
                }
            }
            for entry in view.lineage() {
                match entry {
                    Ok(_) => report.entries += 1,
                    Err(error) => {
                        report.findings.push(Finding {
                            level: Level::Fail,
                            check: "lineage",
                            subject: named.clone(),
                            detail: format!("its lineage stops at {error}"),
                        });
                        break;
                    }
                }
            }

            // A lineage whose newest entry names something else is a lineage
            // about another field.
            let newest = view
                .lineage()
                .filter_map(std::result::Result::ok)
                .filter_map(yggdryl::FixLineageEntry::name)
                .last();
            if let Some(newest) = newest {
                if !newest.eq_ignore_ascii_case(field.name()) {
                    report.findings.push(Finding {
                        level: Level::Warn,
                        check: "lineage",
                        subject: named.clone(),
                        detail: format!("its newest lineage entry names {newest:?}"),
                    });
                }
            }

            duplicated_codes(&mut report, field, &named);
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

/// A code set with two records at one value cannot answer either.
fn duplicated_codes(report: &mut Report, field: &Field, named: &str) {
    let mut values: Vec<String> = Vec::new();
    for code in field.as_fix().codes().filter_map(std::result::Result::ok) {
        let held = code.value().to_owned();
        if values.contains(&held) {
            report.findings.push(Finding {
                level: Level::Warn,
                check: "codes",
                subject: named.to_owned(),
                detail: format!("declares {held:?} twice"),
            });
        } else {
            values.push(held);
        }
    }
}

/// A group with no item struct is a group nothing can be read out of.
fn shaped_group(report: &mut Report, field: &Field, named: &str) {
    let Some(item) = list_item(field) else {
        return;
    };
    if item.dtype().as_fields().is_none() {
        report.findings.push(Finding {
            level: Level::Warn,
            check: "groups",
            subject: named.to_owned(),
            detail: "is a list whose item is not a struct".to_owned(),
        });
    }
}

/// A list field's item, where it is one.
fn list_item(field: &Field) -> Option<&Field> {
    match field.dtype() {
        yggdryl::DataType::List(item) | yggdryl::DataType::LargeList(item) => Some(item),
        _ => None,
    }
}

/// Prints a report the way a person reads it.
pub fn render(report: &Report) {
    style::heading("checked");
    for (category, count) in &report.categories {
        style::entry(category.as_str(), &count.to_string());
    }
    style::entry("codes", &report.codes.to_string());
    style::entry("lineage entries", &report.entries.to_string());

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
