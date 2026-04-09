#!/usr/bin/env python3
"""
ltp-analyze.py — parse LTP test output and rank tests by pass count.

Usage:
    python3 ltp-analyze.py [log_file] [--top N] [--csv] [--passed-only]
    cat results.log | python3 ltp-analyze.py

The script handles both ANSI-colored output (from PTY/script) and plain text.
It also handles both:
  - Line-based counting  (TPASS/TFAIL lines)
  - Summary-section parsing  (passed N / failed N blocks)

Output modes:
  default : pretty table sorted by score desc
  --csv   : CSV suitable for spreadsheet import
  --json  : raw JSON (same format as judge_ltp-musl.py)
"""

import sys
import re
import json
import argparse
from dataclasses import dataclass, field
from typing import Dict, List, Optional


# ANSI escape-stripped versions of result codes
ANSI_RE = re.compile(r'\x1b\[[0-9;]*m')

def strip_ansi(s: str) -> str:
    return ANSI_RE.sub('', s)


@dataclass
class CaseResult:
    name: str
    passed:   int = 0
    failed:   int = 0
    broken:   int = 0
    skipped:  int = 0
    warnings: int = 0
    timed_out: bool = False
    return_code: Optional[int] = None

    @property
    def total(self) -> int:
        return self.passed + self.failed + self.broken + self.skipped + self.warnings

    @property
    def score(self) -> int:
        return self.passed

    @property
    def status(self) -> str:
        if self.timed_out:
            return "TIMEOUT"
        if self.total == 0:
            return "EMPTY"
        if self.failed == 0 and self.broken == 0:
            return "PASS"
        if self.passed == 0:
            return "FAIL"
        return "PARTIAL"


def parse_ltp_log(content: str) -> Dict[str, CaseResult]:
    """
    Parse LTP output that looks like:

        RUN LTP CASE abort01
        ...  TPASS: ...
        ...  TFAIL: ...
        Summary:
        passed   2
        ...
        FAIL LTP CASE abort01 : 0

    Handles both ANSI-colored and plain text.
    """
    results: Dict[str, CaseResult] = {}
    current: Optional[CaseResult] = None
    in_summary = False

    # Strip carriage returns (PTY adds them)
    content = content.replace('\r\n', '\n').replace('\r', '\n')

    for raw_line in content.split('\n'):
        line = strip_ansi(raw_line).strip()

        # ── New test case starts ─────────────────────────────────────────────
        if line.startswith('RUN LTP CASE'):
            parts = line.split()
            if len(parts) >= 4:
                name = parts[-1]
                current = CaseResult(name=name)
                in_summary = False
            continue

        # ── Test case ends ───────────────────────────────────────────────────
        if current and line.startswith(f'FAIL LTP CASE {current.name}'):
            parts = line.split()
            try:
                rc = int(parts[-1])
                current.return_code = rc
                if rc == 124:
                    current.timed_out = True
            except (ValueError, IndexError):
                pass
            results[current.name] = current
            current = None
            in_summary = False
            continue

        if current is None:
            continue

        # ── Summary section ──────────────────────────────────────────────────
        if line == 'Summary:':
            in_summary = True
            continue

        if in_summary:
            if not line:
                in_summary = False
                continue
            m = re.match(r'^(passed|failed|broken|skipped|warnings)\s+(\d+)', line)
            if m:
                key, val = m.group(1), int(m.group(2))
                # Summary values take precedence (authoritative)
                setattr(current, key, getattr(current, key) + val)
            continue

        # ── Line-based counting (before Summary or when no Summary) ──────────
        # Both ANSI-colored: "TPASS: " and plain: ": TPASS: "
        # Use ANSI-stripped line already
        if ': TPASS: ' in line or line.startswith('TPASS:') or 'TPASS  :' in line:
            current.passed += 1
        elif ': TFAIL: ' in line or line.startswith('TFAIL:') or 'TFAIL  :' in line:
            current.failed += 1
        elif ': TBROK: ' in line or line.startswith('TBROK:'):
            current.broken += 1
        elif ': TCONF: ' in line or line.startswith('TCONF:'):
            current.skipped += 1
        elif ': TWARN: ' in line or line.startswith('TWARN:'):
            current.warnings += 1

    return results


