"""The offline FIX importer preserves Orchestra identity and references."""

from __future__ import annotations

import copy
import importlib.util
import pathlib
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "generate_fix_dictionary.py"
SPEC = importlib.util.spec_from_file_location("generate_fix_dictionary", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)


def wire_field(name: str, tag: int, dtype: str = "utf8") -> dict:
    return {
        "name": name,
        "dtype": {"type": dtype},
        "nullable": True,
        "metadata": {"fix:tag": str(tag)},
    }


def member(kind: str, identifier: int, required: bool = False) -> dict:
    return {"kind": kind, "id": identifier, "required": required}


class FixCatalogGeneration(unittest.TestCase):
    def setUp(self) -> None:
        self.fields = [
            wire_field("nopartyids", 453, "int32"),
            wire_field("partyid", 448),
            wire_field("nopartysubids", 802, "int32"),
            wire_field("partysubid", 523),
        ]
        self.latest = {
            "groups": {
                "Parties": {"id": 1012, "tag": 453, "members": [member("field", 448), member("group", 2077)]},
                "PtysSubGrp": {"id": 2077, "tag": 802, "members": [member("field", 523)]},
            },
            "components": {},
            "messages": {"D": {"id": 1, "name": "NewOrderSingle", "members": [member("group", 1012, True)]}},
            "code_sets": {},
        }

    def test_counter_and_group_are_distinct_with_nested_references(self) -> None:
        catalog = GENERATOR.build_catalog(self.latest, self.fields)
        groups = {field["name"]: field for field in catalog["groups"]}
        components = {field["name"]: field for field in catalog["components"]}
        self.assertEqual({"type": "int32"}, catalog["fields"][1]["dtype"])
        self.assertEqual("453", groups["parties"]["metadata"]["fix:counter"])
        self.assertNotIn("fix:tag", groups["parties"]["metadata"])
        item = groups["parties"]["dtype"]["field"]
        self.assertEqual("party", item["name"])
        self.assertFalse(item["nullable"])
        self.assertEqual({"fix:component": "party"}, item["metadata"])
        children = components["party"]["dtype"]["fields"]
        self.assertEqual(["partyid", "nopartysubids", "ptyssubgrp"], [field["name"] for field in children])
        self.assertEqual({"fix:field": "nopartysubids"}, children[1]["metadata"])
        self.assertEqual({"fix:group": "ptyssubgrp"}, children[2]["metadata"])
        message = catalog["messages"][0]
        self.assertEqual("D", message["metadata"]["fix:msgtype"])
        self.assertEqual([False, False], [field["nullable"] for field in message["dtype"]["fields"]])

    def test_one_counter_keeps_every_group_context(self) -> None:
        self.latest["groups"]["RequestedParties"] = {"id": 9001, "tag": 453, "members": [member("field", 448, True)]}
        catalog = GENERATOR.build_catalog(self.latest, self.fields)
        matching = [field["name"] for field in catalog["groups"] if field["metadata"]["fix:counter"] == "453"]
        self.assertEqual(["parties", "requestedparties"], matching)

    def test_global_names_do_not_collide_with_wire_fields(self) -> None:
        self.fields.extend([wire_field("party", 9001), wire_field("ratesource", 1446), wire_field("securityxml", 1185), wire_field("securitystatus", 965)])
        self.latest["groups"]["RateSource"] = {"id": 1062, "tag": 453, "members": [member("field", 1446)]}
        self.latest["components"]["SecurityXML"] = {"id": 1060, "members": [member("field", 1185)]}
        self.latest["messages"]["f"] = {"id": 2, "name": "SecurityStatus", "members": [member("field", 965)]}
        catalog = GENERATOR.build_catalog(self.latest, self.fields)
        names = [field["name"] for entries in catalog.values() for field in entries]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue({"partycomponent", "ratesourcegrp", "ratesourcecomponent", "securityxmlcomponent", "securitystatusmessage"}.issubset(names))

    def test_numbered_party_collections_singularize_before_the_number(self) -> None:
        self.assertEqual("Party", GENERATOR.entry_name("Parties"))
        self.assertEqual("NestedParty2", GENERATOR.entry_name("NestedParties2"))
        self.assertEqual("SecAltID", GENERATOR.entry_name("SecAltIDGrp"))

    def test_catalog_names_do_not_shadow_scalar_aliases(self) -> None:
        self.fields[0]["metadata"]["fix:aliases"] = "parties,party"
        catalog = GENERATOR.build_catalog(self.latest, self.fields)
        self.assertIn("partiesgrp", [field["name"] for field in catalog["groups"]])
        self.assertIn("partycomponent", [field["name"] for field in catalog["components"]])

    def test_fix_service_packs_resolve_to_bounded_numeric_versions(self) -> None:
        self.assertEqual("5.0.2", GENERATOR.version_of("FIX.5.0SP2"))
        self.assertEqual("4.4", GENERATOR.version_of("FIX.4.4"))
        for spelling, expected in [
            ("5.0sp250", "5.0.250"),
            ("FIX.5.0Sp250", "5.0.250"),
            ("FIX.5.0sP250", "5.0.250"),
            ("FIXT.1.1sp250", "1.1.250"),
            ("5.0sp0", "5.0.0"),
            ("5.0SP256", "5.0.256"),
            ("255.255sP65535", "255.255.65535"),
        ]:
            with self.subTest(spelling=spelling):
                self.assertEqual(expected, GENERATOR.version_of(spelling))
        for spelling in [
            "5.0sp65536", "256.0SP1", "5.256sp1", "5.0sp", "5.0sp-1",
            "5sp2", "5.0.SP2", "5.0.1sp2", "5.0sp2.3", "5.0sp2_EP250",
            "fix.5.0sp250", "fixt.1.1SP250", "5.0ſp250",
        ]:
            with self.subTest(spelling=spelling):
                with self.assertRaises(ValueError):
                    GENERATOR.version_of(spelling)
        self.assertEqual((255, 255, 65535), GENERATOR.version_key("255.255.65535"))
        self.assertLess(GENERATOR.version_key("5.0.2"), GENERATOR.version_key("5.0.12"))
        for value in ["256.0.0", "1.256.0", "1.0.65536", "5.0SP2", "5.0sp250", "5.0Sp250", "1.2.3.4", "1.2-beta"]:
            with self.assertRaises(ValueError, msg=value):
                GENERATOR.version_key(value)

    def test_field_enums_are_inline_even_when_the_source_reuses_a_code_set(self) -> None:
        latest = GENERATOR.parse_orchestra(b'''<repository xmlns="http://fixprotocol.io/2020/orchestra/repository" version="FIX.5.0SP2">
          <codeSets><codeSet name="SourceCodeSet" type="String"><code name="CUSIP" value="1"/></codeSet></codeSets>
          <fields><field id="22" name="SecurityIDSource" type="SourceCodeSet"/>
          <field id="456" name="SecurityAltIDSource" type="String" codeSet="SourceCodeSet"/></fields>
        </repository>''')
        parsed = {source.source_id: {"fields": {}} for source in GENERATOR.SOURCES}
        parsed["orchestra-latest"] = latest
        catalog = GENERATOR.build(parsed)
        self.assertEqual({"fields", "messages", "components", "groups"}, set(catalog))
        self.assertEqual(2, len(catalog["fields"]))
        enums = [field["metadata"]["fix:codes"] for field in catalog["fields"]]
        self.assertEqual(enums[0], enums[1])
        self.assertIn('"name":"CUSIP"', enums[0])
        self.assertTrue(all("fix:codeset" not in field["metadata"] for field in catalog["fields"]))

    def test_message_type_is_text_with_its_inline_enum(self) -> None:
        latest = GENERATOR.parse_orchestra(b'''<repository xmlns="http://fixprotocol.io/2020/orchestra/repository" version="FIX.5.0SP2">
          <codeSets><codeSet name="MsgTypeCodeSet" type="String"><code name="NewOrderSingle" value="D"/></codeSet></codeSets>
          <fields><field id="35" name="MsgType" type="MsgTypeCodeSet"/></fields>
        </repository>''')
        parsed = {source.source_id: {"fields": {}} for source in GENERATOR.SOURCES}
        parsed["orchestra-latest"] = latest
        field = GENERATOR.build(parsed)["fields"][0]
        self.assertEqual({"type": "utf8"}, field["dtype"])
        self.assertIn('"name":"NewOrderSingle"', field["metadata"]["fix:codes"])

    def test_invalid_graphs_fail_with_location(self) -> None:
        invalid = copy.deepcopy(self.latest)
        invalid["groups"]["PtysSubGrp"]["members"] = [member("group", 1012)]
        with self.assertRaisesRegex(ValueError, "cyclic.*Parties"):
            GENERATOR.build_catalog(invalid, self.fields)
        invalid = copy.deepcopy(self.latest)
        invalid["groups"]["Parties"]["members"] = [member("field", 99999)]
        with self.assertRaisesRegex(ValueError, "Parties: unresolved field 99999"):
            GENERATOR.build_catalog(invalid, self.fields)
        invalid = copy.deepcopy(self.latest)
        invalid["groups"]["Parties"]["tag"] = 448
        with self.assertRaisesRegex(ValueError, "counter 448 must be an int32 field"):
            GENERATOR.build_catalog(invalid, self.fields)

    def test_native_documents_replace_retired_trees(self) -> None:
        catalog = GENERATOR.build_catalog(self.latest, self.fields)
        documents = GENERATOR.render_tree(catalog)
        self.assertIn("fields/4.json", documents)
        self.assertIn("groups/parties.json", documents)
        self.assertIn("components/party.json", documents)
        self.assertNotIn("layouts.json", documents)
        with tempfile.TemporaryDirectory() as directory:
            out = pathlib.Path(directory)
            (out / "primitive").mkdir()
            (out / "primitive" / "4.json").write_text("[]", encoding="utf-8")
            (out / "layouts.json").write_text("{}", encoding="utf-8")
            hashes = GENERATOR.write_tree(out, documents)
            self.assertFalse((out / "primitive").exists())
            self.assertFalse((out / "layouts.json").exists())
            self.assertEqual(set(documents), set(hashes))


if __name__ == "__main__":
    unittest.main()
