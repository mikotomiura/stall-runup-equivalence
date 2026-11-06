#!/usr/bin/env bash
# 確認（凍結の後に 1 回だけ）: CS1 2019 年版の取り置きの書き手だけの中間を作り、確認の解析を回す。事前登録 v10 の P3・P11（逸脱 1 の照合の直しを含む）。
# 凍結の前には走らせない。2 段に分ける（判断 18 の 3）。1 回だけの保証を手元の git の履歴の外（OSF の時刻）に置くため。
#
#   1 段目: run-confirm-holdout.sh start <OSF の登録の URL> <登録に書いた confirm-manifest.sha256 の SHA-256>
#     照合（manifest の SHA-256 が登録の値と同じ → manifest の全てのファイル → 入力・取り置きの一覧・gse-replay の SHA-256 →
#     Python の版と pip freeze → 作業ツリーがきれい → 完走の記録も結果のファイルのコミットも無い → 前の開始の記録が無い）を
#     通ったら、照合した値と時刻を results_holdout/start.json に書いてコミットし、そこで止まる（計算はしない）。
#     研究者は start.json を OSF の登録に添えて、OSF の時刻を刻む。
#   2 段目: run-confirm-holdout.sh compute <OSF に添えた start.json の URL> <OSF が示すその SHA-256>
#     start.json が HEAD にコミット済みで、その SHA-256 が渡した値と同じこと・同じ開始の記録で計算を始めていないことと、
#     1 段目と同じ照合を確かめ、計算の開始の記録 compute.json をコミットしてから計算する。完走したら結果と来歴をコミットする。
#     研究者は結果（confirm_holdout.json・provenance_holdout.json）も OSF の登録に添える。
#
# 途中で止まったら（2 段目）results_holdout/failed.json を書いてコミットして止める。直した後の再実行は逸脱で、
# CONFIRM_RERUN_REASON に理由を書いて 1 段目からやり直したときだけ通す（前の dir は results_holdout.failed.<時刻> に移して
# コミットする。新しい start.json をまた OSF に添える。結果は照合を通った最初の完走だけを使う）。
# 手順の止め（完走・結果・開始の記録）はこの clone の git の履歴の中だけで効く。外から確かめられる記録は OSF に添えた
# start.json と結果の時刻である（P11）。
set -euo pipefail
STAGE="${1:?段を渡す（start か compute）}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXP_REL="experiments/20261105-findings-paper1-prefreeze"
: "${DATA_DIR:=$HOME/gse-public-data}"
PY="${CONFIRM_PY:-$DATA_DIR/venv-findings1/bin/python}"
CS1="$DATA_DIR/cs1"
K19="$CS1/raw/2019/keystrokes.csv"
BIN="${GSE_REPLAY:-$DATA_DIR/klicke/bin/gse-replay}"
WORK="$CS1/work2019_holdout"
OUT="$EXP_DIR/results_holdout"
OUT_REL="$EXP_REL/results_holdout"
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
die() { echo "$1" >&2; exit 1; }
git_last() { git -C "$REPO_ROOT" log --all --format=%H -n 1 "$@"; }

