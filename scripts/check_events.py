#!/usr/bin/env python3
"""Validate contract event and function definitions against docs/CONTRACTS.md.

Compares, for every event emitted by the contracts, the full payload shape
(indexed fields, data fields, field ordering and data_format) against the
tables documented in docs/CONTRACTS.md. Topic-name-only comparison is not
enough: a wrong payload table (e.g. the retirement event) would otherwise
pass silently.

Also compares exported contract method names, their parameter names/types
and return types against the documented function reference, so obsolete
entrypoints and outdated argument lists cannot linger in the docs.

Duplicate rows and duplicate discriminator definitions are reported with
source locations before any set comparison, so they cannot be silently
discarded.

Usage:
    python3 scripts/check_events.py

Exit code 0 when every event and function matches, 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

from check_errors import check_error_tables

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


@dataclass
class FunctionSpec:
    """Structured description of a single contract entrypoint."""

    params: list[str] = field(default_factory=list)
    returns: str = "()"

    def signature(self) -> tuple[tuple[str, ...], str]:
        return (tuple(self.params), self.returns)


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

# Matches exported contract entrypoints: `pub fn name(...) -> Ret {`.
CONTRACT_FN_RE = re.compile(
    r"pub\s+fn\s+(?P<name>\w+)\s*\((?P<params>[^)]*)\)"
    r"\s*(?:->\s*(?P<ret>[^{]+?))?\s*\{",
    re.DOTALL,
)

# Matches a single function parameter: `name: Type`.
PARAM_RE = re.compile(r"(?:mut\s+)?(?P<name>\w+)\s*:\s*(?P<ty>.+)", re.DOTALL)


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


def parse_rust_functions(
    source: str, origin: str = "<source>"
) -> tuple[dict[str, FunctionSpec], list[str]]:
    """Return ({fn_name: FunctionSpec}, duplicate_errors) from a Rust file.

    Only `pub fn` entrypoints are considered; duplicate definitions are
    reported with their source locations.
    """

    functions: dict[str, FunctionSpec] = {}
    seen_at: dict[str, int] = {}
    duplicates: list[str] = []
    for match in CONTRACT_FN_RE.finditer(source):
        name = match.group("name")
        line = _line_of(source, match.start())
        if name in seen_at:
            duplicates.append(
                f"duplicate function definition `{name}` at {origin}:{line} "
                f"(first defined at {origin}:{seen_at[name]})"
            )
            continue
        seen_at[name] = line

        spec = FunctionSpec()
        raw_params = match.group("params").strip()
        if raw_params:
            for raw in raw_params.split(","):
                raw = raw.strip()
                if not raw:
                    continue
                p_match = PARAM_RE.match(raw)
                if p_match:
                    spec.params.append(
                        f"{p_match.group('name')}: "
                        f"{_normalise_type(p_match.group('ty'))}"
                    )
                else:
                    spec.params.append(_normalise_type(raw))
        ret = match.group("ret")
        spec.returns = _normalise_type(ret) if ret else "()"
        functions[name] = spec
    return functions, duplicates


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

# A documented function reference looks like:
#
#   ### `deposit`
#
#   | Param  | Type   |
#   |--------|--------|
#   | amount | i128   |
#
#   returns: i128
FUNCTION_HEADING_RE = re.compile(
    r"^#{2,4}\s+`?(?P<name>\w+)`?\s*$", re.MULTILINE
)
PARAM_ROW_RE = re.compile(
    r"^\|\s*(?P<param>[^|]+?)\s*\|\s*(?P<ty>[^|]+?)\s*\|",
    re.MULTILINE,
)
DOC_RETURNS_RE = re.compile(r"returns\s*:\s*(?P<ret>[^\n]+)", re.IGNORECASE)

# Controlled allowlist of non-interface prose headings that must never be
# treated as function-reference sections. Extend deliberately, not casually.
FUNCTION_PROSE_ALLOWLIST = frozenset(
    {
        "overview",
        "events",
        "storage",
        "errors",
        "notes",
        "compatibility",
        "security",
        "gas",
    }
)


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


def parse_doc_functions(text: str) -> tuple[dict[str, FunctionSpec], list[str]]:
    """Return ({fn_name: FunctionSpec}, duplicate_errors) from the docs.

    Only sections that contain a `returns:` marker are treated as function
    references; prose headings in FUNCTION_PROSE_ALLOWLIST are skipped.
    """

    functions: dict[str, FunctionSpec] = {}
    seen_at: dict[str, int] = {}
    duplicates: list[str] = []
    headings = list(FUNCTION_HEADING_RE.finditer(text))
    for idx, heading in enumerate(headings):
        name = heading.group("name")
        if name.lower() in FUNCTION_PROSE_ALLOWLIST:
            continue
        start = heading.end()
        end = headings[idx + 1].start() if idx + 1 < len(headings) else len(text)
        section = text[start:end]

        ret_match = DOC_RETURNS_RE.search(section)
        if not ret_match:
            continue

        heading_line = _line_of(text, heading.start())
        if name in seen_at:
            duplicates.append(
                f"duplicate function section `{name}` at docs/CONTRACTS.md:{heading_line} "
                f"(first documented at docs/CONTRACTS.md:{seen_at[name]})"
            )
            continue
        seen_at[name] = heading_line

        spec = FunctionSpec(returns=_normalise_type(ret_match.group("ret")))
        for row in PARAM_ROW_RE.finditer(section):
            param = row.group("param").strip()
            if param.lower() in {"param", "parameter", "name"}:
                continue
            if set(param) <= {"-", ":"}:
                continue
            spec.params.append(f"{param}: {_normalise_type(row.group('ty'))}")
        functions[name] = spec
    return functions, duplicates


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


def compare_functions(
    source_fns: dict[str, FunctionSpec], doc_fns: dict[str, FunctionSpec]
) -> list[str]:
    errors: list[str] = []

    for name in sorted(source_fns):
        if name not in doc_fns:
            errors.append(f"function `{name}` is exported but not documented")
            continue
        src = source_fns[name]
        doc = doc_fns[name]
        if src.signature() != doc.signature():
            errors.append(
                f"function `{name}` signature mismatch:\n"
                f"    source: params={src.params} returns={src.returns}\n"
                f"    docs:   params={doc.params} returns={doc.returns}"
            )

    for name in sorted(doc_fns):
        if name not in source_fns:
            errors.append(f"function `{name}` is documented but not exported")

    return errors


def main() -> int:
    if not DOCS.exists():
        print(f"error: {DOCS} not found", file=sys.stderr)
        return 1

    text = DOCS.read_text(encoding="utf-8")
    doc_events, doc_event_dupes = parse_doc_events(text)
    doc_fns, doc_fn_dupes = parse_doc_functions(text)

    source_events: dict[str, EventSpec] = {}
    source_fns: dict[str, FunctionSpec] = {}
    errors: list[str] = []
    errors.extend(doc_event_dupes)
    errors.extend(doc_fn_dupes)

    for path in sorted(CONTRACTS_DIR.rglob("*.rs")):
        origin = str(path.relative_to(ROOT))
        source = path.read_text(encoding="utf-8")
        events, event_dupes = parse_rust_events(source, origin)
        fns, fn_dupes = parse_rust_functions(source, origin)
        errors.extend(event_dupes)
        errors.extend(fn_dupes)
        for name, spec in events.items():
            if name in source_events:
                errors.append(f"event `{name}` defined in multiple files")
                continue
            source_events[name] = spec
        for name, spec in fns.items():
            if name in source_fns:
                errors.append(f"function `{name}` defined in multiple files")
                continue
            source_fns[name] = spec

    errors.extend(compare(source_events, doc_events))
    errors.extend(compare_functions(source_fns, doc_fns))

    # Cross-reference each contract's documented error codes against its enum.
    errors.extend(check_error_tables())

    if errors:
        for err in errors:
            print(err, file=sys.stderr)
        return 1

    print(
        f"ok: {len(source_events)} events and {len(source_fns)} functions match "
        f"docs/CONTRACTS.md"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

