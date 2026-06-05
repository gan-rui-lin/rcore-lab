#!/usr/bin/env python3
"""
Score rcore-lab OS competition logs with the official judge scripts.

This script does not reimplement scoring. It extracts test-group segments from
QEMU logs, feeds each segment to the corresponding official judge script, and
only aggregates the official JSON output.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


START_RE = re.compile(r"#### OS COMP TEST GROUP START ([A-Za-z0-9_-]+-(musl|glibc)) ####")
END_RE = re.compile(r"#### OS COMP TEST GROUP END ([A-Za-z0-9_-]+-(musl|glibc)) ####")


@dataclass
class Segment:
    suite: str
    data: bytes
    start_line: int
    end_line: int | None

    @property
    def complete(self) -> bool:
        return self.end_line is not None

    @property
    def line_range(self) -> str:
        end = str(self.end_line) if self.end_line is not None else "EOF"
        return f"{self.start_line}-{end}"


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def find_judge_dir(explicit: str | None) -> Path:
    if explicit:
        judge_dir = Path(explicit).expanduser().resolve()
    else:
        candidates = [
            repo_root().parent / "autotest-for-oskernel" / "kernel" / "judge",
            Path.cwd().resolve().parent / "autotest-for-oskernel" / "kernel" / "judge",
        ]
        judge_dir = next((p for p in candidates if p.exists()), candidates[0])

    if not judge_dir.is_dir():
        raise FileNotFoundError(f"official judge directory not found: {judge_dir}")
    return judge_dir


def normalize_suite(value: str) -> str:
    value = value.strip()
    if not value:
        return "auto"
    if value in {"all", "auto"}:
        return value
    if re.fullmatch(r"[A-Za-z0-9_-]+", value):
        return value
    raise ValueError(f"invalid suite selector: {value}")


def split_segments(log_path: Path) -> list[Segment]:
    lines = log_path.read_bytes().splitlines(keepends=True)
    segments: list[Segment] = []
    active_suite: str | None = None
    active_start = 0
    active_lines: list[bytes] = []

    for lineno, raw_line in enumerate(lines, 1):
        line = raw_line.decode("latin-1", errors="replace").strip()
        start_match = START_RE.search(line)
        end_match = END_RE.search(line)

        if start_match:
            if active_suite is not None:
                segments.append(
                    Segment(
                        suite=active_suite,
                        data=b"".join(active_lines),
                        start_line=active_start,
                        end_line=None,
                    )
                )
            active_suite = start_match.group(1)
            active_start = lineno
            active_lines = [raw_line]
            continue

        if active_suite is not None:
            active_lines.append(raw_line)
            if end_match and end_match.group(1) == active_suite:
                segments.append(
                    Segment(
                        suite=active_suite,
                        data=b"".join(active_lines),
                        start_line=active_start,
                        end_line=lineno,
                    )
                )
                active_suite = None
                active_start = 0
                active_lines = []

    if active_suite is not None:
        segments.append(
            Segment(
                suite=active_suite,
                data=b"".join(active_lines),
                start_line=active_start,
                end_line=None,
            )
        )

    return segments


def whole_file_segment(log_path: Path, suite: str) -> Segment:
    return Segment(
        suite=suite,
        data=log_path.read_bytes(),
        start_line=1,
        end_line=None,
    )


def select_segments(log_path: Path, suite: str) -> list[Segment]:
    marked = split_segments(log_path)
    if suite in {"auto", "all"}:
        return marked
    if suite in {"musl", "glibc"}:
        return [seg for seg in marked if seg.suite.endswith(f"-{suite}")]
    if "-" not in suite:
        return [seg for seg in marked if seg.suite.rsplit("-", 1)[0] == suite]

    selected = [seg for seg in marked if seg.suite == suite]
    if selected:
        return selected
    if marked:
        return []
    return [whole_file_segment(log_path, suite)]


def run_official_judge(judge_dir: Path, suite: str, data: bytes) -> list[dict[str, Any]]:
    judge = judge_dir / f"judge_{suite}.py"
    if not judge.is_file():
        raise FileNotFoundError(f"official judge script not found: {judge}")
    clean_data = data.replace(b"\r", b"")
    proc = subprocess.run(
        [sys.executable, str(judge)],
        input=clean_data,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"{judge.name} failed with code {proc.returncode}: {stderr}")

    text = proc.stdout.decode("utf-8", errors="replace").strip()
    if not text:
        return []
    data_obj = json.loads(text)
    if not isinstance(data_obj, list):
        raise ValueError(f"{judge.name} did not return a JSON list")
    return [row for row in data_obj if isinstance(row, dict)]


def to_int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def to_float(value: Any) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def has_count_fields(row: dict[str, Any]) -> bool:
    return "pass" in row or "all" in row or "total" in row


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    normalized = [
        {
            "name": str(row.get("name", "")),
            "pass": to_int(row.get("pass", 0)),
            "all": to_int(row.get("all", row.get("total", 0))),
            "score": to_float(row.get("score", row.get("pass", 0))),
            "counted": has_count_fields(row),
        }
        for row in rows
    ]
    total_cases = len(normalized)
    total_pass = sum(row["pass"] for row in normalized)
    total_all = sum(row["all"] for row in normalized)
    total_score = sum(row["score"] for row in normalized)
    counted_rows = [row for row in normalized if row["counted"]]
    return {
        "cases": total_cases,
        "pass": total_pass,
        "all": total_all,
        "score": total_score,
        "score_rate": (total_score / total_all) if total_all else 0.0,
        "full": sum(1 for row in counted_rows if row["all"] > 0 and row["pass"] == row["all"]),
        "partial": sum(1 for row in counted_rows if 0 < row["pass"] < row["all"]),
        "zero": sum(1 for row in counted_rows if row["all"] > 0 and row["pass"] == 0),
        "no_stat": sum(1 for row in counted_rows if row["all"] == 0),
    }


def score_log(log_path: Path, suite: str, judge_dir: Path, complete_only: bool) -> list[dict[str, Any]]:
    segments = select_segments(log_path, suite)
    if complete_only:
        segments = [seg for seg in segments if seg.complete]

    results: list[dict[str, Any]] = []
    for seg in segments:
        rows = run_official_judge(judge_dir, seg.suite, seg.data)
        summary = summarize(rows)
        results.append(
            {
                "log": str(log_path),
                "suite": seg.suite,
                "complete": seg.complete,
                "line_range": seg.line_range,
                "judge": f"judge_{seg.suite}.py",
                **summary,
            }
        )
    return results


def total_row(rows: list[dict[str, Any]]) -> dict[str, Any]:
    total_all = sum(to_int(row["all"]) for row in rows)
    total_score = sum(to_float(row["score"]) for row in rows)
    total_pass = sum(to_int(row["pass"]) for row in rows)
    return {
        "log": "TOTAL",
        "suite": "-",
        "complete": all(bool(row["complete"]) for row in rows),
        "line_range": "-",
        "judge": "-",
        "cases": sum(to_int(row["cases"]) for row in rows),
        "pass": total_pass,
        "all": total_all,
        "score": total_score,
        "score_rate": (total_pass / total_all) if total_all else 0.0,
        "full": sum(to_int(row["full"]) for row in rows),
        "partial": sum(to_int(row["partial"]) for row in rows),
        "zero": sum(to_int(row["zero"]) for row in rows),
        "no_stat": sum(to_int(row["no_stat"]) for row in rows),
    }


def format_bool(value: bool) -> str:
    return "yes" if value else "no"


def format_number(value: Any) -> str:
    num = to_float(value)
    if abs(num - round(num)) < 1e-9:
        return str(int(round(num)))
    return f"{num:.4f}"


def print_markdown(rows: list[dict[str, Any]], include_total: bool) -> None:
    output_rows = list(rows)
    if include_total and len(rows) > 1:
        output_rows.append(total_row(rows))

    print("| suite | cases | pass | all | score |")
    print("|---|---:|---:|---:|---:|")
    for row in output_rows:
        suite = "TOTAL" if row["log"] == "TOTAL" else row["suite"]
        print(
            f"| {suite} "
            f"| {row['cases']} "
            f"| {row['pass']} "
            f"| {row['all']} "
            f"| {format_number(row['score'])} |"
        )


def print_csv(rows: list[dict[str, Any]], include_total: bool) -> None:
    output_rows = list(rows)
    if include_total and len(rows) > 1:
        output_rows.append(total_row(rows))
    fieldnames = [
        "log",
        "suite",
        "complete",
        "line_range",
        "cases",
        "pass",
        "all",
        "score",
        "score_rate",
        "full",
        "partial",
        "zero",
        "no_stat",
        "judge",
    ]
    writer = csv.DictWriter(sys.stdout, fieldnames=fieldnames)
    writer.writeheader()
    for row in output_rows:
        writer.writerow(row)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Score OS competition logs by calling the official judge_*.py scripts."
    )
    parser.add_argument("logs", nargs="+", help="QEMU log file(s)")
    parser.add_argument(
        "--suite",
        default="auto",
        help=(
            "suite selector: auto/all for all marked groups, musl/glibc for one libc, "
            "ltp/basic/lmbench for both libcs of a suite, or exact group like ltp-musl"
        ),
    )
    parser.add_argument(
        "--judge-dir",
        help="path to autotest-for-oskernel/kernel/judge (default: auto-detect sibling repo)",
    )
    parser.add_argument(
        "--complete-only",
        action="store_true",
        help="skip LTP groups that have START but no END marker",
    )
    parser.add_argument(
        "--format",
        choices=["markdown", "json", "csv"],
        default="markdown",
        help="output format (default: markdown)",
    )
    parser.add_argument(
        "--no-total",
        action="store_true",
        help="do not append a TOTAL row when multiple segments are scored",
    )
    args = parser.parse_args()

    try:
        suite = normalize_suite(args.suite)
        judge_dir = find_judge_dir(args.judge_dir)
        all_rows: list[dict[str, Any]] = []
        for raw_log in args.logs:
            log_path = Path(raw_log).expanduser().resolve()
            if not log_path.is_file():
                raise FileNotFoundError(f"log file not found: {log_path}")
            rows = score_log(log_path, suite, judge_dir, args.complete_only)
            if not rows:
                print(f"[WARN] no matching LTP segment in {log_path}", file=sys.stderr)
            all_rows.extend(rows)
    except Exception as exc:  # noqa: BLE001
        print(f"[ERROR] {exc}", file=sys.stderr)
        return 1

    include_total = not args.no_total
    if args.format == "json":
        output = list(all_rows)
        if include_total and len(all_rows) > 1:
            output.append(total_row(all_rows))
        print(json.dumps(output, ensure_ascii=False, indent=2))
    elif args.format == "csv":
        print_csv(all_rows, include_total)
    else:
        print_markdown(all_rows, include_total)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