# 両方の段の照合（計算の前）。引数は登録に書いた manifest の SHA-256
verify() {
  [ "$(sha "$EXP_DIR/confirm-manifest.sha256")" = "$1" ] || die "confirm-manifest.sha256 が登録の値と違う"
  (cd "$REPO_ROOT" && shasum -a 256 -c "$EXP_REL/confirm-manifest.sha256" >/dev/null) || die "manifest のファイルが登録の版と違う"
  [ "$(sha "$K19")" = b54d8cf56b7719ae0ec9ecf19e5b3788168f087bd4a50238dca1e06f6bfe7955 ] || die "2019 年版の SHA-256 が違う"
  [ "$(sha "$CS1/holdout.json")" = a5a32fcefeb97b038afd170e2672262fd5a71230d5e851319eaa076b606bdc47 ] || die "取り置きの一覧が違う"
  [ "$(sha "$BIN")" = 26cae610383fb6de01651cc904895efa72a691c3dd1e27689a14a4ca51c33bb3 ] || die "gse-replay が違う"
  [ "$("$PY" -I -c 'import sys;print(sys.version.split()[0])')" = 3.14.4 ] || die "Python の版が違う"
  [ "$("$PY" -m pip freeze)" = "$(cat "$EXP_DIR/requirements-confirm.txt")" ] || die "環境が requirements-confirm.txt と違う"
  [ -z "$(git -C "$REPO_ROOT" status --porcelain)" ] || die "作業ツリーがきれいでない（コミットしてから走らせる）"
  # パイプにしない（pipefail の下では grep -q が先に抜けると git log が SIGPIPE で終わり、一致があっても偽になる）
  # 完走の記録があれば、理由があっても止める（結果は最初の完走だけ・P11）
  [ -z "$(git_last --grep='^取り置きの確認の完走')" ] || die "完走の記録がある（2 回目は回さない・P11）"
  # 結果のファイルを含むコミットがあれば（中断の件名でも）、それが最初の完走に当たるので止める（P11）
  [ -z "$(git_last -- "$OUT_REL/confirm_holdout.json")" ] || die "結果のファイルがコミットされている（それを最初の完走として扱う・P11）"
}

export OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
unset CONFIRM_SMOKE

if [ "$STAGE" = start ]; then
  OSF_URL="${2:?OSF の登録の URL を渡す（凍結の前に走らせない）}"
  MANIFEST_SHA="${3:?登録に書いた confirm-manifest.sha256 の SHA-256 を渡す}"
  verify "$MANIFEST_SHA"
  if [ -n "$(git_last --grep='^取り置きの確認の開始')" ]; then
    [ -n "${CONFIRM_RERUN_REASON:-}" ] || die "前の開始の記録がある（最初の完走だけを使う・P11。再実行は CONFIRM_RERUN_REASON に理由を書いて逸脱として）"
  fi
  if [ -e "$OUT" ]; then
    [ ! -e "$OUT/confirm_holdout.json" ] || die "取り置きの結果が既にある（最初の完走だけを使う・P11）"
    [ -n "${CONFIRM_RERUN_REASON:-}" ] || die "前の走りの dir がある。直した後の再実行は CONFIRM_RERUN_REASON に理由を書く（逸脱）"
    FAILED_DIR="$OUT.failed.$(date -u +%Y%m%dT%H%M%SZ)"
    mv "$OUT" "$FAILED_DIR"
    git -C "$REPO_ROOT" add -A "$OUT_REL" "$EXP_REL/$(basename "$FAILED_DIR")"
    git -C "$REPO_ROOT" commit -q -m "取り置きの確認の前の走りを退避（$EXP_REL/$(basename "$FAILED_DIR")・理由: ${CONFIRM_RERUN_REASON}）[skip ci]"
  fi
  "$PY" -I "$EXP_DIR/scripts/confirm_p1.py" --selftest
  mkdir -p "$OUT"
  "$PY" -I -c "import json,sys,datetime;json.dump({'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'osf_url':sys.argv[1],'manifest_sha256':sys.argv[2],'git_head':sys.argv[3],'rerun_reason':sys.argv[4] or None},open(sys.argv[5],'w'),indent=1)" \
    "$OSF_URL" "$MANIFEST_SHA" "$(git -C "$REPO_ROOT" rev-parse HEAD)" "${CONFIRM_RERUN_REASON:-}" "$OUT/start.json"
  git -C "$REPO_ROOT" add "$OUT_REL/start.json"
  git -C "$REPO_ROOT" commit -q -m "取り置きの確認の開始（$OUT_REL/start.json）[skip ci]" -- "$OUT_REL/start.json"
  echo "1 段目を終えた（開始の記録をコミットした・計算はしていない）。"
  echo "次: $OUT/start.json（SHA-256 $(sha "$OUT/start.json")）を OSF の登録に添え、OSF に添えたファイルの URL と OSF が示す SHA-256 を渡して 2 段目を回す。"
  exit 0
fi

