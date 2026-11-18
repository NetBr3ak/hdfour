#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
store=${HDFOUR_FIXTURE_STORE:-"$HOME/.local/share/hdfour-fixtures"}
manager="$store/micromamba"
prefix="$store/env"
export MAMBA_ROOT_PREFIX="$store/mamba"
mkdir -p "$store" fixtures/.build
if [ ! -x "$manager" ]; then
    curl -fsSL https://github.com/mamba-org/micromamba-releases/releases/download/2.10.0-0/micromamba-linux-64 -o "$manager.download"
    printf '%s  %s\n' 701f39c06f8e6a670a0e186c8479b9e5c0502a2f0cda6cef65d83bdd2875e8bd "$manager.download" | sha256sum -c -
    chmod +x "$manager.download"
    mv "$manager.download" "$manager"
fi
printf '%s  %s\n' 701f39c06f8e6a670a0e186c8479b9e5c0502a2f0cda6cef65d83bdd2875e8bd "$manager" | sha256sum -c -
if [ ! -x "$prefix/bin/python" ]; then
    "$manager" create -y -p "$prefix" -f fixtures/conda-lock.txt
fi
"$manager" list -p "$prefix" --explicit --sha256 > fixtures/.build/installed.txt
sed -n '/^https:/p' fixtures/conda-lock.txt > fixtures/.build/locked-packages.txt
sed -n '/^https:/p' fixtures/.build/installed.txt > fixtures/.build/installed-packages.txt
cmp fixtures/.build/locked-packages.txt fixtures/.build/installed-packages.txt
export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
for program in gen sdsdump; do
    gcc -std=c11 -O2 -Wall -Wextra -Werror -I"$prefix/include" \
        "fixtures/c/$program.c" -L"$prefix/lib" -Wl,-rpath,"$prefix/lib" \
        -lhdfeos -lGctp -lmfhdf -ldf -lz -ljpeg -lm -o "fixtures/.build/$program"
done
(cd fixtures && .build/gen)
"$prefix/bin/hdp" dumpsds -h fixtures/F05.hdf > fixtures/F05-hdp.txt
"$prefix/bin/python" -I fixtures/dump.py
"$prefix/bin/python" -I fixtures/dump.py --check
"$prefix/bin/python" -I fixtures/window_reference.py
"$prefix/bin/python" -I fixtures/test_fixtures.py
find fixtures corpus -type f \( -name 'F??.hdf' -o -name 'F12.raw' -o -name 'F05-hdp.txt' -o -path 'fixtures/golden/*' -o -path 'corpus/R0[23].hdf' \) -print \
    | LC_ALL=C sort | xargs sha256sum > fixtures/SHA256SUMS
printf 'PASS fixture build and golden check\n'
