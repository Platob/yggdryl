#!/usr/bin/env python3
"""P9 phase 2: the market holder `instcode` across the graph files.

Exact-string edits, each anchor asserted to match once. Idempotent: a file
already carrying the edit is passed over.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]


def edit(path: str, old: str, new: str) -> None:
    file = ROOT / path
    text = file.read_text(encoding="utf-8")
    if new in text and old not in text:
        return
    count = text.count(old)
    assert count == 1, f"{path}: expected one match of {old[:60]!r}, got {count}"
    file.write_text(text.replace(old, new), encoding="utf-8")


# --- graph/market.rs: the trait's holder, the follow, the merge ------------
edit(
    "rust/market/src/graph/market.rs",
    "    fn get_ticker(&self) -> Option<&str>;\n"
    "    /// Sets [`Self::get_ticker`].\n"
    "    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool);\n",
    "    fn get_ticker(&self) -> Option<&str>;\n"
    "    /// Sets [`Self::get_ticker`].\n"
    "    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool);\n"
    "    /// The cross code of the instrument this element is about\n"
    "    /// ([`Instrument::get_crosscode`](crate::Instrument)): a real ISIN for a\n"
    "    /// security an agency numbered, a `class:body` for an FX pair or a\n"
    "    /// derivative - the instruments table's own key, so a reader joins a\n"
    "    /// market table to it on `instcode = crosscode` with no lookup. Written\n"
    "    /// by a parse where the code is a function of the element's own facts\n"
    "    /// and by a lifecycle's fill from the resolved instrument, followed\n"
    "    /// along a chain, fed to no digest: a reading of facts the row already\n"
    "    /// feeds, as `crossuuid` is.\n"
    "    fn get_instcode(&self) -> Option<&str>;\n"
    "    /// Records the cross code of the instrument this element is about;\n"
    "    /// `None` clears it.\n"
    "    fn set_instcode(&mut self, code: Option<Str>, overwrite: bool);\n",
)
edit(
    "rust/market/src/graph/market.rs",
    "use yggdryl::{Ccy, Cfi, Decimal, Isin, Mic, Result, Unit};\n",
    "use yggdryl::{Ccy, Cfi, Decimal, Isin, Mic, Result, Str, Unit};\n",
)
edit(
    "rust/market/src/graph/market.rs",
    "        // And the currency the instrument originates in, another.\n"
    "        changed |= moved(\n"
    "            this.get_origccy().clone(),\n"
    "            better(this.get_origccy().clone(), previous.get_origccy(), false),\n"
    "            |ccy| this.set_origccy(ccy, true),\n"
    "        );\n"
    "    }\n",
    "        // And the currency the instrument originates in, another.\n"
    "        changed |= moved(\n"
    "            this.get_origccy().clone(),\n"
    "            better(this.get_origccy().clone(), previous.get_origccy(), false),\n"
    "            |ccy| this.set_origccy(ccy, true),\n"
    "        );\n"
    "        // And the instrument's code, the instrument fact every row carries.\n"
    "        if this.get_instcode().is_none()\n"
    "            && let Some(code) = previous.get_instcode()\n"
    "        {\n"
    "            this.set_instcode(Some(Str::new(code)), true);\n"
    "            changed = true;\n"
    "        }\n"
    "    }\n",
)
edit(
    "rust/market/src/graph/market.rs",
    "    // Two statements naming different ISINs name two instruments, whose\n"
    "    // identifiers never mix: the leading statement's stand whole.\n"
    "    if names_other_instrument(this, other) {\n",
    "    changed |= moved(\n"
    "        this.get_instcode().map(Str::new),\n"
    "        stated(\n"
    "            this.get_instcode().map(Str::new),\n"
    "            other.get_instcode().map(Str::new),\n"
    "            later,\n"
    "        ),\n"
    "        |code| this.set_instcode(code, true),\n"
    "    );\n"
    "    // Two statements naming different ISINs name two instruments, whose\n"
    "    // identifiers never mix: the leading statement's stand whole.\n"
    "    if names_other_instrument(this, other) {\n",
)

# --- graph/facts.rs: the field, the accessors, the copy ----------------------
edit(
    "rust/market/src/graph/facts.rs",
    "    ticker: Option<SmolStr>,\n    metadata: Option<Box<Metadata>>,\n}\n",
    "    ticker: Option<SmolStr>,\n"
    "    /// The resolved instrument's cross code: inline to twenty-three bytes,\n"
    "    /// one shared `Arc<str>` beyond, so a copy along a chain is a byte\n"
    "    /// copy or a reference count.\n"
    "    instcode: Option<Str>,\n"
    "    metadata: Option<Box<Metadata>>,\n}\n",
)
edit(
    "rust/market/src/graph/facts.rs",
    "            ticker: None,\n            metadata: None,\n        }\n    }\n}\n",
    "            ticker: None,\n            instcode: None,\n            metadata: None,\n        }\n    }\n}\n",
)
edit(
    "rust/market/src/graph/facts.rs",
    "    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool) {\n"
    "        let ticker = ticker.filter(|held| !held.is_empty());\n"
    "        if lands(&self.ticker, &ticker, self.ticker.is_none(), overwrite) {\n"
    "            self.ticker = ticker;\n"
    "        }\n"
    "    }\n",
    "    fn set_ticker(&mut self, ticker: Option<SmolStr>, overwrite: bool) {\n"
    "        let ticker = ticker.filter(|held| !held.is_empty());\n"
    "        if lands(&self.ticker, &ticker, self.ticker.is_none(), overwrite) {\n"
    "            self.ticker = ticker;\n"
    "        }\n"
    "    }\n"
    "\n"
    "    fn get_instcode(&self) -> Option<&str> {\n"
    "        self.instcode.as_ref().map(Str::as_str)\n"
    "    }\n"
    "\n"
    "    fn set_instcode(&mut self, code: Option<Str>, overwrite: bool) {\n"
    "        let code = code.filter(|held| !held.is_empty());\n"
    "        if lands(&self.instcode, &code, self.instcode.is_none(), overwrite) {\n"
    "            self.instcode = code;\n"
    "        }\n"
    "    }\n",
)
edit(
    "rust/market/src/graph/facts.rs",
    "    this.set_ticker(other.get_ticker().map(SmolStr::new), true);\n"
    "    this.set_metadata(Some(other.get_metadata().clone()), true);\n",
    "    this.set_ticker(other.get_ticker().map(SmolStr::new), true);\n"
    "    this.set_instcode(other.get_instcode().map(Str::new), true);\n"
    "    this.set_metadata(Some(other.get_metadata().clone()), true);\n",
)
edit(
    "rust/market/src/graph/facts.rs",
    "use yggdryl::{Ccy, Cfi, Decimal, Mic, Result, State, Unit, Uuid};\n",
    "use yggdryl::{Ccy, Cfi, Decimal, Mic, Result, State, Str, Unit, Uuid};\n",
)

# --- graph/mod.rs: the delegate macro -----------------------------------------
edit(
    "rust/market/src/graph/mod.rs",
    "            fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {\n"
    "                $crate::graph::Market::set_ticker(&mut self.$($field).+, ticker, overwrite);\n"
    "            }\n",
    "            fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {\n"
    "                $crate::graph::Market::set_ticker(&mut self.$($field).+, ticker, overwrite);\n"
    "            }\n"
    "            fn get_instcode(&self) -> Option<&str> {\n"
    "                $crate::graph::Market::get_instcode(&self.$($field).+)\n"
    "            }\n"
    "            fn set_instcode(&mut self, code: Option<::yggdryl::Str>, overwrite: bool) {\n"
    "                $crate::graph::Market::set_instcode(&mut self.$($field).+, code, overwrite);\n"
    "            }\n",
)

# --- graph/operation.rs: the two explicit implementations ---------------------
op = ROOT / "rust/market/src/graph/operation.rs"
text = op.read_text(encoding="utf-8")
old = (
    "    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {\n"
    "        self.data.set_ticker(ticker, overwrite);\n"
    "    }\n"
)
new = old + (
    "    fn get_instcode(&self) -> Option<&str> {\n"
    "        self.data.get_instcode()\n"
    "    }\n"
    "    fn set_instcode(&mut self, code: Option<yggdryl::Str>, overwrite: bool) {\n"
    "        self.data.set_instcode(code, overwrite);\n"
    "    }\n"
)
if "fn get_instcode" not in text:
    assert text.count(old) == 2, text.count(old)
    op.write_text(text.replace(old, new), encoding="utf-8")

# --- graph/market_data.rs: the enum's delegation --------------------------------
edit(
    "rust/market/src/graph/market_data.rs",
    "    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {\n"
    "        delegate_by_variant!(self, set_ticker, ticker, overwrite);\n"
    "    }\n",
    "    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {\n"
    "        delegate_by_variant!(self, set_ticker, ticker, overwrite);\n"
    "    }\n"
    "    fn get_instcode(&self) -> Option<&str> {\n"
    "        delegate_by_variant!(self, get_instcode)\n"
    "    }\n"
    "    fn set_instcode(&mut self, code: Option<yggdryl::Str>, overwrite: bool) {\n"
    "        delegate_by_variant!(self, set_instcode, code, overwrite);\n"
    "    }\n",
)

# --- graph/market_column.rs: the 37th column ------------------------------------
edit(
    "rust/market/src/graph/market_column.rs",
    "//! The thirty-six columns every market element is stated in.\n",
    "//! The thirty-seven columns every market element is stated in.\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "    SecurityIds,\n"
    "    /// The ISIN it names: the `ISIN` security identifier, projected.\n"
    "    IsinCode,\n",
    "    SecurityIds,\n"
    "    /// The cross code of the instrument it is about: the instruments\n"
    "    /// table's key, a real ISIN or a `class:body`.\n"
    "    InstCode,\n"
    "    /// The ISIN it names: the `ISIN` security identifier, projected.\n"
    "    IsinCode,\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "    pub const ALL: [Self; 36] = [\n",
    "    pub const ALL: [Self; 37] = [\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "        Self::SecurityIds,\n        Self::IsinCode,\n        Self::CfiCode,\n",
    "        Self::SecurityIds,\n        Self::InstCode,\n        Self::IsinCode,\n        Self::CfiCode,\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    '            Self::SecurityIds => "securityids",\n            Self::IsinCode => "isincode",\n',
    '            Self::SecurityIds => "securityids",\n            Self::InstCode => "instcode",\n            Self::IsinCode => "isincode",\n',
)
edit(
    "rust/market/src/graph/market_column.rs",
    '            Self::SecurityIds => "Security IDs",\n            Self::IsinCode => "ISIN Code",\n',
    '            Self::SecurityIds => "Security IDs",\n            Self::InstCode => "Instrument Code",\n            Self::IsinCode => "ISIN Code",\n',
)
edit(
    "rust/market/src/graph/market_column.rs",
    '            Self::IsinCode => "The ISIN the element names: its isin security identifier.",\n',
    "            Self::InstCode => {\n"
    '                "The cross code of the instrument the element is about - a real ISIN for a security an agency numbered, a CFI class and its characteristics for an FX pair or a derivative - the instruments table\'s key; null where no instrument is resolved."\n'
    "            }\n"
    '            Self::IsinCode => "The ISIN the element names: its isin security identifier.",\n',
)
edit(
    "rust/market/src/graph/market_column.rs",
    "    /// `map<ccy, decimal>` for the rates - keys and values required - and\n"
    "    /// `utf8` for the ticker.\n",
    "    /// `map<ccy, decimal>` for the rates - keys and values required - and\n"
    "    /// `utf8` for the ticker and the instrument code.\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "            Self::Ticker => DataType::utf8(),\n",
    "            Self::Ticker | Self::InstCode => DataType::utf8(),\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "            Self::Ticker => element.get_ticker().map(Scalar::from),\n",
    "            Self::Ticker => element.get_ticker().map(Scalar::from),\n"
    "            Self::InstCode => element.get_instcode().map(Scalar::from),\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "            Self::Ticker => element.set_ticker(value.as_str().map(SmolStr::new), true),\n",
    "            Self::Ticker => element.set_ticker(value.as_str().map(SmolStr::new), true),\n"
    "            Self::InstCode => element.set_instcode(value.as_str().map(Str::new), true),\n",
)
edit(
    "rust/market/src/graph/market_column.rs",
    "use yggdryl::{\n    Ccy, Cfi, DataType, Decimal, Field, Isin, Mic, Result, Scalar, TimeUnit, Timezone, Unit,\n};\n",
    "use yggdryl::{\n    Ccy, Cfi, DataType, Decimal, Field, Isin, Mic, Result, Scalar, Str, TimeUnit, Timezone, Unit,\n};\n",
)

# --- graph/arrow.rs: the storage, the three arms, the doc pin -------------------
edit(
    "rust/market/src/graph/arrow.rs",
    "                MarketColumn::Ticker => Storage::Text,\n",
    "                MarketColumn::Ticker | MarketColumn::InstCode => Storage::Text,\n",
)
edit(
    "rust/market/src/graph/arrow.rs",
    "            Column::Market(MarketColumn::Ticker) => market.get_ticker(),\n",
    "            Column::Market(MarketColumn::Ticker) => market.get_ticker(),\n"
    "            Column::Market(MarketColumn::InstCode) => market.get_instcode(),\n",
)
edit(
    "rust/market/src/graph/arrow.rs",
    "            } else if column == MarketColumn::Ticker {\n"
    "                target.set_ticker(leaf.text(row).map(SmolStr::new), true);\n",
    "            } else if column == MarketColumn::Ticker {\n"
    "                target.set_ticker(leaf.text(row).map(SmolStr::new), true);\n"
    "            } else if column == MarketColumn::InstCode {\n"
    "                target.set_instcode(leaf.text(row).map(yggdryl::Str::new), true);\n",
)
edit(
    "rust/market/src/graph/arrow.rs",
    "            } else if column == MarketColumn::Ticker {\n"
    "                match leaf.text(row) {\n"
    "                    Some(stated) if Some(stated) != canonical.get_ticker() => {\n"
    "                        return Err(differs(\n"
    "                            path,\n"
    "                            column.name(),\n"
    "                            &canonical.get_ticker(),\n"
    "                            &stated,\n"
    "                        ));\n"
    "                    }\n",
    "            } else if column == MarketColumn::InstCode {\n"
    "                match leaf.text(row) {\n"
    "                    Some(stated) if Some(stated) != canonical.get_instcode() => {\n"
    "                        return Err(differs(\n"
    "                            path,\n"
    "                            column.name(),\n"
    "                            &canonical.get_instcode(),\n"
    "                            &stated,\n"
    "                        ));\n"
    "                    }\n"
    "                    _ => {}\n"
    "                }\n"
    "            } else if column == MarketColumn::Ticker {\n"
    "                match leaf.text(row) {\n"
    "                    Some(stated) if Some(stated) != canonical.get_ticker() => {\n"
    "                        return Err(differs(\n"
    "                            path,\n"
    "                            column.name(),\n"
    "                            &canonical.get_ticker(),\n"
    "                            &stated,\n"
    "                        ));\n"
    "                    }\n",
)
edit(
    "rust/market/src/graph/arrow.rs",
    "    /// assert_eq!(field.field_len(), 6 + 9 + 36 + 5 + 3 + 6);\n"
    "    /// let nested: Vec<&str> = field.fields()[59..].iter().map(|field| field.name()).collect();\n",
    "    /// // `instcode` joined the market band, D42: sixty-six columns.\n"
    "    /// assert_eq!(field.field_len(), 6 + 9 + 37 + 5 + 3 + 6);\n"
    "    /// let nested: Vec<&str> = field.fields()[60..].iter().map(|field| field.name()).collect();\n",
)
print("instcode edits applied")
