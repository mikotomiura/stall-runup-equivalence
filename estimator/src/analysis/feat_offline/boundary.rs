//! `T1b` (境界) — pause 直前/直後の `KeyCategory`・Space/Enter/文末句読点・IME 局面
//! (ADR-030 D10 が投資順序で最有力と名指しした Tier)。
//!
//! pause の判定そのものは `super::extract_pause_resumptions` (共有ヘルパ、
//! `features.rs::PAUSE_THRESHOLD_MS` が SSOT) にそのまま委ねる。本モジュールは
//! その出力に「直前/直後に何を押したか」を重ねるだけで、独自の pause 定義や
//! 数値定数は 1 つも持たない (Codex TASK-PRE 指摘 5)。
//!
//! IME 局面は既存 `ime_shadow` 行 (`dwell_ms` / `nav` / `abandon`) の範囲に閉じる。
//! `.idea/survey §6.3` の追加候補は `capability matrix` (ADR-032 M7) が先で、本 issue の範囲外。
//! ⚠️ ライブ変換 ON では `abandon` が偽陽性で立ちうる (起票 6) ため、件数のみを記録し、
//! 状態の解釈はコード・コメントのどちらにも書かない。
//!
//! # ⚠️★★★ `ime_*` の `0` は「使わなかった」と「計器が無い」を区別しない
//!
//! `ime_dwell_ms` / `ime_nav` / `ime_abandon` はどちらの場合も **`0`** になる。
//! ★ `capabilities.rs` は `ime_shadow` を **`config_gated`** と裁定しており (逐語
//! 「**0 行は依然「判定不能」であって「信号が無かった」ではない**」)、
//! ⚠️ **`0` を実測値と読むと「測ったふり」になる**。
//!
//! → ★ **判別は `Q` (evidence quality・issue 007) が持つ**。本モジュールは数えるだけで、
//! `Option` にもしない (⚠️ `null` にしても「未使用」と「不在」は依然分けられないため、
//! 分離の責務を 2 か所に散らさない)。
//! ⚠️ したがって **`--features` の出力を `Q` 無しで解釈しないこと**。
//!
//! ⚠️ さらに Windows と macOS では `dwell_ms` の**由来**も 1 行になる変換の**母数**も違う
//! (`capabilities.rs` 逐語「**`abandon` 率を OS 間で直接比べてはならない**」)。
//! ★ ところが `meta` 行に OS を示すフィールドは**存在しない**
//! (→ `.steering/20260914-mhd-feature-offline-tiers/blockers.md` 起票 1)。

use std::collections::BTreeMap;

use crate::analysis::anonymize::KeyCategory;
use crate::analysis::metrics::KeyPress;

/// `logger.rs::format_ime_shadow_line` が書く `ime_shadow` 行 1 件。
/// 読み出し (NDJSON → 本型) は `bin/replay.rs` が担う。
#[derive(Debug, Clone, Copy)]
pub struct ImeShadowEvent {
    pub t_ms: u64,
    pub dwell_ms: u64,
    pub nav: u32,
    pub abandon: bool,
}

/// 1 tick ぶんの `T1b` (境界) 値。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Boundary {
    /// この tick で resume した pause の「直前」(pause を開始した press) の `KeyCategory` 分布。
    /// キーは `KeyCategory::as_str()` (14 種のうち観測されたものだけ、疎な map)。
    pub prev_cat_counts: BTreeMap<String, u64>,
    /// 同じく「直後」(pause を終える press = resume 自体) の `KeyCategory` 分布。
    pub next_cat_counts: BTreeMap<String, u64>,
    /// tick 内の Space press 数。
    pub space: u64,
    /// tick 内の Enter press 数。
    pub enter: u64,
    /// tick 内の文末句読点 (`KeyCategory::SentencePunctuation`) press 数。
    pub sentence_punctuation: u64,
    /// tick に帰属する `ime_shadow` 行の `dwell_ms` 合計。
    pub ime_dwell_ms: u64,
    /// 同じく candidate nav 件数の合計。
    pub ime_nav: u64,
    /// 同じく `abandon=true` だった件数 (★ 件数のみを記録し、状態の解釈は行わない)。
    pub ime_abandon: u64,
}

/// pause 1 件ぶんの直前/直後カテゴリ。`resuming_ts` は帰属先 tick の判定に使う
/// (`super::build_feature_rows` の「pause は再開側の tick に帰属する」規則と対称)。
#[derive(Debug, Clone, Copy)]
pub struct PauseBoundaryPair {
    pub resuming_ts: u64,
    pub prev_category: KeyCategory,
    pub next_category: KeyCategory,
}

