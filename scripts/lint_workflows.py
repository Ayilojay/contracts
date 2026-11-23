#!/usr/bin/env python3
"""Lint every GitHub Actions workflow file in .github/workflows/.

Checks:
  - Valid YAML.
  - Every job has either (runs-on + steps) or a reusable-workflow reference
    (jobs.<id>.uses).  Jobs that have a name but no runner, no steps, and no
    reusable reference are incomplete stubs and are rejected.

Usage:
    python3 scripts/lint_workflows.py

Exit code 0 when all workflow files pass, 1 otherwise.
"""

from __future__ import annotations

import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS_DIR = ROOT / ".github" / "workflows"


def _lint_file(path: Path) -> list[str]:
    """Return a list of error strings for a single workflow file."""
    errors: list[str] = []

    try:
        doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    except yaml.YAMLError as exc:
        return [f"{path.name}: YAML parse error: {exc}"]

    if not isinstance(doc, dict):
        return [f"{path.name}: top-level value is not a mapping"]

    jobs = doc.get("jobs")
    if not isinstance(jobs, dict):
        # A file with no jobs section is valid (e.g. reusable actions), but
        # an explicitly empty or malformed one is not.
        if jobs is not None:
            errors.append(f"{path.name}: `jobs` is not a mapping")
        return errors

    for job_id, job in jobs.items():
        if not isinstance(job, dict):
            errors.append(f"{path.name}: job `{job_id}` is not a mapping")
            continue

        has_steps = isinstance(job.get("steps"), list) and len(job["steps"]) > 0
        has_runner = "runs-on" in job
        has_uses = "uses" in job  # reusable workflow reference

        if has_uses:
            # A reusable-workflow job needs only `uses`; runner and steps are
            # not permitted alongside it.
            if has_runner or has_steps:
                errors.append(
                    f"{path.name}: job `{job_id}` mixes `uses` with "
                    f"`runs-on`/`steps` (not allowed for reusable workflows)"
                )
        elif has_runner and has_steps:
            pass  # fully defined normal job
        elif has_runner and not has_steps:
            errors.append(
                f"{path.name}: job `{job_id}` has `runs-on` but no `steps`"
            )
        elif has_steps and not has_runner:
            errors.append(
                f"{path.name}: job `{job_id}` has `steps` but no `runs-on`"
            )
        else:
            # Neither a reusable reference nor a runner+steps definition.
            errors.append(
                f"{path.name}: job `{job_id}` is incomplete "
                f"(missing `runs-on` and `steps`, or a reusable `uses` reference)"
            )

    return errors


def lint(workflows_dir: Path = WORKFLOWS_DIR) -> list[str]:
    """Lint all workflow files under *workflows_dir* and return all errors."""
    errors: list[str] = []
    yml_files = sorted(workflows_dir.glob("*.yml")) + sorted(
        workflows_dir.glob("*.yaml")
    )
    if not yml_files:
        errors.append(f"no workflow files found in {workflows_dir}")
        return errors
    for path in yml_files:
        errors.extend(_lint_file(path))
    return errors


def main() -> int:
    errors = lint()
    if errors:
        for err in errors:
            print(err, file=sys.stderr)
        return 1
    workflow_count = len(
        list(WORKFLOWS_DIR.glob("*.yml")) + list(WORKFLOWS_DIR.glob("*.yaml"))
    )
    print(f"ok: {workflow_count} workflow file(s) passed lint")
    return 0


if __name__ == "__main__":
    sys.exit(main())
