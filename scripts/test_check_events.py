"""Tests for scripts/check_events.py.

Focused regression coverage for duplicate event documentation rows and
duplicate discriminator definitions, plus distinct topics with valid
payloads (issue #242).
"""

import importlib.util
import sys
from pathlib import Path

import pytest


SCRIPT_PATH = Path(__file__).resolve().parent / "check_events.py"


def _load_check_events():
    spec = importlib.util.spec_from_file_location("check_events", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_events"] = module
    spec.loader.exec_module(module)
    return module


check_events = _load_check_events()


VALID_PAYLOAD = "(address, i128)"


def _write(path: Path, text: str) -> Path:
    path.write_text(text, encoding="utf-8")
    return path


def test_distinct_topics_with_valid_payloads_pass(tmp_path):
    rust = _write(
        tmp_path / "events.rs",
        """
        pub fn emit_transfer() {
            env.events().publish((symbol_short!("transfer"),), (from, amount));
        }

        pub fn emit_mint() {
            env.events().publish((symbol_short!("mint"),), (to, amount));
        }
        """,
    )
    docs = _write(
        tmp_path / "EVENTS.md",
        """
        | Topic | Payload |
        | --- | --- |
        | transfer | (address, i128) |
        | mint | (address, i128) |
        """,
    )

    result = check_events.check(rust_path=rust, docs_path=docs)
    assert result.duplicates == []
    assert result.ok


def test_duplicate_documentation_rows_are_reported(tmp_path):
    rust = _write(
        tmp_path / "events.rs",
        """
        pub fn emit_transfer() {
            env.events().publish((symbol_short!("transfer"),), (from, amount));
        }
        """,
    )
    docs = _write(
        tmp_path / "EVENTS.md",
        """
        | Topic | Payload |
        | --- | --- |
        | transfer | (address, i128) |
        | transfer | (address, i128) |
        """,
    )

    result = check_events.check(rust_path=rust, docs_path=docs)
    assert not result.ok
    assert any("transfer" in d.message for d in result.duplicates)
    # Duplicate must be reported with a source location (line number).
    assert all(d.line is not None for d in result.duplicates)


def test_duplicate_discriminator_definitions_are_reported(tmp_path):
    rust = _write(
        tmp_path / "events.rs",
        """
        pub fn emit_transfer() {
            env.events().publish((symbol_short!("transfer"),), (from, amount));
        }

        pub fn emit_transfer_again() {
            env.events().publish((symbol_short!("transfer"),), (from, amount));
        }
        """,
    )
    docs = _write(
        tmp_path / "EVENTS.md",
        """
        | Topic | Payload |
        | --- | --- |
        | transfer | (address, i128) |
        """,
    )

    result = check_events.check(rust_path=rust, docs_path=docs)
    assert not result.ok
    assert any("transfer" in d.message for d in result.duplicates)
    assert all(d.line is not None for d in result.duplicates)