/// pause の直前/直後 press の `KeyCategory` を対応付ける。
///
/// pause の判定は `super::extract_pause_resumptions` (共有ヘルパ) の出力を**そのまま**使う。
/// 本関数はどの 2 press が対になるかを再判定しない — helper が返す resuming_ts の列と、
/// sorted press を 1 度だけ左から右へ歩く走査は常に同じ順序 (どちらも sorted press の
/// 連続ペアに基づく) なので、値の一致だけで機械的に対応付けられる。
pub fn pause_boundary_pairs(press_keys: &[KeyPress]) -> Vec<PauseBoundaryPair> {
    let mut sorted: Vec<KeyPress> = press_keys.to_vec();
    sorted.sort_by_key(|k| k.timestamp);
    let ts: Vec<u64> = sorted.iter().map(|k| k.timestamp).collect();
    let resumptions = super::extract_pause_resumptions(&ts);

    let mut pending = resumptions.into_iter();
    let mut next_expected = pending.next();
    let mut out = Vec::new();
    for i in 1..sorted.len() {
        let Some(expected) = next_expected else {
            break;
        };
        if ts[i] == expected {
            out.push(PauseBoundaryPair {
                resuming_ts: ts[i],
                prev_category: KeyCategory::from_vk(sorted[i - 1].vk),
                next_category: KeyCategory::from_vk(sorted[i].vk),
            });
            next_expected = pending.next();
        }
    }
    out
}

