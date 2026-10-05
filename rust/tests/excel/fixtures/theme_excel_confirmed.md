# Excel tint color oracle

`theme_excel_confirmed.json` records 110 font colors opened without repair by Microsoft Excel 16.0 build 20430.0. The workbook was authored through the Rust `Workbook::set_style` API from an openpyxl Office-theme base. The fixture captures the **wire** theme scheme and indexed palette RGB, the caller's input tint, the saved OOXML font `tint` spelling, Excel's `Font.TintAndShade` readback, and Excel's raw `Font.Color` BGR integer. Its 87 nonzero tints include ordinary palette shades, arbitrary fractions, and half-step boundaries; 23 zero or untinted colors pin their bases. The Rust regression converts the BGR result to RGB and uses the saved input tint. It does not derive an expected color from Rust itself.

Excel's `TintAndShade` readback quantizes nonzero inputs to `trunc(input * 32767) / 32767` on this build. That readback is recorded as evidence, **not** substituted for the OOXML input when computing RGB. The observed 87 tinted colors match integer HLS240 conversion with positive luminance terms truncated separately. The SpreadsheetML documentation describes an HLSMAX of 255; this fixture pins this Excel build's rendering result and does not redefine the file format.

To regenerate the local desktop input, use the committed 5 KB
`theme_oracle_base.xlsx` fixture beside this note. It carries the Office theme
and indexed palette recorded in the JSON; no scratch path or build-artifact hash
is involved. From PowerShell at the repository root, run:

```powershell
$env:YGGDRYL_EXCEL_STYLE_EXPORT_DIR = Join-Path $PWD 'rust/target/excel-desktop/style-oracle-confirmed'
cargo test --locked -p yggdryl --test excel export_desktop_tint_oracle -- --ignored --nocapture
python/.venv/Scripts/python.exe scripts/check_excel_desktop.py styles --active --timeout 300 --cases rust/target/excel-desktop/style-oracle-confirmed/style-cases.json --output-dir rust/target/excel-desktop/styles-confirmed-active
```

The explicitly ignored Rust exporter reads the committed 110 inputs, applies
each through `Workbook::set_style`, computes expectations through the current
`StyleSheet::resolve`, and verifies them after a Rust save/reopen. It writes
`styles-from-rust.xlsx` and `style-cases.json`; it never opens Excel. Run the
Python step only when the user's Excel is available. Its `results.json` must
contain 110 passing cases, no repair or failures, and `cleanup_completed: true`.
Choose a fresh desktop output directory for each run.
