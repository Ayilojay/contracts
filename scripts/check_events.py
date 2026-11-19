#!/usr/bin/env python3
"""Validate contract event definitions against docs/CONTRACTS.md.

Compares, for every event emitted by the contracts, the full payload shape
(indexed fields, data fields, field ordering and data_format) against the
tables documented in docs/CONTRACTS.md. Topic-name-only comparison is not
enough: a wrong payload table (e.g. the retirement event) would otherwise
pass silently.

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


def parse_rust_events(source: str) -> dict[str, EventSpec]:
    """Return {event_name: EventSpec} parsed from a Rust source file."""

    events: dict[str, EventSpec] = {}
    for match in CONTRACT_EVENT_RE.finditer(source):
        name = match.group("name")
        body = match.group("body")
        header = source[match.start() : match.start("body")]

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
    return events


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


def parse_doc_events(text: str) -> dict[str, EventSpec]:
    """Return {event_name: EventSpec} parsed from docs/CONTRACTS.md."""

    events: dict[str, EventSpec] = {}
    headings = list(EVENT_HEADING_RE.finditer(text))
    for idx, heading in enumerate(headings):
        name = heading.group("name")
        start = heading.end()
        end = headings[idx + 1].start() if idx + 1 < len(headings) else len(text)
        section = text[start:end]

        rows = list(TABLE_ROW_RE.finditer(section))
        if not rows:
            continue

        fmt_match = DOC_DATA_FORMAT_RE.search(section)
        spec = EventSpec(
            data_format=fmt_match.group("fmt") if fmt_match else "single-value"
        )
        for row in rows:
            entry = f"{row.group('field').strip()}: {_normalise_type(row.group('ty'))}"
            if row.group("kind").lower() == "topic":
                spec.topics.append(entry)
            else:
                spec.data.append(entry)
        events[name] = spec
    return events


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


def collect_source_events() -> dict[str, EventSpec]:
    events: dict[str, EventSpec] = {}
    for path in sorted(CONTRACTS_DIR.rglob("*.rs")):
        events.update(parse_rust_events(path.read_text(encoding="utf-8")))
    return events


def main() -> int:
    if not DOCS.exists():
        print(f"error: {DOCS} not found", file=sys.stderr)
        return 1

    source_events = collect_source_events()
    doc_events = parse_doc_events(DOCS.read_text(encoding="utf-8"))

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
