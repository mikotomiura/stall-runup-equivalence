#!/usr/bin/env bash
# 変異（conditions.md の Q7）: 名指しの対照が赤になることを確かめる。赤（exit 3）が期待どおり。
set -uo pipefail
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${DATA_DIR:=$HOME/gse-public-data}"
NEW="$DATA_DIR/landscape/coauthor_work_obs"
SC="$EXP_DIR/scripts"; OUT="${OUT:-$EXP_DIR/results}/mutations"; SEED="$(cat "$EXP_DIR/SEED")"
mkdir -p "$OUT"
run() { python3 -I "$SC/perperson.py" "$@"; echo "exit=$?"; }
run --selftest "$NEW" "$SEED" "$OUT/m1.json" --mutate m1 --only P-EM4
run --selftest "$NEW" "$SEED" "$OUT/m2.json" --mutate m2 --only P-EM3
run --pd2 "$NEW" "$OUT/m3_pd2.json" --mutate m3
run --selftest "$NEW" "$SEED" "$OUT/m3_em1.json" --mutate m3 --only P-EM1
run --selftest "$NEW" "$SEED" "$OUT/m4.json" --mutate m4 --only P-L2
run --selftest "$NEW" "$SEED" "$OUT/m5.json" --mutate m5 --only P-L2
run --selftest "$NEW" "$SEED" "$OUT/m6.json" --mutate m6 --only P-S
