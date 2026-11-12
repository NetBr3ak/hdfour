#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
seconds=${1:-1800}
case "$seconds" in
    ''|*[!0-9]*|0) echo 'Usage: sh fuzz/run.sh [positive seconds per target]' >&2; exit 2 ;;
esac
python3 -I fuzz/seed.py
cd fuzz
cargo fuzz build
status=0
for target in open_read_all codec_deflate codec_rle odl; do
    if cargo fuzz run "$target" -- \
        -max_total_time="$seconds" -max_len=65536 -timeout=10 -rss_limit_mb=1024; then
        printf 'PASS %s %s seconds\n' "$target" "$seconds"
    else
        status=1
        printf 'FAIL %s: preserve and minimise the artifact before changing the core.\n' "$target" >&2
    fi
done
exit "$status"
