#!/usr/bin/env python3

from __future__ import annotations

import argparse
import re
from collections import defaultdict
from pathlib import Path


NUMERIC_SUFFIXES = [".c", ".sh", ".py", ".run-test", ".pl", ".txt", ".json", ".ksh"]
NAME_FALLBACK_SUFFIXES = ["_64", "_16", "A", "B"]


def to_int(value: str) -> int:
    value = value.strip()
    return int(value) if value not in ("", "-") else 0


def strip_tags(value: str) -> str:
    return re.sub(r"<.*?>", "", value).strip()


def parse_suite(html: str, suite_name: str) -> dict[str, dict[str, int]]:
    table_match = re.search(
        rf"<h3>{re.escape(suite_name)}</h3>\s*<table>(.*?)</table>",
        html,
        re.S,
    )
    if not table_match:
        raise ValueError(f"missing suite table: {suite_name}")

    suite = {}
    for row_html in re.findall(r"<tr>(.*?)</tr>", table_match.group(1), re.S):
        cols = [
            strip_tags(cell)
            for cell in re.findall(r"<t[dh][^>]*>(.*?)</t[dh]>", row_html, re.S)
        ]
        if len(cols) != 8 or cols[0] in ("测试点", "总分"):
            continue

        numbers = [to_int(value) for value in cols[1:]]
        suite[cols[0]] = {
            "rv_pass": numbers[0],
            "rv_all": numbers[1],
            "rv_score": numbers[2],
            "la_pass": numbers[3],
            "la_all": numbers[4],
            "la_score": numbers[5],
            "total": numbers[6],
            "pass_sum": numbers[0] + numbers[3],
            "all_sum": numbers[1] + numbers[4],
        }
    return suite


def build_case_index(ltp_root: Path) -> dict[str, list[str]]:
    index: dict[str, list[str]] = defaultdict(list)
    for path in ltp_root.rglob("*"):
        if not path.is_file():
            continue
        stem = path.name
        for suffix in NUMERIC_SUFFIXES:
            if stem.endswith(suffix):
                stem = stem[: -len(suffix)]
                break
        index[stem].append(path.relative_to(ltp_root).as_posix())
    return index


def resolve_case_path(case_name: str, case_index: dict[str, list[str]]) -> str:
    candidates = [case_name]
    for suffix in NAME_FALLBACK_SUFFIXES:
        if case_name.endswith(suffix):
            candidates.append(case_name[: -len(suffix)])

    for candidate in candidates:
        paths = case_index.get(candidate)
        if paths:
            return paths[0]
    return ""


def classify_direction(case_path: str) -> str:
    if "/ipc/msg" in case_path:
        return "SysV IPC: 消息队列"
    if "/ipc/shm" in case_path:
        return "SysV IPC: 共享内存"
    if any(token in case_path for token in ("/waitpid/", "/waitid/", "/wait4/", "/wait/", "/times/")):
        return "进程等待/回收/时间统计"
    if any(
        token in case_path
        for token in (
            "/access/",
            "/stat/",
            "/open",
            "/chmod/",
            "/chown/",
            "/creat/",
            "/readlink/",
            "/link/",
            "/mkdir/",
            "/pathconf/",
            "/realpath/",
            "/unlink",
            "/faccessat",
            "/fchmod",
            "/fchown",
            "/getdents",
            "/chroot/",
            "/utime/",
            "/utimes/",
        )
    ):
        return "文件权限/路径语义"
    if any(
        token in case_path
        for token in (
            "/setregid/",
            "/setreuid/",
            "/setresuid/",
            "/setresgid/",
            "/setfsuid/",
            "/setfsgid/",
            "/setgid/",
            "/setuid/",
            "/getgid/",
            "/getuid/",
            "/getegid/",
            "/geteuid/",
            "/setegid/",
        )
    ):
        return "凭证/身份切换"
    if any(
        token in case_path
        for token in (
            "/adjtimex/",
            "/clock_",
            "/sched_",
            "/setpriority/",
            "/getpriority/",
            "/gettid/",
            "/stime",
            "/timerfd/",
            "/settimeofday/",
            "/exit_group/",
            "/time/",
        )
    ):
        return "调度/时钟/优先级"
    if any(token in case_path for token in ("/bind/", "/accept/", "/socket/", "/connect/", "/select/")):
        return "网络基础能力"
    if case_path.startswith("network/"):
        return "网络基础能力"
    if any(token in case_path for token in ("/sig", "/signal", "/kill/", "/tkill/")):
        return "信号处理"
    if any(token in case_path for token in ("/mmap", "/munmap", "/mlock", "/writev/", "/truncate/", "/write/", "/tee/")):
        return "内存映射/IO 边角语义"
    if any(token in case_path for token in ("/uname/", "/utsname/", "/unshare/")):
        return "系统信息/容器边界"
    return "其他/待细分"


