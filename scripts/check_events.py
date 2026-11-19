#!/usr/bin/env python3
"""Validate contract event definitions against docs/CONTRACTS.md.

Compares, for every event emitted by the contracts, the full payload shape
(indexed fields, data fields, field ordering and data_format) against the
tables documented in docs/CONTRACTS.md. Topic-name-only comparison is not
enough: a wrong payload table (e.g. the retirement event) would otherwise
pass silently.

Duplicate rows and duplicate discriminator definitions are reported with
source locations before any set comparison, so they cannot be silently
discarded.

Usage:
    python3 scripts/check_events.py

Exit code 0 when every event matches, 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs" / "CONTRACTS.md"
CONTRACTS_DIR = ROOT / "contracts"


@dataclass
class EventSpec:
    """Structured description of a single event payload."""

    topics: list[str] = field(default_factory=list)
    data: list[str] = field(default_factory=list)
    data_format: str = "single-value"

    def signature(self) -> tuple[tuple[str, ...], tuple[str, ...], str]:
        return (tuple(self.topics), tuple(self.data), self.data_format)


# ---------------------------------------------------------------------------
# Source parsing: extract event specs from Rust contract sources.
# ---------------------------------------------------------------------------

# Matches `#[contractevent]` blocks and captures the struct body.
CONTRACT_EVENT_RE = re.compile(
    r"#\[contractevent[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub\s+)?struct\s+(?P<name>\w+)\s*\{(?P<body>[^}]*)\}",
    re.DOTALL,
)

# Matches `#[topic]` markers on struct fields.
TOPIC_ATTR_RE = re.compile(r"#\[topic\]")

# Matches a struct field: `pub name: Type,`
FIELD_RE = re.compile(r"(?:pub\s+)?(?P<name>\w+)\s*:\s*(?P<ty>[^,]+),")

# Matches `#[contractevent(data_format = "map")]` style attributes.
DATA_FORMAT_RE = re.compile(r"data_format\s*=\s*\"(?P<fmt>[^\"]+)\"")


def _normalise_type(ty: str) -> str:
    return re.sub(r"\s+", " ", ty.strip())


def _line_of(source: str, offset: int) -> int:
    return source.count("\n", 0, offset) + 1


def parse_rust_events(
    source: str, origin: str = "<source>"
) -> tuple[dict[str, EventSpec], list[str]]:
    """Return ({event_name: EventSpec}, duplicate_errors) from a Rust file.

    Duplicate discriminator definitions (the same event name declared more
    than once) are reported with their source locations instead of being
    silently overwritten.
    """

    events: dict[str, EventSpec] = {}
    seen_at: dict[str, int] = {}
    duplicates: list[str] = []
    for match in CONTRACT_EVENT_RE.finditer(source):
        name = match.group("name")
        body = match.group("body")
        header = source[match.start() : match.start("body")]
        line = _line_of(source, match.start())

        if name in seen_at:
            duplicates.append(
                f"duplicate event definition `{name}` at {origin}:{line} "
                f"(first defined at {origin}:{seen_at[name]})"
            )
            continue
        seen_at[name] = line

        fmt_match = DATA_FORMAT_RE.search(header)
        data_format = fmt_match.group("fmt") if fmt_match else "single-value"

        spec = EventSpec(data_format=data_format)
        # Split the body into field chunks so `#[topic]` stays attached to the
        # field that follows it.
        chunks = re.split(r"(?=#\[topic\])", body)
        for chunk in chunks:
            is_topic = bool(TOPIC_ATTR_RE.search(chunk))
            for f_match in FIELD_RE.finditer(chunk):
                fname = f_match.group("name")
                fty = _normalise_type(f_match.group("ty"))
                entry = f"{fname}: {fty}"
                if is_topic:
                    spec.topics.append(entry)
                else:
                    spec.data.append(entry)
        events[name] = spec
    return events, duplicates


# ---------------------------------------------------------------------------
# Docs parsing: extract event specs from docs/CONTRACTS.md tables.
# ---------------------------------------------------------------------------

# A documented event section looks like:
#
#   ### `retire`
#
#   | Kind    | Field | Type   |
#   |---------|-------|--------|
#   | topic   | vault | Address|
#   | data    | amount| i128   |
#
#   data_format: single-value
EVENT_HEADING_RE = re.compile(r"^#{2,4}\s+`?(?P<name>\w+)`?\s*$", re.MULTILINE)
TABLE_ROW_RE = re.compile(
    r"^\|\s*(?P<kind>topic|data)\s*\|\s*(?P<field>[^|]+?)\s*\|\s*(?P<ty>[^|]+?)\s*\|",
    re.MULTILINE | re.IGNORECASE,
)
DOC_DATA_FORMAT_RE = re.compile(r"data_format\s*:\s*(?P<fmt>[\w-]+)", re.IGNORECASE)


def parse_doc_events(text: str) -> tuple[dict[str, EventSpec], list[str]]:
    """Return ({event_name: EventSpec}, duplicate_errors) from the docs.

    Duplicate event sections and duplicate rows within a section are reported
    with their line numbers before any set comparison.
    """

    events: dict[str, EventSpec] = {}
    seen_at: dict[str, int] = {}
    duplicates: list[str] = []
    headings = list(EVENT_HEADING_RE.finditer(text))
    for idx, heading in enumerate(headings):
        name = heading.group("name")
        start = heading.end()
        end = headings[idx + 1].start() if idx + 1 < len(headings) else len(text)
        section = text[start:end]

        rows = list(TABLE_ROW_RE.finditer(section))
        if not rows:
            continue

        heading_line = _line_of(text, heading.start())
        if name in seen_at:
            duplicates.append(
                f"duplicate event section `{name}` at docs/CONTRACTS.md:{heading_line} "
                f"(first documented at docs/CONTRACTS.md:{seen_at[name]})"
            )
            continue
        seen_at[name] = heading_line

        fmt_match = DOC_DATA_FORMAT_RE.search(section)
        spec = EventSpec(
            data_format=fmt_match.group("fmt") if fmt_match else "single-value"
        )
        row_seen: dict[tuple[str, str], int] = {}
        for row in rows:
            kind = row.group("kind").lower()
            field_name = row.group("field").strip()
            row_line = _line_of(text, start + row.start())
            key = (kind, field_name)
            if key in row_seen:
                duplicates.append(
                    f"duplicate {kind} row `{field_name}` for event `{name}` at "
                    f"docs/CONTRACTS.md:{row_line} "
                    f"(first listed at docs/CONTRACTS.md:{row_seen[key]})"
                )
                continue
            row_seen[key] = row_line

            entry = f"{field_name}: {_normalise_type(row.group('ty'))}"
            if kind == "topic":
                spec.topics.append(entry)
            else:
                spec.data.append(entry)
        events[name] = spec
    return events, duplicates


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------


def compare(
    source_events: dict[str, EventSpec], doc_events: dict[str, EventSpec]
) -> list[str]:
    errors: list[str] = []

    for name in sorted(source_events):
        if name not in doc_events:
            errors.append(f"event `{name}` is emitted but not documented")
            continue
        src = source_events[name]
        doc = doc_events[name]
        if src.signature() != doc.signature():
            errors.append(
                f"event `{name}` payload mismatch:\n"
                f"    source: topics={src.topics} data={src.data} "
                f"data_format={src.data_format}\n"
                f"    docs:   topics={doc.topics} data={doc.data} "
                f"data_format={doc.data_format}"
            )

    for name in sorted(doc_events):
        if name not in source_events:
            errors.append(f"event `{name}` is documented but not emitted")

    return errors


def collect_source_events() -> tuple[dict[str, EventSpec], list[str]]:
    events: dict[str, EventSpec] = {}
    duplicates: list[str] = []
    for path in sorted(CONTRACTS_DIR.rglob("*.rs")):
        origin = str(path.relative_to(ROOT))
        parsed, dup_errors = parse_rust_events(
            path.read_text(encoding="utf-8"), origin=origin
        )
        events.update(parsed)
        duplicates.extend(dup_errors)
    return events, duplicates


def main() -> int:
    if not DOCS.exists():
        print(f"error: {DOCS} not found", file=sys.stderr)
        return 1

    source_events, source_dups = collect_source_events()
    doc_events, doc_dups = parse_doc_events(DOCS.read_text(encoding="utf-8"))

    duplicates = source_dups + doc_dups
    if duplicates:
        print("Duplicate event definitions detected:", file=sys.stderr)
        for dup in duplicates:
            print(f"  - {dup}", file=sys.stderr)
        return 1

    errors = compare(source_events, doc_events)
    if errors:
        print("Event payload check failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(f"OK: {len(source_events)} event payloads match docs/CONTRACTS.md")
    return 0


if __name__ == "__main__":
    sys.exit(main())
