"""Tests for scripts/lint_workflows.py.

Covers the two normal-job configurations (complete and incomplete), the
reusable-workflow reference form, and the invalid mixed form.  An incomplete
job — one that has a name but no runs-on, no steps, and no uses reference —
must produce at least one error so it cannot slip past CI undetected.
"""

import importlib.util
import sys
import textwrap
from pathlib import Path

import pytest

SCRIPT_PATH = Path(__file__).resolve().parent / "lint_workflows.py"


def _load_lint_workflows():
    spec = importlib.util.spec_from_file_location("lint_workflows", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules["lint_workflows"] = module
    spec.loader.exec_module(module)
    return module


lint_workflows = _load_lint_workflows()


def _write(tmp_path: Path, filename: str, text: str) -> Path:
    p = tmp_path / filename
    p.write_text(textwrap.dedent(text), encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# Fixtures: complete jobs pass
# ---------------------------------------------------------------------------


def test_complete_normal_job_passes(tmp_path):
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          build:
            runs-on: ubuntu-latest
            steps:
              - uses: actions/checkout@v4
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors == [], f"unexpected errors: {errors}"


def test_reusable_workflow_job_passes(tmp_path):
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          call-other:
            uses: org/repo/.github/workflows/other.yml@main
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors == [], f"unexpected errors: {errors}"


# ---------------------------------------------------------------------------
# Fixtures: incomplete jobs fail
# ---------------------------------------------------------------------------


def test_incomplete_job_no_runner_no_steps_fails(tmp_path):
    """A job stub with only a name must be rejected."""
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          contracttype-size:
            name: Contract type encoding size
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors, "expected at least one error for an incomplete job stub"
    assert any("contracttype-size" in e for e in errors), (
        f"error should name the offending job; got: {errors}"
    )


def test_job_with_runner_but_no_steps_fails(tmp_path):
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          orphan:
            runs-on: ubuntu-latest
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors, "expected an error for a job with runs-on but no steps"
    assert any("orphan" in e for e in errors)


def test_job_with_steps_but_no_runner_fails(tmp_path):
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          orphan:
            steps:
              - run: echo hi
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors, "expected an error for a job with steps but no runs-on"
    assert any("orphan" in e for e in errors)


def test_reusable_job_mixed_with_runner_fails(tmp_path):
    _write(
        tmp_path,
        "ci.yml",
        """\
        name: CI
        on: push
        jobs:
          bad:
            uses: org/repo/.github/workflows/other.yml@main
            runs-on: ubuntu-latest
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert errors, "expected an error for a reusable job mixed with runs-on"
    assert any("bad" in e for e in errors)


# ---------------------------------------------------------------------------
# YAML parse error
# ---------------------------------------------------------------------------


def test_invalid_yaml_fails(tmp_path):
    p = tmp_path / "ci.yml"
    p.write_text(": this: is: not: valid: yaml: [\n", encoding="utf-8")
    errors = lint_workflows.lint(tmp_path)
    assert errors, "expected a parse error for invalid YAML"


# ---------------------------------------------------------------------------
# Multiple files: errors from each file are collected
# ---------------------------------------------------------------------------


def test_multiple_files_errors_aggregated(tmp_path):
    _write(
        tmp_path,
        "a.yml",
        """\
        name: A
        on: push
        jobs:
          stub-a:
            name: Stub A
        """,
    )
    _write(
        tmp_path,
        "b.yml",
        """\
        name: B
        on: push
        jobs:
          stub-b:
            name: Stub B
        """,
    )
    errors = lint_workflows.lint(tmp_path)
    assert any("stub-a" in e for e in errors)
    assert any("stub-b" in e for e in errors)
