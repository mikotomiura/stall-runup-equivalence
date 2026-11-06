//! `T0` (activity) — ADR-030 D10 の Tier 表で最も基本の Tier。
//!
//! keys / delete / paste / mean IKI / 単純 pause / net production の 6 値。
//! **causal** (tick が閉じた時点までの実測のみ。事後ラベルを含まない — 詳細は
//! `super` のモジュール doc「Tier ごとの causal / retrospective の区分」を参照)。
//!
//! 本ファイルは純関数のみを持つ。数値定数は 1 つも宣言しない —— pause 閾値は
//! `super::extract_pause_resumptions` (= `features.rs::PAUSE_THRESHOLD_MS`) の
//! 呼び出し側 ([`super::build_feature_rows`]) が既に適用済みの集計結果を受け取る
//! (ADR-030 D3 逐語「新しい定数を作らず、既存パラメータを対称に再利用する」)。

use crate::analysis::metrics::{is_backspace, KeyPress};

/// `T0` (activity) の 1 tick ぶんの値。
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct T0Activity {
    /// tick 内の press 総数。
    pub keys: u64,
    /// tick 内の backspace/delete press 数。
    pub deletes: u64,
    /// tick 内の paste (Ctrl+V / Cmd+V) 発生数。
    /// ★ **文字数が欠落した paste 行も数える** (`T0` は件数だけを見るため。
    /// ⚠️ `--metrics` 系が使う `collect_paste_events` は文字数の無い行を落とすので値が違う)。
    pub pastes: u64,
    /// tick に帰属する打鍵間隔の平均 (ms)。帰属規則は `super` のモジュール doc を参照。
    /// 間隔が 1 件も無い tick (press 0 or 1 件) では `null` (欠測。`0.0` にフォールバックしない)。
    ///
    /// ⚠️★★★ **`features.rs` の F1 (Flight Time 中央値) とは別定義である。**
    /// ★ F1 はキーリピートを除外し 2000ms でキャップするが、本値は**全 press の
    /// press-to-press 間隔・キャップ無し**なので、長い pause 1 本が tick の平均を支配する。
    /// ⚠️ D10 は `T0` を block ablation の**基準**に置いているので、同名で別物であることを
    /// 比較時に取り違えないこと。
    pub mean_iki_ms: Option<f64>,
    /// tick に帰属する「単純 pause」の数。
    /// ★ 判定は **連続する press の間隔** が `features.rs::PAUSE_THRESHOLD_MS` 以上であること
    /// (⚠️ 「無入力」一般ではない —— paste は間隔を切らないし、paste で再開した pause も数えない。
    /// ★ issue 005 が `stall onset` を作るときは、この違いを踏まえて判断すること)。
    pub pauses: u64,
    /// net production = 文字生成キー press 数 − backspace/delete press 数 (挿入系 − 削除系)。
    /// 削除超過の tick では負になりうる。
    pub net_production: i64,
}

/// `T0Activity` の純計算。呼び出し側 ([`super::build_feature_rows`]) が既に
/// tick へ帰属済みの入力を渡す — 本関数自身は tick 境界判定も pause 閾値判定も行わない。
///
/// - `keys_in_tick`: この tick に属する press イベント。
/// - `pastes_in_tick`: この tick に属する paste の件数。
/// - `iki_values_ms`: この tick に帰属する打鍵間隔 (ms) の列 (帰属規則は `super` を参照)。
/// - `pause_count`: 上記のうち `PAUSE_THRESHOLD_MS` 以上だった件数
///   (`super::extract_pause_resumptions` が判定済み)。
pub fn compute_t0_activity(
    keys_in_tick: &[KeyPress],
    pastes_in_tick: u64,
    iki_values_ms: &[u64],
    pause_count: u64,
) -> T0Activity {
    let keys = keys_in_tick.len() as u64;
    let deletes = keys_in_tick.iter().filter(|k| is_backspace(k.vk)).count() as u64;
    let mean_iki_ms = if iki_values_ms.is_empty() {
        None
    } else {
        Some(iki_values_ms.iter().sum::<u64>() as f64 / iki_values_ms.len() as f64)
    };
    T0Activity {
        keys,
        deletes,
        pastes: pastes_in_tick,
        mean_iki_ms,
        pauses: pause_count,
        net_production: super::net_production(keys_in_tick),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u32) -> KeyPress {
        KeyPress { timestamp: 0, vk }
    }

    /// AC5: 小さな合成入力で 6 値すべてが手計算と一致することを確認する。
    ///
    /// 手計算: press 5 件 (letter 'A' ×3, backspace ×2) + paste 1 件、
    /// 間隔 [100, 300] ms (pause 閾値未満) が 2 件 → 平均 200ms、pause 0 件。
    /// keys=5 / deletes=2 / pastes=1 / mean_iki_ms=Some(200.0) / pauses=0 /
    /// net_production = 3 (letter) − 2 (backspace) = 1。
    #[test]
    fn counts_match_hand_computed() {
        let letter = key(0x41); // 'A' — text-producing
        let backspace = key(0x08); // VK_BACK
        let keys_in_tick = [letter, letter, letter, backspace, backspace];
        let iki_values_ms = [100u64, 300u64];

        let activity = compute_t0_activity(&keys_in_tick, 1, &iki_values_ms, 0);

        assert_eq!(activity.keys, 5);
        assert_eq!(activity.deletes, 2);
        assert_eq!(activity.pastes, 1);
        assert_eq!(activity.mean_iki_ms, Some(200.0));
        assert_eq!(activity.pauses, 0);
        assert_eq!(activity.net_production, 1);
    }

    #[test]
    fn empty_tick_has_null_mean_iki_and_zero_counts() {
        let activity = compute_t0_activity(&[], 0, &[], 0);
        assert_eq!(activity.keys, 0);
        assert_eq!(activity.deletes, 0);
        assert_eq!(activity.pastes, 0);
        assert_eq!(activity.mean_iki_ms, None);
        assert_eq!(activity.pauses, 0);
        assert_eq!(activity.net_production, 0);
    }

    #[test]
    fn single_key_tick_has_null_mean_iki() {
        // press が 1 件だけの tick は間隔が定義できないため null (0.0 にフォールバックしない)。
        let activity = compute_t0_activity(&[key(0x41)], 0, &[], 0);
        assert_eq!(activity.keys, 1);
        assert_eq!(activity.mean_iki_ms, None);
    }
}
