#!/usr/bin/env python3
"""Validate the declared feature totals against the inventory table.

Runs with the Python standard library only and does not build TraceDraw.
"""

from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
FEATURES = ROOT / "docs" / "features.md"
COUNTS = re.compile(
    r"^Counts: works (\d+), partial (\d+) .*?, missing (\d+), not applicable (\d+)(?:[.,]|$)",
    re.MULTILINE,
)
STATUS = (
    ("works", re.compile(r"^works(?:\b|\s|:|;)")),
    ("partial", re.compile(r"^(?:partial|partly)(?:\b|\s|:|;)")),
    ("missing", re.compile(r"^missing(?:\b|\s|:|;)")),
    ("not applicable", re.compile(r"^not applicable(?:\b|\s|:|;)")),
)


def main() -> int:
    contents = FEATURES.read_text(encoding="utf-8")
    match = COUNTS.search(contents)
    if match is None:
        print("ERROR: feature count header is missing or malformed", file=sys.stderr)
        return 1

    expected = dict(zip(("works", "partial", "missing", "not applicable"), map(int, match.groups())))
    actual: Counter[str] = Counter()
    issues: list[str] = []
    section = ""
    for number, line in enumerate(contents.splitlines(), 1):
        if line.startswith("## "):
            section = line[3:]
        if not line.startswith("| "):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 2 or cells[0] in ("Capability", "Element") or cells[0].startswith("---"):
            continue
        if cells[0] == "Icons" and section == "Workspace look":
            # Descriptive design assessment, not an implemented feature.
            continue
        status = cells[1].lower()
        category = next((name for name, pattern in STATUS if pattern.match(status)), None)
        if category is None:
            issues.append(f"line {number}: unknown feature status {cells[1]!r}")
        else:
            actual[category] += 1

    for category, total in expected.items():
        if actual[category] != total:
            issues.append(
                f"{category}: header says {total}, inventory contains {actual[category]}"
            )
    if issues:
        for issue in issues:
            print(f"ERROR: {issue}", file=sys.stderr)
        return 1

    print("Feature inventory is consistent: " + ", ".join(
        f"{category}={actual[category]}" for category in expected
    ))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