def aggregate_diffs(
    glibc_suite: dict[str, dict[str, int]],
    musl_suite: dict[str, dict[str, int]],
    case_index: dict[str, list[str]],
) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    rows = []
    zero = {"pass_sum": 0, "all_sum": 0}

    for case_name in sorted(set(glibc_suite) | set(musl_suite)):
        glibc_row = glibc_suite.get(case_name, zero)
        musl_row = musl_suite.get(case_name, zero)
        pass_diff = musl_row["pass_sum"] - glibc_row["pass_sum"]
        all_diff = musl_row["all_sum"] - glibc_row["all_sum"]
        if pass_diff <= 0 and all_diff <= 0:
            continue

        case_path = resolve_case_path(case_name, case_index)
        rows.append(
            {
                "case": case_name,
                "path": case_path,
                "direction": classify_direction(case_path),
                "kind": "shared" if case_name in glibc_suite else "musl-only",
                "glibc_pass": glibc_row["pass_sum"],
                "glibc_all": glibc_row["all_sum"],
                "musl_pass": musl_row["pass_sum"],
                "musl_all": musl_row["all_sum"],
                "pass_diff": pass_diff,
                "all_diff": all_diff,
            }
        )

    directions = defaultdict(
        lambda: {
            "pass_diff": 0,
            "all_diff": 0,
            "case_count": 0,
            "shared_count": 0,
            "musl_only_count": 0,
            "cases": [],
        }
    )
    for row in rows:
        bucket = directions[row["direction"]]
        bucket["pass_diff"] += row["pass_diff"]
        bucket["all_diff"] += row["all_diff"]
        bucket["case_count"] += 1
        bucket["shared_count"] += int(row["kind"] == "shared")
        bucket["musl_only_count"] += int(row["kind"] == "musl-only")
        bucket["cases"].append(row)

    ranked_directions = sorted(
        (
            {"direction": name, **payload}
            for name, payload in directions.items()
        ),
        key=lambda item: (item["pass_diff"], item["all_diff"]),
        reverse=True,
    )
    ranked_rows = sorted(
        rows,
        key=lambda item: (item["pass_diff"], item["all_diff"], item["case"]),
        reverse=True,
    )
    return ranked_directions, ranked_rows


def top_cases_text(rows: list[dict[str, object]], limit: int) -> str:
    lines = []
    for index, row in enumerate(rows[:limit], start=1):
        lines.append(
            f"{index:>2}. {row['case']:<20} | pass {format_delta(row['pass_diff']):<4} | all {format_delta(row['all_diff']):<4} | {row['kind']:<9} | {row['path'] or 'unmapped'}"
        )
    return "\n".join(lines)


def direction_case_text(direction_rows: list[dict[str, object]], limit: int) -> str:
    lines = []
    for row in direction_rows[:limit]:
        lines.append(
            f"  - {row['case']}: pass {format_delta(row['pass_diff'])}, all {format_delta(row['all_diff'])} ({row['kind']}, {row['path'] or 'unmapped'})"
        )
    return "\n".join(lines)


def format_delta(value: int) -> str:
    return f"{value:+d}"


