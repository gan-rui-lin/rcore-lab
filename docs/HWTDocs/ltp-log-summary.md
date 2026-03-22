# LTP 日志统计脚本

用于把串口日志里的 LTP 输出统计成两套指标：

- `clean pass cases`：按“一个 LTP case 是否干净通过”统计
- `judge-style score`：按所有 `TPASS` 条数累计，接近评测器风格

## 用法

```bash
python3 tools/ltp_log_summary.py /tmp/rcore-ltp.log
```

输出会包含：

- `Total cases`
- `Clean pass cases`
- `Bad cases`
- `Skip-only cases`
- `No-result cases`
- `Judge-style score`
- `Total TPASS/TFAIL/TBROK/TCONF/TWARN`
- `Top bad cases`

如果想拿机器可读结果：

```bash
python3 tools/ltp_log_summary.py /tmp/rcore-ltp.log --json --show-cases > /tmp/ltp-summary.json
```

## 判定口径

- `clean_pass`
  - 该 case 至少出现 1 次 `TPASS`
  - 且没有 `TFAIL` / `TBROK`
- `bad`
  - 出现任意 `TFAIL` 或 `TBROK`
- `skip_only`
  - 没有 `TPASS/TFAIL/TBROK`
  - 但出现了 `TCONF`
- `warn_only`
  - 只有 `TWARN`
- `no_result`
  - 没有明确结果正文

脚本会优先按每条结果正文里的 `TPASS/TFAIL/TBROK/TCONF/TWARN` 统计；如果某个 case 完全没有这些正文，再回退到 `Summary:` 段里的 `passed/failed/broken/skipped/warnings`。
