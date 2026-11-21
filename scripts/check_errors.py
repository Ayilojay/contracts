#!/usr/bin/env python3
"""Check each contract's public error discriminants against its own docs table.

Also validates the deployment manifest (deployed/contract-ids.env) so that
missing or malformed contract IDs, versions, network, and timestamp fields
are rejected before the workflow commits the file.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = ("vault-registry", "access-lease", "subscription")
ENUM = re.compile(r"#\[contracterror\][\s\S]*?\benum\s+Error\s*\{([^}]*)\}")
VARIANT = re.compile(r"^\s*(\w+)\s*=\s*(\d+)\s*,", re.MULTILINE)
ROW = re.compile(r"^\|\s*(\d+)\s*\|\s*`(\w+)`\s*\|", re.MULTILINE)

MANIFEST = ROOT / "deployed" / "contract-ids.env"
REQUIRED_IDS = ("VAULT_REGISTRY_ID", "ACCESS_LEASE_ID", "SUBSCRIPTION_ID")
REQUIRED_FIELDS = REQUIRED_IDS + ("NETWORK", "VERSION", "TIMESTAMP")
CONTRACT_ID = re.compile(r"^C[A-Z2-7]{55}$")
NETWORK = re.compile(r"^(testnet|mainnet|futurenet)$")
VERSION = re.compile(r"^\d+\.\d+\.\d+$")
TIMESTAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")


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


def check_deployment_manifest() -> list[str]:
    if not MANIFEST.is_file():
        return [f"manifest: missing {MANIFEST.relative_to(ROOT)}"]
    values: dict[str, str] = {}
    errors = []
    for lineno, raw in enumerate(MANIFEST.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            errors.append(f"manifest: line {lineno}: expected KEY=VALUE, got {line!r}")
            continue
        key, value = (part.strip() for part in line.split("=", 1))
        if key in values:
            errors.append(f"manifest: line {lineno}: duplicate key {key}")
        values[key] = value
    for key in REQUIRED_FIELDS:
        if key not in values:
            errors.append(f"manifest: missing required field {key}")
        elif not values[key]:
            errors.append(f"manifest: empty value for {key}")
    for key in REQUIRED_IDS:
        value = values.get(key, "")
        if value and not CONTRACT_ID.match(value):
            errors.append(f"manifest: {key}: malformed contract ID {value!r}")
    network = values.get("NETWORK", "")
    if network and not NETWORK.match(network):
        errors.append(f"manifest: NETWORK: unsupported value {network!r}")
    version = values.get("VERSION", "")
    if version and not VERSION.match(version):
        errors.append(f"manifest: VERSION: malformed semver {version!r}")
    timestamp = values.get("TIMESTAMP", "")
    if timestamp and not TIMESTAMP.match(timestamp):
        errors.append(f"manifest: TIMESTAMP: malformed ISO-8601 UTC {timestamp!r}")
    return errors


if __name__ == "__main__":
    issues = check_error_tables() + check_deployment_manifest()
    for issue in issues:
        print(issue, file=sys.stderr)
    if not issues:
        print("All contract error-code tables match their Rust enums.")
        print("Deployment manifest fields are present and well-formed.")
    sys.exit(bool(issues))