def parse_ltp_log_ansi(content: str) -> Dict[str, CaseResult]:
    """
    Parse LTP output that uses ANSI escape codes for TPASS/TFAIL markers.
    This is the format produced when running under a PTY (via script -q).
    Falls back gracefully to plain-text matching.
    """
    results: Dict[str, CaseResult] = {}
    current: Optional[CaseResult] = None

    content = content.replace('\r\n', '\n').replace('\r', '\n')

    # ANSI markers used by LTP
    TPASS_ANSI = '\x1b[1;32mTPASS: \x1b[0m'
    TFAIL_ANSI = '\x1b[1;31mTFAIL: \x1b[0m'
    TBROK_ANSI = '\x1b[1;31mTBROK: \x1b[0m'
    TCONF_ANSI = '\x1b[1;33mTCONF: \x1b[0m'
    TWARN_ANSI = '\x1b[1;35mTWARN: \x1b[0m'

    for raw_line in content.split('\n'):
        plain = strip_ansi(raw_line).strip()

        if plain.startswith('RUN LTP CASE'):
            name = plain.split()[-1]
            current = CaseResult(name=name)
            continue

        if current and plain.startswith(f'FAIL LTP CASE {current.name}'):
            parts = plain.split()
            try:
                rc = int(parts[-1])
                current.return_code = rc
                if rc == 124:
                    current.timed_out = True
            except (ValueError, IndexError):
                pass
            results[current.name] = current
            current = None
            continue

        if current is None:
            continue

        # Count ANSI markers (more reliable than plain text)
        if TPASS_ANSI in raw_line:
            current.passed += 1
        elif TFAIL_ANSI in raw_line:
            current.failed += 1
        elif TBROK_ANSI in raw_line:
            current.broken += 1
        elif TCONF_ANSI in raw_line:
            current.skipped += 1
        elif TWARN_ANSI in raw_line:
            current.warnings += 1
        # Also handle plain-text variants (no ANSI) from piped output
        elif ': TPASS: ' in plain or 'TPASS  :' in plain:
            current.passed += 1
        elif ': TFAIL: ' in plain or 'TFAIL  :' in plain:
            current.failed += 1
        elif ': TBROK: ' in plain:
            current.broken += 1
        elif ': TCONF: ' in plain:
            current.skipped += 1

    return results


def merge(a: Dict[str, CaseResult], b: Dict[str, CaseResult]) -> Dict[str, CaseResult]:
    """Prefer whichever parse found more information."""
    merged = dict(a)
    for k, vb in b.items():
        if k not in merged:
            merged[k] = vb
        else:
            va = merged[k]
            # Use the result with the higher total count
            if vb.total > va.total:
                merged[k] = vb
    return merged


def parse_combined(content: str) -> Dict[str, CaseResult]:
    """Try both parsers and merge results."""
    plain_results = parse_ltp_log(content)
    ansi_results  = parse_ltp_log_ansi(content)
    return merge(plain_results, ansi_results)


