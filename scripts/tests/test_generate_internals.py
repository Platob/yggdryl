"""The generated re-exports keep rustfmt's module-path order."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import generate_internals as generator


class InternalsOrderingTests(unittest.TestCase):
    def test_parent_and_child_exports_sort_by_the_complete_rust_path(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            (source / "excel" / "formula").mkdir(parents=True)
            lib = source / "lib.rs"
            lib.write_text("mod excel;\n", encoding="utf-8")
            (source / "excel" / "mod.rs").write_text("mod formula;\n", encoding="utf-8")
            declaration = '#[cfg(feature = "internals")]\n#[doc(hidden)]\npub mod internals {}\n'
            (source / "excel" / "formula.rs").write_text(
                "mod eval;\nmod number;\n" + declaration, encoding="utf-8"
            )
            for name in ("eval", "number"):
                (source / "excel" / "formula" / f"{name}.rs").write_text(
                    declaration, encoding="utf-8"
                )
            with patch.object(generator, "SRC", source), patch.object(generator, "LIB", lib):
                exports = [line.strip() for line in generator.block().splitlines() if "pub use" in line]
            self.assertEqual(exports, [
                "pub use crate::excel::formula::eval::internals as excel_formula_eval;",
                "pub use crate::excel::formula::internals as excel_formula;",
                "pub use crate::excel::formula::number::internals as excel_formula_number;",
            ])


if __name__ == "__main__":
    unittest.main()
