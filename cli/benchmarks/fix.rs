//! CLI process boundary: help baseline and category reads including registry I/O.

use std::hint::black_box;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixCategory, FixCode, FixRegistry};

struct Fixture(PathBuf, PathBuf);

impl Fixture {
    fn new() -> yggdryl::Result<Self> {
        let temporary = std::fs::canonicalize(Folder::temporary()?.path()?)?;
        let path = temporary.join(format!("ygg-cli-fix-bench-{}", std::process::id()));
        std::fs::create_dir(&path)?;
        let fixture = Self(path, temporary);
        let mut registry = FixRegistry::new();
        for tag in 1..=100 {
            let mut field = DataType::Int32.nullable_field(format!("Field{tag}"));
            field.as_fix_mut().set_tag(tag)?;
            if tag == 54 {
                field
                    .as_fix_mut()
                    .set_codes(&[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")])?;
            }
            registry.create_definition(FixCategory::Fields, field)?;
        }
        let party = DataType::from_fields([DataType::utf8().nullable_field("PartyID")])?
            .required_field("Party");
        registry.create_definition(FixCategory::Components, party.clone())?;
        let mut group = DataType::list(party).nullable_field("Parties");
        group.as_fix_mut().set_counter(1)?;
        group.as_fix_mut().set_component("Party")?;
        registry.create_definition(FixCategory::Groups, group)?;
        let mut message = DataType::from_fields([DataType::utf8().nullable_field("ClOrdID")])?
            .required_field("Order");
        message.as_fix_mut().set_msgtype("D")?;
        registry.create_definition(FixCategory::Components, message)?;
        registry.write_into(&mut Folder::new(fixture.0.clone())?)?;
        Ok(fixture)
    }

    fn measure(&self, label: &str, args: &[&str]) -> std::io::Result<()> {
        let iterations = if cfg!(debug_assertions) { 1 } else { 20 };
        let start = Instant::now();
        for _ in 0..iterations {
            let output = Command::new(env!("CARGO_BIN_EXE_ygg"))
                .args(["fix", "--root"])
                .arg(&self.0)
                .args(args)
                .env("NO_COLOR", "1")
                .output()?;
            assert!(
                output.status.success(),
                "{label}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            black_box(output.stdout);
        }
        println!(
            "{label}: {:.3} ms/op ({iterations} processes)",
            start.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations)
        );
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let target = std::fs::canonicalize(&self.0).expect("resolve owned benchmark fixture");
        assert_eq!(target, self.0, "benchmark fixture target changed");
        assert_eq!(
            target.parent(),
            Some(self.1.as_path()),
            "benchmark fixture escaped its parent"
        );
        std::fs::remove_dir_all(target).expect("remove owned benchmark fixture");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    fixture.measure("help baseline", &["--help"])?;
    for (category, name) in [
        ("fields", "54"),
        ("components", "Order"),
        ("components", "Party"),
        ("groups", "Parties"),
    ] {
        fixture.measure(category, &[category, "read", name, "--json"])?;
    }
    Ok(())
}
