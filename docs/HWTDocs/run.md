```
ts=$(date '+%m%d_%H%M%S')
SINGLE_TEST=all LOG=OFF OFFLINE=1 CARGO_NET_OFFLINE=true \
bash run.sh -f sdcard-rv.img -t rv \
> "docs/HWTDocs/rvlogs/allrv_full_${ts}.log" \
2> "docs/HWTDocs/rvlogs/allrv_full_${ts}.err"

```

```
SINGLE_TEST=all LTP_START_FROM=waitpid10 LOG=OFF OFFLINE=1 CARGO_NET_OFFLINE=true \
timeout 360s bash run.sh -f sdcard-rv.img -t rv \
> docs/HWTDocs/logs/switch.log
```

```
ts=$(date '+%m%d_%H%M%S')
SINGLE_TEST=all LOG=OFF OFFLINE=1 CARGO_NET_OFFLINE=true \
bash run-la.sh -t la --no-data-disk \
> "docs/HWTDocs/lalogs/la_full_${ts}.log" \
2> "docs/HWTDocs/lalogs/la_full_${ts}.err"
```
