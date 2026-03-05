#!/usr/bin/env python3
"""Enforce line coverage for non-test library code under src/**."""

from __future__ import annotations

from pathlib import Path
import argparse
import re
import sys


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lcov", required=True, help="Path to LCOV file")
    parser.add_argument("--root", default=".", help="Repository root")
    parser.add_argument("--require", type=float, default=100.0, help="Required line coverage percent")
    return parser.parse_args()


def cfg_test_excluded_lines(path: Path) -> set[int]:
    lines = path.read_text(encoding="utf-8").splitlines()
    excluded: set[int] = set()
    i = 0
    while i < len(lines):
        if lines[i].strip().startswith("#[cfg(test)]"):
            j = i + 1
            while j < len(lines) and not lines[j].strip():
                j += 1
            if j >= len(lines):
                break
            if not re.match(r"^(?:pub\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*\{", lines[j].strip()):
                i += 1
                continue

            depth = lines[j].count("{") - lines[j].count("}")
            k = j
            while depth > 0 and k + 1 < len(lines):
                k += 1
                depth += lines[k].count("{") - lines[k].count("}")

            for line_no in range(i + 1, k + 2):
                excluded.add(line_no)
            i = k + 1
            continue
        i += 1
    return excluded


def parse_lcov(path: Path) -> dict[Path, dict[int, int]]:
    data: dict[Path, dict[int, int]] = {}
    current: Path | None = None
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line.startswith("SF:"):
            current = Path(line[3:]).resolve()
            data.setdefault(current, {})
            continue
        if current is None or not line.startswith("DA:"):
            continue
        line_no_str, hits_str = line[3:].split(",", 1)
        line_no = int(line_no_str)
        hits = int(hits_str)
        # Keep max hits in case of repeated DA entries.
        previous = data[current].get(line_no, 0)
        if hits > previous:
            data[current][line_no] = hits
    return data


def main() -> int:
    args = parse_args()
    root = Path(args.root).resolve()
    lcov_path = Path(args.lcov).resolve()

    coverage = parse_lcov(lcov_path)

    total = 0
    covered = 0
    uncovered: list[tuple[Path, int]] = []

    for source, line_hits in sorted(coverage.items()):
        try:
            rel = source.relative_to(root)
        except ValueError:
            continue

        rel_str = rel.as_posix()
        if not rel_str.startswith("src/"):
            continue

        excluded = cfg_test_excluded_lines(source)
        for line_no, hits in sorted(line_hits.items()):
            if line_no in excluded:
                continue
            total += 1
            if hits > 0:
                covered += 1
            else:
                uncovered.append((source, line_no))

    if total == 0:
        print("no executable library lines found under src/**")
        return 1

    percent = (covered / total) * 100.0
    print(f"library line coverage: {covered}/{total} ({percent:.2f}%)")

    if percent + 1e-9 < args.require:
        print(f"required: {args.require:.2f}%")
        if uncovered:
            print("uncovered library lines:")
            for source, line_no in uncovered:
                print(f"- {source}:{line_no}")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