def build_report(
    html_path: Path,
    ltp_root: Path,
    glibc_suite: dict[str, dict[str, int]],
    musl_suite: dict[str, dict[str, int]],
    ranked_directions: list[dict[str, object]],
    ranked_rows: list[dict[str, object]],
) -> str:
    glibc_pass = sum(row["pass_sum"] for row in glibc_suite.values())
    glibc_all = sum(row["all_sum"] for row in glibc_suite.values())
    musl_pass = sum(row["pass_sum"] for row in musl_suite.values())
    musl_all = sum(row["all_sum"] for row in musl_suite.values())

    shared = set(glibc_suite) & set(musl_suite)
    musl_only = set(musl_suite) - set(glibc_suite)

    shared_rows = [row for row in ranked_rows if row["kind"] == "shared"]
    musl_only_rows = [row for row in ranked_rows if row["kind"] == "musl-only"]

    lines = []
    lines.append("========================================================================")
    lines.append("LTP glibc vs musl：基于 htmls-9000/开放课程.html 的 pass/all 差分分析")
    lines.append("========================================================================")
    lines.append("")
    lines.append("一、原始统计")
    lines.append("------------------------------------------------------------------------")
    lines.append(f"- HTML 来源: {html_path}")
    lines.append(f"- LTP 目录: {ltp_root}")
    lines.append(f"- ltp-glibc: {len(glibc_suite)} 个测试点, pass={glibc_pass}, all={glibc_all}")
    lines.append(f"- ltp-musl : {len(musl_suite)} 个测试点, pass={musl_pass}, all={musl_all}")
    lines.append(f"- 总差值   : pass +{musl_pass - glibc_pass}, all +{musl_all - glibc_all}")
    lines.append(f"- 共享测试点: {len(shared)}")
    lines.append(f"- musl 独有 : {len(musl_only)}")
    lines.append("")
    lines.append("二、先看结论")
    lines.append("------------------------------------------------------------------------")
    lines.append("1. 如果只看“共享测试里 glibc 落后 musl 的部分”，最值得优先修的是：")
    lines.append("   文件权限/路径语义 -> SysV IPC 消息队列 -> 凭证/身份切换 -> 调度/时钟/优先级。")
    lines.append("2. 如果把“musl 已跑到、glibc 还没进入表”的新增空间也算上，最大增量反而来自：")
    lines.append("   进程等待/回收/时间统计（waitpid/waitid/times）和文件权限/路径语义。")
    lines.append("3. `waitpid01` 单项就有 pass +292 / all +292，是当前页面里最大的单点差值；")
    lines.append("   但它属于 musl-only，需要先确认是 glibc 侧没有编进来、没有跑到，还是被运行环境屏蔽。")
    lines.append("4. `access01` 仍然是“共享测试中的头号目标”，因为它不是覆盖问题，而是 glibc 已在表中但明显落后。")
    lines.append("")
    lines.append("三、按改动方向排序（综合 shared + musl-only）")
    lines.append("------------------------------------------------------------------------")
    for idx, direction in enumerate(ranked_directions, start=1):
        lines.append(
            f"{idx:>2}. {direction['direction']}: pass +{direction['pass_diff']}, all +{direction['all_diff']}, "
            f"{direction['case_count']} 个测试点 (shared {direction['shared_count']}, musl-only {direction['musl_only_count']})"
        )
        direction_rows = sorted(
            direction["cases"],
            key=lambda item: (item["pass_diff"], item["all_diff"], item["case"]),
            reverse=True,
        )
        lines.append(direction_case_text(direction_rows, limit=6))
    lines.append("")
    lines.append("四、共享测试中的高价值方向（更像“直接补语义拿分”）")
    lines.append("------------------------------------------------------------------------")
    shared_direction_order = sorted(
        (
            direction
            for direction in ranked_directions
            if direction["shared_count"] > 0
        ),
        key=lambda item: (
            sum(case["pass_diff"] for case in item["cases"] if case["kind"] == "shared"),
            sum(case["all_diff"] for case in item["cases"] if case["kind"] == "shared"),
        ),
        reverse=True,
    )
    for idx, direction in enumerate(shared_direction_order[:6], start=1):
        shared_cases = [case for case in direction["cases"] if case["kind"] == "shared"]
        pass_diff = sum(case["pass_diff"] for case in shared_cases)
        all_diff = sum(case["all_diff"] for case in shared_cases)
        lines.append(
            f"{idx:>2}. {direction['direction']}: pass +{pass_diff}, all +{all_diff}, {len(shared_cases)} 个共享测试点"
        )
        lines.append(direction_case_text(sorted(shared_cases, key=lambda item: (item["pass_diff"], item["all_diff"], item["case"]), reverse=True), limit=5))
    lines.append("")
    lines.append("五、musl-only 的高价值方向（更像“补覆盖/补启用”）")
    lines.append("------------------------------------------------------------------------")
    musl_only_direction_order = sorted(
        (
            direction
            for direction in ranked_directions
            if direction["musl_only_count"] > 0
        ),
        key=lambda item: (
            sum(case["pass_diff"] for case in item["cases"] if case["kind"] == "musl-only"),
            sum(case["all_diff"] for case in item["cases"] if case["kind"] == "musl-only"),
        ),
        reverse=True,
    )
    for idx, direction in enumerate(musl_only_direction_order[:6], start=1):
        musl_only_cases = [case for case in direction["cases"] if case["kind"] == "musl-only"]
        pass_diff = sum(case["pass_diff"] for case in musl_only_cases)
        all_diff = sum(case["all_diff"] for case in musl_only_cases)
        lines.append(
            f"{idx:>2}. {direction['direction']}: pass +{pass_diff}, all +{all_diff}, {len(musl_only_cases)} 个 musl-only 测试点"
        )
        lines.append(direction_case_text(sorted(musl_only_cases, key=lambda item: (item["pass_diff"], item["all_diff"], item["case"]), reverse=True), limit=5))
    lines.append("")
    lines.append("六、全局 Top 20 单项差值")
    lines.append("------------------------------------------------------------------------")
    lines.append(top_cases_text(ranked_rows, limit=20))
    lines.append("")
    lines.append("七、建议的优先级")
    lines.append("------------------------------------------------------------------------")
    lines.append("A. 想要“更稳地回收 glibc 已经在跑的分数”：")
    lines.append("   先修 `access/stat/open/chmod/chown` 这一组文件权限语义，再修 `msg*`，再修 `setreuid/setregid/setresuid`。")
    lines.append("B. 想要“冲更大分差，但要先排查测试覆盖链路”：")
    lines.append("   先核查 `waitpid01 + waitid* + times03` 为什么只在 musl 表里出现，再决定是补 glibc 构建、运行脚本，还是补内核语义。")
    lines.append("C. `all` 差值明显大于 `pass` 的点需要额外警惕：")
    lines.append("   例如 `shmctl02`、`truncate03`、`times03`，这类通常意味着 musl 跑到了更多子项，而 glibc 侧不是单纯“少过了几个”。")
    lines.append("")
    lines.append("八、使用说明")
    lines.append("------------------------------------------------------------------------")
    lines.append(
        "重新生成命令: `python3 scripts/analyze_ltp_html.py --html htmls-9000/开放课程.html --ltp-root /home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524/testcases --output ltp_glibc_musl_comparison.txt`"
    )
    lines.append("")
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description="Analyze LTP glibc vs musl pass/all gaps from a saved HTML page.")
    parser.add_argument("--html", required=True, type=Path, help="Saved HTML page containing ltp-glibc and ltp-musl tables.")
    parser.add_argument("--ltp-root", required=True, type=Path, help="LTP testcases root used to map cases back to directories.")
    parser.add_argument("--output", type=Path, help="Optional output file. Defaults to stdout.")
    args = parser.parse_args()

    html = args.html.read_text(encoding="utf-8", errors="ignore")
    glibc_suite = parse_suite(html, "ltp-glibc")
    musl_suite = parse_suite(html, "ltp-musl")
    case_index = build_case_index(args.ltp_root)
    ranked_directions, ranked_rows = aggregate_diffs(glibc_suite, musl_suite, case_index)
    report = build_report(
        html_path=args.html,
        ltp_root=args.ltp_root,
        glibc_suite=glibc_suite,
        musl_suite=musl_suite,
        ranked_directions=ranked_directions,
        ranked_rows=ranked_rows,
    )

    if args.output:
        args.output.write_text(report, encoding="utf-8")
    else:
        print(report)


if __name__ == "__main__":
    main()
