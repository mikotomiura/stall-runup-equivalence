//! transfer-pipeline: raw NDJSON を被験者端末で匿名化する純粋変換 (age 暗号化前段)。
//!
//! data-privacy `anonymization.md` 準拠:
//! - **VK コード → `KeyCategory`** (どの文字かを失い、種類のみ残す → 本文再構成不能)。
//! - **絶対 epoch 時刻 → セッション開始からの相対 ms** (他ログとの突合による再識別を防ぐ)。
//! - ウィンドウタイトル/プロセス名/本文は元 NDJSON に存在しない (logger 設計) ため追加除去は不要。
//!
//! 本モジュールは `age` 非依存の純粋関数 (lib に常駐し常にテストされる)。暗号化は bin `gse-package`。

use serde_json::Value;

/// VK コードをプライバシーセーフなキー種別カテゴリに変換する (anonymization.md 準拠)。
/// 「どのキーか」を失い「どんな種類か」だけ残す。特徴量再計算に必要な
/// backspace/delete の区別は保持する (F3 修正率・F6 削除後停止率)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCategory {
    Alphabetic,
    Numeric,
    /// OEM 句読点・記号とテンキー演算子 (`; = , - . / ` [ \ ] '` と `* + - . / separator`)。
    ///
    /// **なぜ独立の category か (ADR-014 判断 6 / D-7)**: これらは raw では
    /// `is_text_producing` が true だが、旧版は `Other` へ畳んでいたため匿名化後に
    /// text-producing と判別できず、`manual_insert` 系がその件数だけ過小評価されていた。
    /// `manual_contribution_ratio` は `research-positioning.md §5` **H-Y の主要アウトカム**である。
    ///
    /// **本文再構成不能は保たれる**: **17 個の異なる VK** が 1 つの category へ畳まれるため、
    /// `[` と `=` と `;` を区別できない (どの記号かは残さない)。⚠️ この畳み込み数が小さくなると
    /// 識別性が上がる (`punctuation_folds_many_distinct_keys` が回帰ロックしている)。
    Punctuation,
    /// 文/節の境界を担う句読点キー — `VK_OEM_PERIOD`(0xBE) と `VK_OEM_COMMA`(0xBC)。
    ///
    /// **なぜ `Punctuation` から分けるか (2026-08-23 研究者判断)**:
    /// `personalization-and-online-learning.md §5.2` は **boundary keys を「採用 (弁別の前提条件)」**
    /// に格上げしており ([P121] N=475・2 独立文脈)、その接地は散文の階層
    /// **語内 < 語間 < 文間 < 段落間** である。`Punctuation` へ一括で畳むと **「文間」の段だけが
    /// 失われる** (`[` `=` `-` と同じラベルになるため)。⚠️ **収集後は取り返せない。**
    ///
    /// ⚠️ **完全な文末判定ではない**: 修飾キー状態を記録していないので `.` と `>`、`,` と `<` は
    /// 区別できない。★ 0xBE は文末の**代理**である (日本語ローマ字入力では `。` が 0xBE から出るので
    /// 代理としては強い)。
    ///
    /// ⚠️ **識別性**: 2 VK のカテゴリは文のリズムを見せる。★ ただし**どの文字かは残らない**ので
    /// 本文再構成は不能であり、文の区切りは `Space` / `Enter` からも既に推定できる。
    SentencePunctuation,
    /// Tab。`is_text_producing` が true なので `Other` に畳まない (同上)。
    Tab,
    Backspace,
    Delete,
    Space,
    Enter,
    Navigation,
    Modifier,
    Function,
    Escape,
    Other,
}