def main():
    parser = argparse.ArgumentParser(description='Analyze LTP test results')
    parser.add_argument('log_file', nargs='?', help='Log file (default: stdin)')
    parser.add_argument('--top', type=int, default=0, help='Show only top N tests (by score)')
    parser.add_argument('--csv', action='store_true', help='Output CSV')
    parser.add_argument('--json', action='store_true', dest='json_out', help='Output JSON')
    parser.add_argument('--passed-only', action='store_true', help='Show only tests with passed > 0')
    parser.add_argument('--min-score', type=int, default=0, help='Minimum score threshold')
    args = parser.parse_args()

    if args.log_file:
        with open(args.log_file, 'rb') as f:
            content = f.read().decode('latin-1')
    else:
        content = sys.stdin.buffer.read().decode('latin-1')

    results = parse_combined(content)

    if not results:
        print("No LTP test results found in input.", file=sys.stderr)
        sys.exit(1)

    rows = sorted(results.values(), key=lambda r: (-r.score, r.name))

    if args.passed_only:
        rows = [r for r in rows if r.passed > 0]

    if args.min_score > 0:
        rows = [r for r in rows if r.score >= args.min_score]

    if args.top > 0:
        rows = rows[:args.top]

    # ── Summary stats ─────────────────────────────────────────────────────────
    all_rows = list(results.values())
    n_total        = len(all_rows)
    n_pass         = sum(1 for r in all_rows if r.status == 'PASS')
    n_fail         = sum(1 for r in all_rows if r.status == 'FAIL')
    n_partial      = sum(1 for r in all_rows if r.status == 'PARTIAL')
    n_empty        = sum(1 for r in all_rows if r.status == 'EMPTY')
    n_timeout      = sum(1 for r in all_rows if r.timed_out)
    total_tpass    = sum(r.passed   for r in all_rows)
    total_tfail    = sum(r.failed   for r in all_rows)
    total_tbrok    = sum(r.broken   for r in all_rows)
    total_tconf    = sum(r.skipped  for r in all_rows)

    # ── JSON output ───────────────────────────────────────────────────────────
    if args.json_out:
        out = [{
            'name':    r.name,
            'pass':    r.passed,
            'fail':    r.failed,
            'broken':  r.broken,
            'skipped': r.skipped,
            'all':     r.total,
            'score':   r.score,
            'status':  r.status,
            'timed_out': r.timed_out,
        } for r in rows]
        print(json.dumps(out, indent=2))
        return

    # ── CSV output ────────────────────────────────────────────────────────────
    if args.csv:
        print('name,passed,failed,broken,skipped,total,score,status,timed_out')
        for r in rows:
            print(f'{r.name},{r.passed},{r.failed},{r.broken},{r.skipped},'
                  f'{r.total},{r.score},{r.status},{int(r.timed_out)}')
        return

    # ── Pretty table ──────────────────────────────────────────────────────────
    print()
    print('=' * 80)
    print(f'  LTP Results Summary  ({n_total} tests total)')
    print('=' * 80)
    print(f'  ALL PASS:    {n_pass:4d}   (all subtests passed)')
    print(f'  PARTIAL:     {n_partial:4d}   (some passed, some failed)')
    print(f'  FAIL:        {n_fail:4d}   (all subtests failed or broken)')
    print(f'  EMPTY:       {n_empty:4d}   (no output / skipped)')
    print(f'  TIMEOUT:     {n_timeout:4d}   (killed by timeout)')
    print(f'  Total TPASS: {total_tpass:4d}   subtest assertions passed')
    print(f'  Total TFAIL: {total_tfail:4d}   subtest assertions failed')
    print(f'  Total TBROK: {total_tbrok:4d}   subtest broken')
    print(f'  Total TCONF: {total_tconf:4d}   subtest skipped/not-supported')
    print('=' * 80)

    if rows:
        print()
        print(f'  {"Rank":<5} {"Test Name":<35} {"Score":>6} {"Pass":>5} {"Fail":>5} {"Brok":>5} {"Skip":>5} {"Total":>6} {"Status":<10}')
        print(f'  {"-"*4} {"-"*34} {"-"*6} {"-"*5} {"-"*5} {"-"*5} {"-"*5} {"-"*6} {"-"*9}')
        for i, r in enumerate(rows, 1):
            to_flag = ' [TIMEOUT]' if r.timed_out else ''
            print(f'  {i:<5} {r.name:<35} {r.score:>6} {r.passed:>5} {r.failed:>5} {r.broken:>5} {r.skipped:>5} {r.total:>6}  {r.status:<9}{to_flag}')

    print()

    # High-value targets: tests that fully pass (PASS status) sorted by score
    high_value = [r for r in all_rows if r.status == 'PASS' and r.score > 0]
    high_value.sort(key=lambda r: -r.score)
    if high_value:
        print()
        print(f'  Top HIGH-VALUE tests (fully passing, ranked by score):')
        print(f'  {"Rank":<5} {"Test Name":<35} {"Score":>6}')
        print(f'  {"-"*4} {"-"*34} {"-"*6}')
        for i, r in enumerate(high_value[:50], 1):
            print(f'  {i:<5} {r.name:<35} {r.score:>6}')
        if len(high_value) > 50:
            print(f'  ... and {len(high_value)-50} more fully-passing tests')
        print()


if __name__ == '__main__':
    main()
