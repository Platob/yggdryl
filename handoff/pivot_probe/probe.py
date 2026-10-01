"""Probe: which minimal pivot table parts do openpyxl and LibreOffice accept?

Writes a data sheet with openpyxl, injects a pivot cache definition, empty
records and a pivot table definition by hand, then reads it back with
openpyxl and converts it with LibreOffice to ODS, looking for the DataPilot.
"""

import os
import subprocess
import sys
import zipfile

import openpyxl

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "base.xlsx")
OUT = os.path.join(HERE, "pivot.xlsx")

wb = openpyxl.Workbook()
data = wb.active
data.title = "Data"
rows = [
    ("Region", "Product", "Sales"),
    ("East", "Apples", 10),
    ("West", "Apples", 20),
    ("East", "Pears", 5),
    ("West", "Pears", 7),
    ("East", "Apples", 3),
]
for row in rows:
    data.append(row)
report = wb.create_sheet("Report")
# The rendered values, as a writer that does not rely on refresh lays them.
rendered = [
    ("Sum of Sales", "Product", None, None),
    ("Region", "Apples", "Pears", "Grand Total"),
    ("East", 13, 5, 18),
    ("West", 20, 7, 27),
    ("Grand Total", 33, 12, 45),
]
for r, row in enumerate(rendered, start=3):
    for c, value in enumerate(row, start=1):
        if value is not None:
            report.cell(row=r, column=c, value=value)
wb.save(SRC)

NS = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"

cache = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotCacheDefinition {NS} r:id="rId1" refreshOnLoad="1" recordCount="5" createdVersion="6" refreshedVersion="6" minRefreshableVersion="3">
<cacheSource type="worksheet"><worksheetSource ref="A1:C6" sheet="Data"/></cacheSource>
<cacheFields count="3">
<cacheField name="Region" numFmtId="0"><sharedItems count="2"><s v="East"/><s v="West"/></sharedItems></cacheField>
<cacheField name="Product" numFmtId="0"><sharedItems count="2"><s v="Apples"/><s v="Pears"/></sharedItems></cacheField>
<cacheField name="Sales" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="3" maxValue="20"/></cacheField>
</cacheFields>
</pivotCacheDefinition>"""

records = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotCacheRecords {NS} count="5">
<r><x v="0"/><x v="0"/><n v="10"/></r>
<r><x v="1"/><x v="0"/><n v="20"/></r>
<r><x v="0"/><x v="1"/><n v="5"/></r>
<r><x v="1"/><x v="1"/><n v="7"/></r>
<r><x v="0"/><x v="0"/><n v="3"/></r>
</pivotCacheRecords>"""

table = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<pivotTableDefinition {NS} name="PivotTable1" cacheId="1" applyNumberFormats="0" applyBorderFormats="0" applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0" applyWidthHeightFormats="1" dataCaption="Values" updatedVersion="6" minRefreshableVersion="3" useAutoFormatting="1" itemPrintTitles="1" createdVersion="6" indent="0" outline="1" outlineData="1" multipleFieldFilters="0">
<location ref="A3:D7" firstHeaderRow="1" firstDataRow="2" firstDataCol="1"/>
<pivotFields count="3">
<pivotField axis="axisRow" showAll="0"><items count="3"><item x="0"/><item x="1"/><item t="default"/></items></pivotField>
<pivotField axis="axisCol" showAll="0"><items count="3"><item x="0"/><item x="1"/><item t="default"/></items></pivotField>
<pivotField dataField="1" showAll="0"/>
</pivotFields>
<rowFields count="1"><field x="0"/></rowFields>
<rowItems count="3"><i><x/></i><i><x v="1"/></i><i t="grand"><x/></i></rowItems>
<colFields count="1"><field x="1"/></colFields>
<colItems count="3"><i><x/></i><i><x v="1"/></i><i t="grand"><x/></i></colItems>
<dataFields count="1"><dataField name="Sum of Sales" fld="2" baseField="0" baseItem="0"/></dataFields>
<pivotTableStyleInfo name="PivotStyleLight16" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/>
</pivotTableDefinition>"""


def rels(*entries):
    body = "".join(f'<Relationship Id="{i}" Type="{REL}/{t}" Target="{target}"/>' for i, t, target in entries)
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>'


with zipfile.ZipFile(SRC) as source, zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as target:
    names = source.namelist()
    for name in names:
        data_bytes = source.read(name)
        text = data_bytes.decode("utf-8") if name.endswith((".xml", ".rels")) else None
        if name == "[Content_Types].xml":
            text = text.replace(
                "</Types>",
                '<Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>'
                '<Override PartName="/xl/pivotCache/pivotCacheRecords1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/>'
                '<Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/>'
                "</Types>",
            )
        elif name == "xl/_rels/workbook.xml.rels":
            text = text.replace(
                "</Relationships>",
                f'<Relationship Id="rIdPc1" Type="{REL}/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/></Relationships>',
            )
        elif name == "xl/workbook.xml":
            pivot = '<pivotCaches><pivotCache cacheId="1" r:id="rIdPc1"/></pivotCaches>'
            if "<calcPr" in text:
                end = text.index(">", text.index("<calcPr")) + 1
                text = text[:end] + pivot + text[end:]
            else:
                text = text.replace("</workbook>", pivot + "</workbook>")
            if 'xmlns:r=' not in text:
                text = text.replace("<workbook ", f'<workbook xmlns:r="{REL}" ', 1)
        if text is not None:
            data_bytes = text.encode("utf-8")
        target.writestr(name, data_bytes)
    target.writestr("xl/pivotCache/pivotCacheDefinition1.xml", cache)
    target.writestr("xl/pivotCache/pivotCacheRecords1.xml", records)
    target.writestr("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", rels(("rId1", "pivotCacheRecords", "pivotCacheRecords1.xml")))
    target.writestr("xl/pivotTables/pivotTable1.xml", table)
    target.writestr("xl/pivotTables/_rels/pivotTable1.xml.rels", rels(("rId1", "pivotCacheDefinition", "../pivotCache/pivotCacheDefinition1.xml")))
    sheet_rels = "xl/worksheets/_rels/sheet2.xml.rels"
    assert sheet_rels not in names
    target.writestr(sheet_rels, rels(("rId1", "pivotTable", "../pivotTables/pivotTable1.xml")))

loaded = openpyxl.load_workbook(OUT)
pivots = loaded["Report"]._pivots
print("openpyxl pivots:", len(pivots))
for pivot in pivots:
    print("  name", pivot.name, "location", pivot.location.ref, "rows", [f.x for f in pivot.rowFields], "cols", [f.x for f in pivot.colFields], "data", [(d.name, d.fld) for d in pivot.dataFields])
    print("  cache fields", [f.name for f in pivot.cache.cacheFields], "source", pivot.cache.cacheSource.worksheetSource.ref)
roundtrip = os.path.join(HERE, "roundtrip.xlsx")
loaded.save(roundtrip)
print("openpyxl re-saved ok")

out_dir = os.path.join(HERE, "lo")
os.makedirs(out_dir, exist_ok=True)
result = subprocess.run(
    ["soffice", "--headless", "--convert-to", "ods", "--outdir", out_dir, OUT],
    capture_output=True, text=True, timeout=180,
)
print("soffice:", result.returncode, result.stdout.strip(), result.stderr.strip()[:300])
ods = os.path.join(out_dir, "pivot.ods")
with zipfile.ZipFile(ods) as z:
    content = z.read("content.xml").decode("utf-8")
print("LibreOffice DataPilot present:", "table:data-pilot-table " in content)
start = content.find("<table:data-pilot-table ")
print(content[start:start + 700])
sys.exit(0)
