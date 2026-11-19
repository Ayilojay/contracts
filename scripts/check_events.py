#!/usr/bin/env python3
"""check_events.py — CI guard for issue #121.

Diffs the ``#[contractevent(topics = ["<topic>", ...])]`` definitions found in
each contract's Rust source against the event tables in ``docs/CONTRACTS.md``.

Exit 0  → every topic in the Rust source has a matching row in the docs, and
          every row in the docs has a matching definition in the source.
Exit 1  → at least one mismatch was found; the offending topics are printed.

Usage
-----
    python3 scripts/check_events.py

Run from the repository root (the script resolves paths relative to the
directory that contains it).

How it works
------------
Source extraction
    The script scans each contract's ``src/lib.rs`` for lines that match::

        #[contractevent(topics = ["<topic>", ...])]

    It captures the *first* string literal in the ``topics`` array; that is the
    fixed symbol used as the event discriminator and the value that appears in
    the docs table.

Docs extraction
    The script finds each ``### Events`` heading in ``docs/CONTRACTS.md`` and
    reads the Markdown table rows that follow it.  Each row is expected to
    contain a backtick-quoted topic of the form ``("topic", ...)`` or
    ``("topic",)``.  The first quoted word inside the parentheses is extracted.

Contract mapping
    The script maps each contract to its docs section by the heading text
    immediately above the ``### Events`` heading (e.g. ``## vault-registry``).
    The mapping is::

        vault-registry  → contracts/vault-registry/src/lib.rs
        access-lease    → contracts/access-lease/src/lib.rs
        subscription    → contracts/subscription/src/lib.rs

Error-code cross-reference (issue #236)
    The shared error-code table in ``docs/CONTRACTS.md`` documents numeric
    values for each contract's error enum.  This script also parses the Rust
    ``#[contracterror]`` enums and verifies that every documented value matches
    the enum definition, so stale examples (e.g. subscription ``NotInitialised``
    or ``InvalidPrice``) are caught in CI.
"""

import re
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent

# Maps the heading slug used in CONTRACTS.md to the Rust source path.
CONTRACT_MAP: dict[str, Path] = {
    "vault-registry": REPO_ROOT / "contracts" / "vault-registry" / "src" / "lib.rs",
    "access-lease":   REPO_ROOT / "contracts" / "access-lease"   / "src" / "lib.rs",
    "subscription":   REPO_ROOT / "contracts" / "subscription"   / "src" / "lib.rs",
}

CONTRACTS_MD = REPO_ROOT / "docs" / "CONTRACTS.md"

# Matches:  #[contractevent(topics = ["init"], ...)]
#           #[contractevent(topics = ["grant", ...], ...)]
# Captures the first topic string literal.
_RE_RUST_EVENT = re.compile(
    r'#\[contractevent\s*\(\s*topics\s*=\s*\[\s*"([^"]+)"'
)

# Matches a table row whose first backtick-quoted cell contains a topic tuple,
# e.g.  | `("init",)` | ...  or  | `("grant", resource_id, buyer)` | ...
# Captures the first quoted word inside the parens.
_RE_DOCS_TOPIC = re.compile(
    r'`\(\s*"([^"]+)"'
)

# Matches a Rust enum variant with an explicit discriminant, e.g.
#     NotInitialised = 8,
# Captures the variant name and the numeric value.
_RE_RUST_ERROR_VARIANT = re.compile(
    r'^\s*([A-Z][A-Za-z0-9_]*)\s*=\s*(\d+)\s*,?\s*$'
)

# Matches a docs error-code row, e.g.
#     | `NotInitialised` | 8 | ... |
# Captures the variant name and the documented numeric value.
_RE_DOCS_ERROR_ROW = re.compile(
    r'`([A-Z][A-Za-z0-9_]*)`\s*\|\s*(\d+)\b'
)

# ---------------------------------------------------------------------------
# Extraction helpers
# ---------------------------------------------------------------------------


def extract_rust_topics(path: Path) -> list[str]:
    """Return every first-topic string from ``#[contractevent(...)]`` in *path*."""
    topics: list[str] = []
    text = path.read_text(encoding="utf-8")
    for m in _RE_RUST_EVENT.finditer(text):
        topics.append(m.group(1))
    return topics


def extract_docs_sections(path: Path) -> dict[str, list[str]]:
    """Parse *path* and return a mapping of contract-slug → [topic, ...].

    The parser walks the Markdown line by line and tracks which top-level
    ``## <slug>`` section it is inside.  When it hits a ``### Events`` heading
    it starts collecting topic strings from the table rows that follow; it
    stops when it hits the next heading of any level.
    """
    sections: dict[str, list[str]] = {}
    current_contract: str | None = None
    in_events_table = False

    for line in path.read_text(encoding="utf-8").splitlines():
        # Top-level contract section heading: ## vault-registry
        m_h2 = re.match(r"^##\s+([a-z][a-z0-9-]+)\s*$", line)
        if m_h2:
            current_contract = m_h2.group(1)
            in_events_table = False
            continue

        # ### Events heading inside a contract section
        if re.match(r"^###\s+Events\s*$", line) and current_contract:
            in_events_table = True
            sections.setdefault(current_contract, [])
            continue

        # Any other heading ends the events table
        if re.match(r"^#{1,6}\s", line):
            in_events_table = False
            continue

        # Inside the events table: extract topic from backtick-quoted cell
        if in_events_table and current_contract:
            m_topic = _RE_DOCS_TOPIC.search(line)
            if m_topic:
                sections[current_contract].append(m_topic.group(1))

    return sections


