#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
case "$(uname -m)" in
    x86_64) ;;
    *) echo 'This command inspects the x86-64 build only.' >&2; exit 2 ;;
esac
cargo rustc -p hdfour --release --lib -- --emit=asm
assembly=$(find target/release/deps -name 'hdfour-*.s' -type f)
if [ -z "$assembly" ]; then
    echo 'No assembly output found.' >&2
    exit 1
fi
awk '
    /^_.*swap_in_place.*:$/ { printing = 1; found = 1 }
    printing { print }
    printing && /^\.Lfunc_end/ { printing = 0 }
    END { if (!found) exit 1 }
' $assembly
