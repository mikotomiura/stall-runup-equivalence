#!/usr/bin/env bash
# Drop-in stand-in for gse-replay used by the equivalence gate (see ../README.md, "Equivalence gate").
# Runs the reference binary ($GATE_OLD) and this crate's binary ($GATE_NEW) with the same arguments
# and stdin, appends one line to $GATE_LOG, and hands the reference's stdout, stderr and exit code
# back to the caller unchanged, so the calling pipeline behaves exactly as it did with the reference.
# Log line: <stdout match|MISMATCH> <exit match|MISMATCH> <stderr match|MISMATCH> <mode> <exit code> <stdout sha256>
#   followed, on a mismatch, by new=<this crate's exit code> and the input's sha256 (to rerun that input alone).
#   mode is the flags only (the input path is not written to the log).
set -u
: "${GATE_OLD:?reference gse-replay}" "${GATE_NEW:?new gse-replay}" "${GATE_LOG:?log file}"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/gate.XXXXXX")" || exit 97
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/in"
"$GATE_OLD" "$@" < "$tmp/in" > "$tmp/o.out" 2> "$tmp/o.err"; eo=$?
"$GATE_NEW" "$@" < "$tmp/in" > "$tmp/n.out" 2> "$tmp/n.err"; en=$?
mode="${*:2}"  # every caller passes the input path first; the flags and their values follow
[ -z "$mode" ] && mode="(no-flag)"
s1=match; cmp -s "$tmp/o.out" "$tmp/n.out" || s1=MISMATCH
s2=match; [ "$eo" = "$en" ] || s2=MISMATCH
s3=match; cmp -s "$tmp/o.err" "$tmp/n.err" || s3=MISMATCH
h="$(shasum -a 256 "$tmp/o.out" | cut -d' ' -f1)"
extra=""
if [ "$s1$s2$s3" != matchmatchmatch ]; then
  extra=" new=$en in=$(shasum -a 256 "$tmp/in" | cut -d' ' -f1)"
  # for --features and no-flag calls the input is the path argument, not stdin
  if [ -f "${1:-}" ]; then
    a="$(shasum -a 256 "$1" | cut -d' ' -f1)"; extra="$extra arg=$a"
    # keep the input next to the log (local only, outside the repo) so the call can be rerun alone
    mkdir -p "$GATE_LOG.inputs" && cp "$1" "$GATE_LOG.inputs/$a"
  fi
fi
# a lost log line would hide a mismatch, so a failed write fails the call (and with it the pipeline)
printf '%s %s %s [%s] %s %s%s\n' "$s1" "$s2" "$s3" "$mode" "$eo" "$h" "$extra" >> "$GATE_LOG" \
  || { echo "compare-replay: cannot write $GATE_LOG" >&2; exit 98; }
cat "$tmp/o.err" >&2
cat "$tmp/o.out"
exit "$eo"