def extract_rust_error_codes(path: Path) -> dict[str, int]:
    """Return ``{variant: value}`` for the ``#[contracterror]`` enum in *path*.

    Only variants with an explicit numeric discriminant are captured; the
    ``#[contracterror]`` attribute is used to locate the enum so unrelated
    enums in the same file are ignored.
    """
    text = path.read_text(encoding="utf-8")
    codes: dict[str, int] = {}

    # Find the enum that follows a #[contracterror] attribute.
    m_attr = re.search(r'#\[contracterror\]', text)
    if not m_attr:
        return codes

    m_enum = re.search(r'\benum\s+[A-Za-z0-9_]+\s*\{', text[m_attr.end():])
    if not m_enum:
        return codes

    body_start = m_attr.end() + m_enum.end()
    depth = 1
    i = body_start
    while i < len(text) and depth > 0:
        if text[i] == '{':
            depth += 1
        elif text[i] == '}':
            depth -= 1
        i += 1
    body = text[body_start:i - 1]

    for line in body.splitlines():
        m = _RE_RUST_ERROR_VARIANT.match(line)
        if m:
            codes[m.group(1)] = int(m.group(2))

    return codes


def extract_docs_error_codes(path: Path) -> dict[str, int]:
    """Return ``{variant: value}`` from the shared error-code table in *path*.

    The shared table is the one under the ``### Error codes`` heading (or any
    heading containing ``Error``) that lists ``| `Variant` | N |`` rows.
    """
    codes: dict[str, int] = {}
    in_error_table = False

    for line in path.read_text(encoding="utf-8").splitlines():
        if re.match(r"^#{1,6}\s", line):
            in_error_table = bool(re.search(r"error", line, re.IGNORECASE))
            continue

        if in_error_table:
            m = _RE_DOCS_ERROR_ROW.search(line)
            if m:
                codes[m.group(1)] = int(m.group(2))

    return codes


# ---------------------------------------------------------------------------
# Main comparison logic
# ---------------------------------------------------------------------------


def check() -> int:
    """Compare Rust source topics against docs tables.

    Returns 0 if everything matches, 1 if there are any mismatches.
    """
    docs_sections = extract_docs_sections(CONTRACTS_MD)
    errors: list[str] = []

    for slug, src_path in CONTRACT_MAP.items():
        if not src_path.exists():
            errors.append(f"[{slug}] source file not found: {src_path}")
            continue

        rust_topics = set(extract_rust_topics(src_path))
        docs_topics = set(docs_sections.get(slug, []))

        missing_from_docs = rust_topics - docs_topics
        missing_from_rust = docs_topics - rust_topics

        if missing_from_docs:
            for t in sorted(missing_from_docs):
                errors.append(
                    f'[{slug}] topic "{t}" is defined in Rust '
                    f"but missing from docs/CONTRACTS.md"
                )

        if missing_from_rust:
            for t in sorted(missing_from_rust):
                errors.append(
                    f'[{slug}] topic "{t}" appears in docs/CONTRACTS.md '
                    f"but has no #[contractevent] definition in the source"
                )

        if not missing_from_docs and not missing_from_rust:
            print(f"[{slug}] OK — {len(rust_topics)} event(s) match")

    # Cross-reference documented error codes against the Rust enums.
    docs_error_codes = extract_docs_error_codes(CONTRACTS_MD)
    for slug, src_path in CONTRACT_MAP.items():
        if not src_path.exists():
            continue

        rust_codes = extract_rust_error_codes(src_path)
        for variant, value in sorted(rust_codes.items()):
            if variant not in docs_error_codes:
                continue
            documented = docs_error_codes[variant]
            if documented != value:
                errors.append(
                    f"[{slug}] error code `{variant}` is documented as "
                    f"{documented} in docs/CONTRACTS.md but the enum defines {value}"
                )

    if errors:
        print("\nEvent table drift detected:\n", file=sys.stderr)
        for e in errors:
            print(f"  ✗ {e}", file=sys.stderr)
        print(
            "\nUpdate docs/CONTRACTS.md to match the #[contractevent] definitions "
            "(or vice-versa) and re-run this check.",
            file=sys.stderr,
        )
        return 1

    print("\nAll event tables are in sync with the source.")
    return 0


if __name__ == "__main__":
    sys.exit(check())
