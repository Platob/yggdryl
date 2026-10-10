#!/usr/bin/env python3
"""The table's keys become clones of the instrument's own `Str` (D42.19)."""
import sys
from pathlib import Path

path = Path("/home/user/yggdryl/rust/market/src/instrument.rs")
text = path.read_text()
edits = [
    ("    isins: &'t mut HashMap<i128, SmolStr>,", "    isins: &'t mut HashMap<i128, Str>,"),
    ("    fn list(&mut self, key: IndexKey<'_>, code: &SmolStr) {", "    fn list(&mut self, key: IndexKey<'_>, code: &Str) {"),
    ("    fn list_all(&mut self, instrument: &Instrument, code: &SmolStr) {", "    fn list_all(&mut self, instrument: &Instrument, code: &Str) {"),
    ("    fn moved(&mut self, code: &SmolStr, held: &Instrument, next: &Instrument) {", "    fn moved(&mut self, code: &Str, held: &Instrument, next: &Instrument) {"),
    ("    One(&'t SmolStr),\n    Several(Vec<SmolStr>),", "    One(&'t Str),\n    Several(Vec<Str>),"),
    ("    fn of(mut codes: impl Iterator<Item = &'t SmolStr>) -> Self {", "    fn of(mut codes: impl Iterator<Item = &'t Str>) -> Self {"),
    ("    fn one(self) -> Option<&'t SmolStr> {", "    fn one(self) -> Option<&'t Str> {"),
    ("    answers: HashMap<EconomicReading, Option<SmolStr>>,", "    answers: HashMap<EconomicReading, Option<Str>>,"),
    ("    rows: Arc<BTreeMap<SmolStr, Instrument>>,\n    /// Each ISIN held - real or minted, packed - to the code holding it.\n    isins: Arc<HashMap<i128, SmolStr>>,\n    /// Each code an instrument had before a re-key to the code it has.\n    aliases: Arc<HashMap<SmolStr, SmolStr>>,",
     "    rows: Arc<BTreeMap<Str, Instrument>>,\n    /// Each ISIN held - real or minted, packed - to the code holding it.\n    isins: Arc<HashMap<i128, Str>>,\n    /// Each code an instrument had before a re-key to the code it has.\n    aliases: Arc<HashMap<Str, Str>>,"),
    ("static EMPTY_ROWS: LazyLock<Arc<BTreeMap<SmolStr, Instrument>>> = LazyLock::new(Arc::default);\nstatic EMPTY_ISINS: LazyLock<Arc<HashMap<i128, SmolStr>>> = LazyLock::new(Arc::default);\nstatic EMPTY_ALIASES: LazyLock<Arc<HashMap<SmolStr, SmolStr>>> = LazyLock::new(Arc::default);",
     "static EMPTY_ROWS: LazyLock<Arc<BTreeMap<Str, Instrument>>> = LazyLock::new(Arc::default);\nstatic EMPTY_ISINS: LazyLock<Arc<HashMap<i128, Str>>> = LazyLock::new(Arc::default);\nstatic EMPTY_ALIASES: LazyLock<Arc<HashMap<Str, Str>>> = LazyLock::new(Arc::default);"),
    ("    fn indexes(&mut self) -> (Indexes<'_>, &mut BTreeMap<SmolStr, Instrument>) {", "    fn indexes(&mut self) -> (Indexes<'_>, &mut BTreeMap<Str, Instrument>) {"),
    ("    fn code_of(&self, key: &str) -> Option<&SmolStr> {", "    fn code_of(&self, key: &str) -> Option<&Str> {"),
    ("        let states = |code: &&SmolStr, on: Option<&Mic>| {", "        let states = |code: &&Str, on: Option<&Mic>| {"),
    ("        let code = || instrument.crosscode_smol();", "        let code = || instrument.crosscode.storage().clone();"),
    ("        let mut best: Option<(f64, &SmolStr)> = None;\n        let mut tied = false;\n        let mut below: Option<(f64, &SmolStr)> = None;",
     "        let mut best: Option<(f64, &Str)> = None;\n        let mut tied = false;\n        let mut below: Option<(f64, &Str)> = None;"),
    ("                    .map(|(code, _)| code.clone())\n                    .collect(),\n            }),\n            (None, Some((best, code))) => Err(Unmatched::BelowThreshold {\n                best,\n                code: code.clone(),\n            }),",
     "                    .map(|(code, _)| code.storage().clone())\n                    .collect(),\n            }),\n            (None, Some((best, code))) => Err(Unmatched::BelowThreshold {\n                best,\n                code: code.storage().clone(),\n            }),"),
    ("                Found::Several(codes) => {\n                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {\n                        tier: MatchTier::Code(kind.clone()),\n                        codes,\n                    }));",
     "                Found::Several(codes) => {\n                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {\n                        tier: MatchTier::Code(kind.clone()),\n                        codes: spelled_codes(codes),\n                    }));"),
    ("                Found::Several(codes) => {\n                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {\n                        tier: MatchTier::Symbology,\n                        codes,\n                    }));",
     "                Found::Several(codes) => {\n                    return Exact::Ended(Resolution::Unmatched(Unmatched::Ambiguous {\n                        tier: MatchTier::Symbology,\n                        codes: spelled_codes(codes),\n                    }));"),
    ("impl Instrument {\n    /// The key as the table spells it.\n    fn crosscode_smol(&self) -> SmolStr {\n        SmolStr::new(self.crosscode.as_str())\n    }\n}\n",
     "/// The codes of several instruments as a refusal names them.\nfn spelled_codes(codes: Vec<Str>) -> Vec<SmolStr> {\n    codes.into_iter().map(Str::into_inner).collect()\n}\n"),
    ("    fn put(&mut self, code: &SmolStr, next: Instrument) {", "    fn put(&mut self, code: &Str, next: Instrument) {"),
    ("            self.check_mint(&mut instrument);\n            instrument.finalize();\n            let code = instrument.crosscode_smol();\n            self.put(&code, instrument);",
     "            self.check_mint(&mut instrument);\n            instrument.finalize();\n            let code = instrument.crosscode.clone();\n            self.put(&code, instrument);"),
    ("    fn rekey(&mut self, old: &SmolStr, mut next: Instrument) -> Result<()> {\n        next.push_alias(old.as_str())?;\n        next.respell()?;\n        let new = next.crosscode_smol();",
     "    fn rekey(&mut self, old: &Str, mut next: Instrument) -> Result<()> {\n        next.push_alias(old.as_str())?;\n        next.respell()?;\n        let new = next.crosscode.clone();"),
    ("        let dependents: Vec<SmolStr> = self", "        let dependents: Vec<Str> = self"),
    ("            if dependent.underlying() == Some(old.as_str()) {\n                dependent.underlying = Some(Str::new(new.as_str()));\n            }",
     "            if dependent.underlying() == Some(old.as_str()) {\n                dependent.underlying = Some(new.clone());\n            }"),
    ("struct Snapshot {\n    rows: Arc<BTreeMap<SmolStr, Instrument>>,\n    /// The code of the row last answered.\n    after: Option<SmolStr>,",
     "struct Snapshot {\n    rows: Arc<BTreeMap<Str, Instrument>>,\n    /// The code of the row last answered.\n    after: Option<Str>,"),
]
for old, new in edits:
    count = text.count(old)
    assert count == 1, (count, old[:80])
    text = text.replace(old, new)
path.write_text(text)
print("ok", len(edits))
