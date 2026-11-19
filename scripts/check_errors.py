#!/usr/bin/env python3
"""Check each contract's public error discriminants against its own docs table."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = ("vault-registry", "access-lease", "subscription")
ENUM = re.compile(r"#\[contracterror\][\s\S]*?\benum\s+Error\s*\{([^}]*)\}")
VARIANT = re.compile(r"^\s*(\w+)\s*=\s*(\d+)\s*,", re.MULTILINE)
ROW = re.compile(r"^\|\s*(\d+)\s*\|\s*`(\w+)`\s*\|", re.MULTILINE)


def check_error_tables() -> list[str]:
    docs = (ROOT / "docs" / "CONTRACTS.md").read_text(encoding="utf-8")
    errors = []
    for slug in CONTRACTS:
        source = (ROOT / "contracts" / slug / "src" / "lib.rs").read_text(encoding="utf-8")
        enum = ENUM.search(source)
        section = re.search(rf"^## {re.escape(slug)}\s*$([\s\S]*?)(?=^## |\Z)", docs, re.MULTILINE)
        table = re.search(r"^### Errors\s*$([\s\S]*?)(?=^### |\Z)", section.group(1), re.MULTILINE) if section else None
        if not enum or not table:
            errors.append(f"{slug}: missing Error enum or documentation table")
            continue
        code_pairs = [(name, int(code)) for name, code in VARIANT.findall(enum.group(1))]
        doc_pairs = [(name, int(code)) for code, name in ROW.findall(table.group(1))]
        actual, expected = dict(code_pairs), dict(doc_pairs)
        if not actual or not expected or len(actual) != len(code_pairs) or len(expected) != len(doc_pairs):
            errors.append(f"{slug}: empty or duplicate error-code entries")
        if len(set(actual.values())) != len(actual) or len(set(expected.values())) != len(expected):
            errors.append(f"{slug}: duplicate numeric error codes")
        for name in sorted(actual.keys() | expected.keys()):
            if actual.get(name) != expected.get(name):
                errors.append(f"{slug}: {name}: Rust={actual.get(name)}, docs={expected.get(name)}")
    return errors


if __name__ == "__main__":
    issues = check_error_tables()
    for issue in issues:
        print(issue, file=sys.stderr)
    if not issues:
        print("All contract error-code tables match their Rust enums.")
    sys.exit(bool(issues))
