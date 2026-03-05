#!/usr/bin/env python3
"""Validate compliance matrix/index consistency and test references."""

from __future__ import annotations

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "docs/compliance/http11-get-head-rfc-matrix.md"
INDEX = ROOT / "docs/compliance/http11-get-head-requirement-test-index.md"
ALLOWED_STATUSES = {"implemented", "not applicable"}


def parse_markdown_table(path: Path) -> list[list[str]]:
    if not path.is_file():
        raise ValueError(f"required file not found: {path}")
    rows: list[list[str]] = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line.startswith("|"):
            continue
        if re.match(r"^\|\s*:?-{3,}", line):
            continue
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if not cells:
            continue
        if cells[0].lower() == "id":
            continue
        rows.append(cells)
    return rows


def parse_matrix(path: Path) -> dict[str, str]:
    matrix: dict[str, str] = {}
    for cells in parse_markdown_table(path):
        if len(cells) < 4:
            continue
        req_id = cells[0]
        if not re.match(r"^\d{4}-", req_id):
            continue
        status = cells[3].strip().lower()
        if req_id in matrix:
            raise ValueError(f"duplicate requirement ID in matrix: {req_id}")
        if status not in ALLOWED_STATUSES:
            allowed = ", ".join(sorted(ALLOWED_STATUSES))
            raise ValueError(
                f"invalid status for {req_id!r}: {status!r} (allowed: {allowed})"
            )
        matrix[req_id] = status
    if not matrix:
        raise ValueError("no requirement rows found in matrix")
    return matrix


def parse_index(path: Path) -> dict[str, str]:
    index: dict[str, str] = {}
    for cells in parse_markdown_table(path):
        if len(cells) < 2:
            continue
        req_id = cells[0]
        if not re.match(r"^\d{4}-", req_id):
            continue
        tests = cells[1].strip()
        if req_id in index:
            raise ValueError(f"duplicate requirement ID in index: {req_id}")
        index[req_id] = tests
    if not index:
        raise ValueError("no requirement rows found in index")
    return index


def collect_rust_function_names(root: Path) -> set[str]:
    names: set[str] = set()
    pattern = re.compile(r"(?m)^\s*fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(")
    for rel in ("src", "tests"):
        base = root / rel
        if not base.exists():
            continue
        for file in base.rglob("*.rs"):
            names.update(pattern.findall(file.read_text(encoding="utf-8")))
    return names


def extract_test_names(cell: str) -> list[str]:
    return re.findall(r"`([^`]+)`", cell)


def main() -> int:
    try:
        matrix = parse_matrix(MATRIX)
        index = parse_index(INDEX)

        matrix_ids = set(matrix)
        index_ids = set(index)

        errors: list[str] = []

        missing_in_index = sorted(matrix_ids - index_ids)
        if missing_in_index:
            errors.append("IDs missing in index: " + ", ".join(missing_in_index))

        extra_in_index = sorted(index_ids - matrix_ids)
        if extra_in_index:
            errors.append("IDs present only in index: " + ", ".join(extra_in_index))

        function_names = collect_rust_function_names(ROOT)

        for req_id in sorted(matrix_ids & index_ids):
            status = matrix[req_id]
            test_cell = index[req_id].strip()

            if status == "not applicable":
                if test_cell.lower() != "n/a":
                    errors.append(
                        f"{req_id}: status is 'not applicable' but tests cell is {test_cell!r}"
                    )
                continue

            tests = extract_test_names(test_cell)
            if not tests:
                errors.append(f"{req_id}: implemented rows must reference at least one test")
                continue

            missing_tests = sorted(test for test in tests if test not in function_names)
            if missing_tests:
                errors.append(
                    f"{req_id}: referenced tests not found: {', '.join(missing_tests)}"
                )

        if errors:
            print("compliance docs check failed:")
            for error in errors:
                print(f"- {error}")
            return 1

        print(
            f"compliance docs check passed ({len(matrix_ids)} requirements, "
            f"{sum(1 for s in matrix.values() if s == 'implemented')} implemented, "
            f"{sum(1 for s in matrix.values() if s == 'not applicable')} not applicable)"
        )
        return 0
    except (OSError, ValueError) as error:
        print("compliance docs check failed:")
        print(f"- {error}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