impl KeyCategory {
    pub fn from_vk(vk: u32) -> Self {
        match vk {
            0x41..=0x5A => Self::Alphabetic,
            0x30..=0x39 | 0x60..=0x69 => Self::Numeric,
            // 文/節の境界キー。⚠️ Punctuation より **先に** 判定する (レンジが重なるため)。
            0xBC | 0xBE => Self::SentencePunctuation,
            // 残りの OEM 記号 (0xBA ; / 0xBB = / 0xBD - / 0xBF / / 0xC0 `,
            // 0xDB-0xDF = [ \ ] ' OEM_8, 0xE2 = OEM_102) とテンキー演算子 (0x6A-0x6F)。
            // いずれも is_text_producing == true。Numeric (0x60..=0x69) とは互いに素。
            0x6A..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 => Self::Punctuation,
            0x09 => Self::Tab,
            0x08 => Self::Backspace,
            0x2E => Self::Delete,
            0x20 => Self::Space,
            0x0D => Self::Enter,
            // 矢印 (0x25-0x28) + PgUp/PgDown/End/Home (0x21-0x24) = 0x21..=0x28。
            0x21..=0x28 => Self::Navigation,
            // ⚠️★★★ **`0xA0..=0xA5` を 2026-08-23 に足した** (O1 / ADR-016 D8 のゲート分割)。
            //
            // ★ `WH_KEYBOARD_LL` が返すのは **左右を区別した VK** (`VK_LSHIFT`=0xA0 /
            //   `VK_RSHIFT`=0xA1 / `VK_LCONTROL`=0xA2 / `VK_RCONTROL`=0xA3 /
            //   `VK_LMENU`=0xA4 / `VK_RMENU`=0xA5) であって、汎用の 0x10/0x11/0x12 ではない。
            // ⚠️ 足さないと、観測ゲートを広げた瞬間に**修飾キーが `other` として記録され**、
            //   ★ 「修飾キーの種類だけ記録する」という同意文書 注 A の説明と食い違う。
            //
            // ⚠️ **既存ログへの影響はゼロ** —— ★ 旧 `is_typing_key` はこれらを 1 つも通していないので、
            //   既存の raw NDJSON に `0xA0..=0xA5` は現れない (⚠️ 匿名化の写像が変わる行が無い)。
            0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5 => Self::Modifier,
            0x1B => Self::Escape,
            0x70..=0x7B => Self::Function,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alphabetic => "alphabetic",
            Self::Numeric => "numeric",
            Self::Punctuation => "punctuation",
            Self::SentencePunctuation => "sentence_punctuation",
            Self::Tab => "tab",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Space => "space",
            Self::Enter => "enter",
            Self::Navigation => "navigation",
            Self::Modifier => "modifier",
            Self::Function => "function",
            Self::Escape => "escape",
            Self::Other => "other",
        }
    }
}

/// 相対化する**絶対 epoch ミリ秒**フィールド名。`idle_ms` / `dwell_ms` / `latency_ms` /
/// `wall_accum_ms` / `elapsed_ms` / `session_elapsed_ms` 等は既に相対(継続時間)なので触らない。
/// `recovered_at` は通常 raw では null だが、metrics 後付け NDJSON を誤入力した時の防御で含める
/// (null はスキップされ無害・security レビュー defense-in-depth)。
const ABSOLUTE_TIME_FIELDS: &[&str] = &[
    "t",
    "ts",
    "session_start",
    "session_end",
    "scheduled_ts",
    "analysis_window_start_ms",
    "analysis_window_end_ms",
    "recovered_at",
];

/// NDJSON 1 行を匿名化する。`session_start_ms` は当該セッションの絶対開始時刻。
/// パース不能な行は `None` (呼び出し側で除外)。
pub fn anonymize_line(line: &str, session_start_ms: u64) -> Option<String> {
    let mut v: Value = serde_json::from_str(line).ok()?;
    let obj = v.as_object_mut()?;

    // `vk`(生値=本文再構成可) があれば**無条件で** category 化する。
    // type=="key" 限定にしない: 将来 type 名変更や vk を持つ別レコードが増えても漏らさない
    // (現スキーマでは Key のみだが匿名化保証として安全側に倒す・両レビュー指摘)。
    if let Some(vk) = obj.get("vk").and_then(Value::as_u64) {
        let cat = KeyCategory::from_vk(vk as u32);
        obj.remove("vk");
        obj.insert("category".to_string(), Value::from(cat.as_str()));
    }

    // 絶対 epoch 時刻 → session_start 相対 (saturating)。
    for &field in ABSOLUTE_TIME_FIELDS {
        if let Some(n) = obj.get(field).and_then(Value::as_u64) {
            let rel = n.saturating_sub(session_start_ms);
            obj.insert(field.to_string(), Value::from(rel));
        }
    }

    serde_json::to_string(&v).ok()
}

/// raw NDJSON 全体から `meta` の `session_start` (相対化基点) を探す。
/// **フォールバックは持たない**: 正規セッションは必ず先頭に `meta.session_start` を持つため、
/// 無い入力は破損/切り詰めとみなし `None` を返す (silent fallback=0 による誤相対化を避ける)。
pub fn find_session_start(raw: &str) -> Option<u64> {
    for line in raw.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(ss) = v.get("session_start").and_then(Value::as_u64) {
                return Some(ss);
            }
        }
    }
    None
}

/// raw NDJSON 全体を匿名化する (1 行ずつ変換)。`session_start_ms` は相対化基点。
/// 戻り値: (匿名化 NDJSON, パース不能で除外した行数)。除外件数は呼び出し側が監査報告に使う。
pub fn anonymize_ndjson(raw: &str, session_start_ms: u64) -> (String, usize) {
    let mut out = String::with_capacity(raw.len());
    let mut skipped = 0usize;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match anonymize_line(line, session_start_ms) {
            Some(a) => {
                out.push_str(&a);
                out.push('\n');
            }
            None => skipped += 1,
        }
    }
    (out, skipped)
}