[ "$STAGE" = compute ] || die "段は start か compute"
OSF_START_URL="${2:?OSF に添えた start.json の URL を渡す}"
OSF_START_SHA="${3:?OSF が示す start.json の SHA-256 を渡す}"
[ -f "$OUT/start.json" ] || die "開始の記録が無い（1 段目を先に回す）"
[ "$(git -C "$REPO_ROOT" show "HEAD:$OUT_REL/start.json" 2>/dev/null | shasum -a 256 | cut -d' ' -f1)" = "$(sha "$OUT/start.json")" ] \
  || die "開始の記録が HEAD にコミットされていない"
[ "$(sha "$OUT/start.json")" = "$OSF_START_SHA" ] || die "start.json の SHA-256 が OSF に添えたものと違う"
[ ! -e "$OUT/compute.json" ] || die "この開始の記録で計算を始めた記録がある（再実行は 1 段目から CONFIRM_RERUN_REASON を付けて）"
[ ! -e "$OUT/failed.json" ] || die "中断の記録がある（再実行は 1 段目から CONFIRM_RERUN_REASON を付けて）"
START_COMMIT="$(git -C "$REPO_ROOT" log --format=%H -n 1 --grep='^取り置きの確認の開始' HEAD)"
[ -n "$START_COMMIT" ] || die "開始のコミットが HEAD の履歴に無い"
# 同じ start.json で計算を始めた記録か中断の記録が、どの枝にもあれば止める（枝を切り替えて入り直す道を塞ぐ）
START_SHA_NOW="$(sha "$OUT/start.json")"
for c in $(git -C "$REPO_ROOT" log --all --format=%H -E --grep='^取り置きの確認の(計算の開始|中断)'); do
  if [ "$(git -C "$REPO_ROOT" show "$c:$OUT_REL/start.json" 2>/dev/null | shasum -a 256 | cut -d' ' -f1)" = "$START_SHA_NOW" ]; then
    die "同じ開始の記録で計算を始めた記録か中断の記録がある（${c}・再実行は 1 段目から CONFIRM_RERUN_REASON を付けて）"
  fi
done
verify "$("$PY" -I -c "import json,sys;print(json.load(open(sys.argv[1]))['manifest_sha256'])" "$OUT/start.json")"

# 計算の開始の記録（コミットしてから計算に入る。確認のスクリプトはこれと start.json が HEAD に無ければ止まる）
"$PY" -I -c "import json,sys,datetime;json.dump({'compute_started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'osf_start_url':sys.argv[1],'osf_start_sha256':sys.argv[2],'git_head':sys.argv[3]},open(sys.argv[4],'w'),indent=1)" \
  "$OSF_START_URL" "$OSF_START_SHA" "$(git -C "$REPO_ROOT" rev-parse HEAD)" "$OUT/compute.json"
git -C "$REPO_ROOT" add "$OUT_REL/compute.json"
git -C "$REPO_ROOT" commit -q -m "取り置きの確認の計算の開始（$OUT_REL/compute.json）[skip ci]" -- "$OUT_REL/compute.json"
STEP=start
# 止まったら、止まった段を failed.json に書いて、そのままコミットする（中断の記録・P11）
record_failure() {
  "$PY" -I -c "import json,sys,datetime;json.dump({'failed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'step':sys.argv[1],'kind':sys.argv[2]},open(sys.argv[3],'w'),indent=1)" "$STEP" "$1" "$OUT/failed.json" || true
  git -C "$REPO_ROOT" add "$OUT_REL" || true
  git -C "$REPO_ROOT" commit -q -m "取り置きの確認の中断（段: ${STEP}・${1}）[skip ci]" -- "$OUT_REL" || echo "中断の記録をコミットできなかった。手で記録してから研究者に聞く" >&2
  echo "途中で止まった（${STEP}）。直さずに記録し、研究者に聞く（P11）" >&2
}
# ERR の罠は未定義の変数（set -u）などで落ちたときに呼ばれないので、EXIT の罠で記録する（2026-11-06 の予行で、
# 完走のコミットの段で落ちて中断の記録が残らなかった）。完走のコミットの後に DONE=1 にする
on_exit() {
  rc=$?
  trap - EXIT INT TERM
  if [ "${DONE:-0}" != 1 ]; then
    # bash 3.2 は未定義の変数で落ちたとき EXIT の罠の中の $? が 0 になる（2026-11-06 に実測）。完走の印が無ければ 1 にする
    [ "$rc" != 0 ] || rc=1
    if [ "$rc" = 130 ]; then record_failure interrupted; else record_failure "error_rc${rc}"; fi
  fi
  exit "$rc"
}
DONE=0  # 環境に DONE があっても完走の印にしない
trap on_exit EXIT
trap 'exit 130' INT TERM

