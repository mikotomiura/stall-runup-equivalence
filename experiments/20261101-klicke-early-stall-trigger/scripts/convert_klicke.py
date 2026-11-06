"""KLiCKe の記録の CSV 1 本を、本文を含まない GSE の NDJSON に変換する（conditions.md K3）。

- 読むのは DownTime・UpTime・DownEvent の 3 列だけ（TextChange・Activity・CursorPosition は読まない）。
- 1 行 = 1 回の押下。押下と離しの実時刻をそのまま key 行にする（合成しない）。
- マウスの出来事（DownEvent の名前に "click" を含む）は key 行にしない。
- 出力の key 行は時刻・category・press だけ。キーの名前そのものは書き出さない。

使い方（単体）: python3 -I convert_klicke.py <WritingTask.zip> <member> <out.ndjson>
"""
import csv
import io
import json
import re
import sys
import zipfile

HMM_VERSION = "0.5.0"  # meta 行の値（replay は刻印に使うだけ）
T_BASE = 1_700_000_000_000  # 全セッションに同じ定数を足す（K3）
EXPECTED_HEADER = ["", "DownEventID", "UpEventID", "DownTime", "UpTime", "ActionTime", "DownEvent", "UpEvent",
                   "CursorPosition", "PauseTime", "WordCount", "TextChange", "Activity"]
ESCAPE = re.compile(r"\\([\\\"nt])")
UNESCAPE = {"\\": "\\", '"': '"', "n": "\n", "t": "\t"}
SENTENCE_PUNCT = set(".,")
MODIFIERS = {"Shift", "Control", "Alt", "AltGraph", "Meta", "CapsLock", "OS", "Fn", "NumLock", "ScrollLock"}
NAVIGATION = {"ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"}
FUNCTION = {"F%d" % i for i in range(1, 25)}
NAMED = {"Space": "space", "Enter": "enter", "Tab": "tab", "Backspace": "backspace", "Delete": "delete",
         "Escape": "escape"}

csv.field_size_limit(16 << 20)


class LogError(Exception):
    """理由の符号だけを持つ（行の中身を持たない）。"""


def unescape(s):
    return ESCAPE.sub(lambda m: UNESCAPE[m.group(1)], s) if "\\" in s else s


def decode(b):
    try:
        return b.decode("utf-8-sig"), "utf-8"
    except UnicodeDecodeError:
        return b.decode("cp1252", errors="replace"), "cp1252"


def is_mouse(name):
    return "click" in name.lower()


def category(name):
    """DownEvent の値 → GSE の category（K3 の対応表・件数を見る前に固定）。"""
    if name in NAMED:
        return NAMED[name]
    if name in MODIFIERS:
        return "modifier"
    if name in NAVIGATION:
        return "navigation"
    if name in FUNCTION:
        return "function"
    if len(name) == 1:
        c = name
        if c.isascii() and c.isalpha():
            return "alphabetic"
        if c.isascii() and c.isdigit():
            return "numeric"
        if c == " ":
            return "space"
        if c in SENTENCE_PUNCT:
            return "sentence_punctuation"
        if c.isascii() and c.isprintable():
            return "punctuation"
    return "other"


def read_events(raw_bytes):
    """[(down_ms, up_ms, down_event)] と符号化の名前。読めなければ LogError。"""
    s, enc = decode(raw_bytes)
    reader = csv.reader(io.StringIO(s, newline=""), strict=True)
    try:
        header = next(reader, None)
        if header != EXPECTED_HEADER:
            raise LogError("header differs")
        out = []
        for rec in reader:
            if len(rec) != len(header):
                raise LogError("field count differs")
            try:
                down = float(rec[3])
                up = float(rec[4])
            except ValueError:
                raise LogError("unparsable number") from None
            out.append((down, up, unescape(rec[6])))
    except csv.Error:
        raise LogError("csv format") from None
    return out, enc


def to_keys(events):
    """マウスを除いた押下の列 [(t_down, t_up, category)]（ms・整数・T_BASE を足す）と除いた数。"""
    keys, mouse = [], 0
    for down, up, name in events:
        if is_mouse(name):
            mouse += 1
            continue
        td = T_BASE + int(round(down))
        tu = max(td, T_BASE + int(round(up)))
        keys.append((td, tu, category(name)))
    keys.sort(key=lambda k: (k[0], k[1]))
    return keys, mouse


def write_ndjson(keys, dst):
    rows = []
    for td, tu, cat in keys:
        rows.append((td, 0, {"type": "key", "t": td, "category": cat, "press": True}))
        rows.append((tu, 1, {"type": "key", "t": tu, "category": cat, "press": False}))
    rows.sort(key=lambda r: (r[0], r[1]))
    t0 = keys[0][0]
    with open(dst, "w", encoding="utf-8") as f:
        f.write(json.dumps({"type": "meta", "session_start": t0, "hmm_version": HMM_VERSION}) + "\n")
        f.write(json.dumps({"type": "session_start", "t": t0}) + "\n")
        for _, _, r in rows:
            f.write(json.dumps(r) + "\n")


if __name__ == "__main__":
    with zipfile.ZipFile(sys.argv[1]) as z:
        ev, _ = read_events(z.read(sys.argv[2]))
    k, m = to_keys(ev)
    write_ndjson(k, sys.argv[3])
    print(json.dumps({"keys": len(k), "mouse": m}))
