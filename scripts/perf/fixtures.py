#!/usr/bin/env python3
"""Writes the test files of the perf seed (standard library only): a PDF with many pages and an .xlsx sheet.

    python3 -I scripts/perf/fixtures.py <output directory> [pdf pages] [sheet rows]
"""
import sys
import zipfile
from pathlib import Path

out = Path(sys.argv[1])
pages = int(sys.argv[2]) if len(sys.argv) > 2 else 150
rows = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
out.mkdir(parents=True, exist_ok=True)


def pdf(count: int) -> bytes:
    """An A4 PDF of `count` pages: a title, 40 lines of text and a few filled boxes on each page."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", None, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    kids = []
    for number in range(1, count + 1):
        lines = [f"BT /F1 20 Tf 56 780 Td (Quarterly report - page {number}) Tj ET"]
        for line in range(40):
            text = f"Line {line + 1}: revenue, costs and notes for item {number * 100 + line} in the regional summary."
            lines.append(f"BT /F1 10 Tf 56 {750 - line * 16} Td ({text}) Tj ET")
        for box in range(6):
            shade = (box + number) % 6 / 6
            lines.append(f"{shade:.2f} 0.4 0.7 rg {60 + box * 80} 70 60 {20 + (box * 17 + number) % 60} re f")
        stream = "\n".join(lines).encode()
        objects.append(b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream))
        content = len(objects)
        objects.append(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents %d 0 R >>" % content)
        kids.append(len(objects))
    objects[1] = b"<< /Type /Pages /Count %d /Kids [%s] >>" % (count, b" ".join(b"%d 0 R" % kid for kid in kids))
    body = bytearray(b"%PDF-1.4\n")
    offsets = []
    for index, item in enumerate(objects, start=1):
        offsets.append(len(body))
        body += b"%d 0 obj\n%s\nendobj\n" % (index, item)
    xref = len(body)
    body += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    body += b"".join(b"%010d 00000 n \n" % offset for offset in offsets)
    body += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    return bytes(body)


def column(index: int) -> str:
    name = ""
    index += 1
    while index:
        index, rest = divmod(index - 1, 26)
        name = chr(65 + rest) + name
    return name


def xlsx(path: Path, count: int) -> None:
    """One sheet: a header row and `count` rows of 12 columns (text and numbers, inline strings)."""
    header = ["Region", "Product", "Owner", "Status", "Q1", "Q2", "Q3", "Q4", "Total", "Margin", "Notes", "Code"]
    rows = []
    for row in range(count + 1):
        cells = []
        for col in range(len(header)):
            ref = f"{column(col)}{row + 1}"
            if row == 0:
                cells.append(f'<c r="{ref}" t="inlineStr"><is><t>{header[col]}</t></is></c>')
            elif 4 <= col <= 9:
                cells.append(f'<c r="{ref}"><v>{(row * 37 + col * 101) % 10000 / 10}</v></c>')
            else:
                cells.append(f'<c r="{ref}" t="inlineStr"><is><t>{header[col]} {row % 97}-{col}</t></is></c>')
        rows.append(f'<row r="{row + 1}">{"".join(cells)}</row>')
    sheet = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
             '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'
             + "".join(rows) + "</sheetData></worksheet>")
    files = {
        "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
        "_rels/.rels": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sales" sheetId="1" r:id="rId1"/></sheets></workbook>',
        "xl/_rels/workbook.xml.rels": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
        "xl/worksheets/sheet1.xml": sheet,
    }
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, text in files.items():
            archive.writestr(name, text)


(out / "report.pdf").write_bytes(pdf(pages))
xlsx(out / "sales.xlsx", rows)
print(f"wrote {out}/report.pdf ({pages} pages) and {out}/sales.xlsx ({rows} rows)")