STEP=selftest
"$PY" -I "$EXP_DIR/scripts/confirm_p1.py" --selftest
# 3. 取り置きの書き手だけの中間（見た書き手を読み飛ばす一覧を渡す。ID は repo の外にだけ置く）
STEP=build
SKIP="$CS1/skip_seen_2019.json"
"$PY" -I -c "import csv,json,sys;csv.field_size_limit(10**9);r=csv.reader(open(sys.argv[1],newline=''));i=next(r).index('SubjectID');s={x[i] for x in r};h=set(json.load(open(sys.argv[2]))['2019']);assert h<=s;json.dump({'2019':sorted(s-h)},open(sys.argv[3],'w'))" "$K19" "$CS1/holdout.json" "$SKIP"
rm -rf "$WORK"
"$PY" -I "$REPO_ROOT/experiments/20261104-cs1-landscape/scripts/build_cs1.py" 2019 "$K19" "$BIN" "$SKIP" "$WORK" "${JOBS:-6}"
cp "$WORK/build_summary.json" "$OUT/build_summary_holdout.json"
# 取り置きの書き手の照合（逸脱 1・2026-11-06）: 読んだ書き手は全員取り置き、読まれなかった取り置きの書き手は課題を作る行が 0 件
"$PY" -I "$EXP_DIR/scripts/check_holdout_writers.py" "$K19" "$CS1/holdout.json" "$WORK" "$OUT/holdout_writers_check.json"
# 4. 感度の中間（陽性対照つき）
STEP=build_variants
bash "$EXP_DIR/build-variants.sh" "$WORK" "$PY" "$BIN" "${JOBS:-6}"
cp "${WORK}_control_summary.json" "$OUT/rebuild_control_summary_holdout.json"
for v in q995 w15 w60; do cp "${WORK}_$v/rebuild_summary.json" "$OUT/rebuild_summary_${v}_holdout.json"; done
# 5. 確認の解析
STEP=confirm
"$PY" -I "$EXP_DIR/scripts/confirm_p1.py" holdout "$WORK" "$CS1/holdout.json" 20261106 "$OUT"
STEP=provenance
HEAD_SHA="$(git -C "$REPO_ROOT" rev-parse HEAD)"
"$PY" -I -c "import json,sys,datetime,hashlib;h=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest();json.dump({'git_head':sys.argv[1],'finished_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'confirm_holdout_sha256':h(sys.argv[2]),'manifest_sha256':h(sys.argv[3]),'start_sha256':h(sys.argv[4]),'osf_start_url':sys.argv[5]},open(sys.argv[6],'w'),indent=1)" \
  "$HEAD_SHA" "$OUT/confirm_holdout.json" "$EXP_DIR/confirm-manifest.sha256" "$OUT/start.json" "$OSF_START_URL" "$OUT/provenance_holdout.json"
# 完走の記録（手順がコミットする。結果は集計値だけで、書き手ごとの値を含まない）
STEP=commit
git -C "$REPO_ROOT" add "$OUT_REL"
git -C "$REPO_ROOT" commit -q -m "取り置きの確認の完走（${OUT_REL}）[skip ci]" -- "$OUT_REL"
DONE=1
echo "完走した。results_holdout/ をコミットした。"
echo "次: confirm_holdout.json と provenance_holdout.json を OSF の登録に添える（P11）。"