/// tick ごとの `Boundary` を作る純関数。呼び出し側 (`super::build_feature_rows`) が
/// 既に tick へ帰属済みの入力を渡す — 本関数自身は tick 境界判定を行わない
/// (`activity::compute_t0_activity` と対称)。
pub fn compute_boundary(
    keys_in_tick: &[KeyPress],
    pairs_in_tick: &[PauseBoundaryPair],
    ime_in_tick: &[ImeShadowEvent],
) -> Boundary {
    let mut prev_cat_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut next_cat_counts: BTreeMap<String, u64> = BTreeMap::new();
    for pair in pairs_in_tick {
        *prev_cat_counts
            .entry(pair.prev_category.as_str().to_string())
            .or_insert(0) += 1;
        *next_cat_counts
            .entry(pair.next_category.as_str().to_string())
            .or_insert(0) += 1;
    }

    let space = keys_in_tick
        .iter()
        .filter(|k| KeyCategory::from_vk(k.vk) == KeyCategory::Space)
        .count() as u64;
    let enter = keys_in_tick
        .iter()
        .filter(|k| KeyCategory::from_vk(k.vk) == KeyCategory::Enter)
        .count() as u64;
    let sentence_punctuation = keys_in_tick
        .iter()
        .filter(|k| KeyCategory::from_vk(k.vk) == KeyCategory::SentencePunctuation)
        .count() as u64;

    let ime_dwell_ms = ime_in_tick.iter().map(|e| e.dwell_ms).sum();
    let ime_nav = ime_in_tick.iter().map(|e| e.nav as u64).sum();
    let ime_abandon = ime_in_tick.iter().filter(|e| e.abandon).count() as u64;

    Boundary {
        prev_cat_counts,
        next_cat_counts,
        space,
        enter,
        sentence_punctuation,
        ime_dwell_ms,
        ime_nav,
        ime_abandon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ms: u64, vk: u32) -> KeyPress {
        KeyPress { timestamp: ms, vk }
    }

    /// AC3: prev/next_cat_counts のキーは `KeyCategory` の 14 種のうちのものだけ。
    #[test]
    fn prev_cat_keys_are_key_categories() {
        const ALL_CATEGORIES: [&str; 14] = [
            "alphabetic",
            "numeric",
            "punctuation",
            "sentence_punctuation",
            "tab",
            "backspace",
            "delete",
            "space",
            "enter",
            "navigation",
            "modifier",
            "function",
            "escape",
            "other",
        ];
        use crate::analysis::features::PAUSE_THRESHOLD_MS;
        let keys = vec![key(0, 0x41), key(PAUSE_THRESHOLD_MS, 0x08)];
        let pairs = pause_boundary_pairs(&keys);
        let boundary = compute_boundary(&keys, &pairs, &[]);
        for cat in boundary
            .prev_cat_counts
            .keys()
            .chain(boundary.next_cat_counts.keys())
        {
            assert!(
                ALL_CATEGORIES.contains(&cat.as_str()),
                "unexpected category key: {cat}"
            );
        }
    }

    /// AC4: pause 直前/直後のカテゴリが手計算と一致する。
    ///
    /// 手計算: alphabetic(A) → [pause 閾値ぶん空けて] → backspace。pause は 1 件、
    /// prev=alphabetic / next=backspace。
    #[test]
    fn boundary_categories_match_hand_computed() {
        use crate::analysis::features::PAUSE_THRESHOLD_MS;
        let keys = vec![key(0, 0x41), key(PAUSE_THRESHOLD_MS, 0x08)];
        let pairs = pause_boundary_pairs(&keys);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].resuming_ts, PAUSE_THRESHOLD_MS);
        assert_eq!(pairs[0].prev_category, KeyCategory::Alphabetic);
        assert_eq!(pairs[0].next_category, KeyCategory::Backspace);

        let boundary = compute_boundary(&keys, &pairs, &[]);
        assert_eq!(boundary.prev_cat_counts.get("alphabetic"), Some(&1));
        assert_eq!(boundary.next_cat_counts.get("backspace"), Some(&1));
    }

    #[test]
    fn space_enter_sentence_punctuation_are_counted_per_tick() {
        let keys = vec![key(0, 0x20), key(0, 0x0D), key(0, 0xBE)];
        let boundary = compute_boundary(&keys, &[], &[]);
        assert_eq!(boundary.space, 1);
        assert_eq!(boundary.enter, 1);
        assert_eq!(boundary.sentence_punctuation, 1);
    }

    #[test]
    fn ime_fields_aggregate_dwell_nav_abandon() {
        let ime = vec![
            ImeShadowEvent {
                t_ms: 0,
                dwell_ms: 650,
                nav: 2,
                abandon: false,
            },
            ImeShadowEvent {
                t_ms: 0,
                dwell_ms: 920,
                nav: 3,
                abandon: true,
            },
        ];
        let boundary = compute_boundary(&[], &[], &ime);
        assert_eq!(boundary.ime_dwell_ms, 1570);
        assert_eq!(boundary.ime_nav, 5);
        assert_eq!(boundary.ime_abandon, 1);
    }

    #[test]
    fn no_pause_yields_empty_boundary_maps() {
        let keys = vec![key(0, 0x41), key(100, 0x42)];
        let pairs = pause_boundary_pairs(&keys);
        assert!(pairs.is_empty());
        let boundary = compute_boundary(&keys, &pairs, &[]);
        assert!(boundary.prev_cat_counts.is_empty());
        assert!(boundary.next_cat_counts.is_empty());
    }

    /// 同一 timestamp の press が並ぶ境界入力でも、helper の出力順と本関数の走査順が
    /// 一致することを確認する (mod.rs の同種テストと対称)。
    /// ⚠️★★★ **2026-09-15 に検出力を足した** (`code-reviewer` MEDIUM)。
    ///
    /// ★ 旧版は 4 キーとも `0x41`〜`0x44` = すべて `Alphabetic` で、assert は
    /// `pairs.len()` と `resuming_ts` の 2 つだけだった。
    /// → ⚠️ **`prev_category` を `sorted[i-2]` に取り違えても緑のまま通る**ので、
    /// 名前が主張する「ずれ」を**原理的に検出できなかった**。
    /// → ★ カテゴリが全部異なる VK に替え、`prev_category` / `next_category` まで固定する。
    #[test]
    fn duplicate_timestamps_do_not_misalign_pairs() {
        use crate::analysis::features::PAUSE_THRESHOLD_MS;
        // ★ 4 キーすべて別カテゴリ: Alphabetic / Backspace / Space / Enter。
        let keys = vec![
            key(0, 0x41),                      // Alphabetic
            key(PAUSE_THRESHOLD_MS, 0x08),     // Backspace
            key(PAUSE_THRESHOLD_MS, 0x20),     // Space (同時刻・2 件目)
            key(2 * PAUSE_THRESHOLD_MS, 0x0D), // Enter
        ];
        let pairs = pause_boundary_pairs(&keys);
        assert_eq!(pairs.len(), 2);

        // 1 本目の pause: 0 (Alphabetic) → PAUSE_THRESHOLD_MS (Backspace)
        assert_eq!(pairs[0].resuming_ts, PAUSE_THRESHOLD_MS);
        assert_eq!(pairs[0].prev_category, KeyCategory::Alphabetic);
        assert_eq!(pairs[0].next_category, KeyCategory::Backspace);

        // 2 本目の pause: 直前は**同時刻 2 件のうち後ろ** (Space) であり、
        // ⚠️ 1 つ手前 (Backspace) でも 2 つ手前 (Alphabetic) でもない。
        assert_eq!(pairs[1].resuming_ts, 2 * PAUSE_THRESHOLD_MS);
        assert_eq!(pairs[1].prev_category, KeyCategory::Space);
        assert_eq!(pairs[1].next_category, KeyCategory::Enter);
    }
}
