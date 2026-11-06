//! Writing head — 論述・文章作成ドメインの推論ヘッド
//!
//! ADR-006 Alternative C (分離) の Writing 側。β を 2 セット所有し、`ime_open` で切り替える。
//!
//! - `β_writing`       … IME on。日本語 composition のリズム
//! - `β_writing_latin` … IME off。日本語論述の中の英字引用・数式・URL 等のラテン直接入力
//!
//! ⚠️ `β_writing_latin` は **旧 `β_coding` の値をそのまま改名したもの**である
//! (ADR-006 下位決定 #2「再スコープ (collapse しない)」)。
//! 「IME off の日本語論述」は Writing head 内の latin サブ baseline であって、
//! Coding の baseline ではない、という**概念的分離**であり、**数値変更を伴わない**。
//! Coding head が持つ β は別の定数として独立に所有される (以後 Coding 側で再チューニングされる)。

use super::{DomainInferenceHead, HeadId};
use crate::analysis::features::{phi, Features};

// TUNE: β_writing — 日本語 IME 入力のベースライン
// Calibrated from real session data (Mac/Windows, 2026-03).
// Typical Japanese typing: F1=100-130ms, F3=3-10%, F4=2.8-3.5, F5=1-4.
// β must be at or below the typical range so phi() produces non-zero values,
// otherwise all phi→0 and the observation bin collapses to a single Flow bin.
const BETA_WRITING_F1: f64 = 100.0; // Flight Time median (ms) — at lower bound of typical range
const BETA_WRITING_F3: f64 = 0.10; // Correction rate (10%) — IME normal correction 3-15%
const BETA_WRITING_F4: f64 = 2.5; // Burst length — shorter (composition in segments)
const BETA_WRITING_F5: f64 = 1.0; // Pause count — low threshold for sensitivity
const BETA_WRITING_F6: f64 = 0.12; // Pause-after-delete rate

// TUNE: β_writing_latin — IME off のラテン直接入力 (英字引用・数式・URL) のベースライン
// Source: Dhakal et al. (2018) general population medians, coding-adjusted.
// ⚠️ 旧 `BETA_CODING_*` の値をそのまま改名 (ADR-006 下位決定 #2・数値変更なし)。
// ラテン直接入力は日本語 composition より速く・バーストが長いため、
// β_writing に collapse するとラテン窓を誤採点する。
const BETA_WRITING_LATIN_F1: f64 = 150.0; // Flight Time median (ms)
const BETA_WRITING_LATIN_F3: f64 = 0.06; // Correction rate (6%)
const BETA_WRITING_LATIN_F4: f64 = 5.0; // Burst length (chars)
const BETA_WRITING_LATIN_F5: f64 = 2.0; // Pause count (per 30 s)
const BETA_WRITING_LATIN_F6: f64 = 0.08; // Pause-after-delete rate

/// 論述・文章作成ドメインの推論ヘッド。
///
/// Phase1 (task_kind = argumentative 固定) で走っていたのはこの head の挙動そのものである。
///
/// ⚠️ `id` は **識別子だけ**を切り替える。`writing` と `writing_fallback` で
/// **数値挙動は完全に同一**である (Creative / Other を Writing head で採点している事実を
/// ログ上で分離できるようにするためだけのもの・D-6)。
#[derive(Debug, Clone, Copy)]
pub struct WritingHead {
    id: HeadId,
}

impl WritingHead {
    /// 論述・文通向け (`argumentative` / `correspondence`)。
    pub fn new() -> Self {
        Self {
            id: HeadId::Writing,
        }
    }

    /// Creative 向けのフォールバック。**数値挙動は `new()` と同一**で、識別子だけが違う。
    pub fn fallback() -> Self {
        Self {
            id: HeadId::WritingFallback,
        }
    }
}

impl Default for WritingHead {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainInferenceHead for WritingHead {
    fn head_id(&self) -> HeadId {
        self.id
    }

    /// 潜在二軸 (X = Cognitive Friction, Y = Engagement) を算出する。
    ///
    /// # 軸の合成
    ///
    /// ```text
    /// X = 0.30 × φ(F3) + 0.25 × φ(F6) + 0.25 × φ(F1) + 0.20 × φ(F5)
    /// Y = 0.40 × φ(F4) + 0.35 × (1 − φ(F1)) + 0.25 × (1 − φ(F5))
    /// ```
    ///
    /// # Context-specific β (dual baseline)
    ///
    /// `ime_open = true`  → `β_writing`: Japanese typing baseline.
    ///   Slower flight times and shorter bursts are normal during Japanese composition.
    ///   Normalizing against a tighter baseline prevents systematic Flow over-detection
    ///   when the user is simply typing romaji at their natural Japanese pace.
    ///
    /// `ime_open = false` → `β_writing_latin`: Latin/alphanumeric baseline.
    ///   Faster flight times and longer bursts are the reference for direct latin input
    ///   (quotations, formulas, URLs) inside Japanese argumentative writing.
    ///
    /// Both sets are population-level estimates; future work can adapt them per-session
    /// using EWMA updates (update only the matching β when IME state is known).
    fn latent_axes(&self, features: &Features, ime_open: bool) -> (f64, f64) {
        let (beta_f1, beta_f3, beta_f4, beta_f5, beta_f6) = if ime_open {
            (
                BETA_WRITING_F1,
                BETA_WRITING_F3,
                BETA_WRITING_F4,
                BETA_WRITING_F5,
                BETA_WRITING_F6,
            )
        } else {
            (
                BETA_WRITING_LATIN_F1,
                BETA_WRITING_LATIN_F3,
                BETA_WRITING_LATIN_F4,
                BETA_WRITING_LATIN_F5,
                BETA_WRITING_LATIN_F6,
            )
        };

        let phi1 = phi(features.f1_flight_time_median, beta_f1);
        let phi3 = phi(features.f3_correction_rate, beta_f3);
        let phi4 = phi(features.f4_burst_length, beta_f4);
        let phi5 = phi(features.f5_pause_count, beta_f5);
        let phi6 = phi(features.f6_pause_after_del_rate, beta_f6);

        // X: Friction (高いほど「つまずき」)  重み合計 = 1.0
        let x = (0.30 * phi3 + 0.25 * phi6 + 0.25 * phi1 + 0.20 * phi5).clamp(0.0, 1.0);

        // Y: Engagement (高いほど「滑らかな出力」)  重み合計 = 1.0
        let y = (0.40 * phi4 + 0.35 * (1.0 - phi1) + 0.25 * (1.0 - phi5)).clamp(0.0, 1.0);

        (x, y)
    }

    /// IME 変換修正の BS 連打は認知的 Stuck ではなく正常操作なので、
    /// IME が開いている間はペナルティビン (obs=25) を無効化する。
    ///
    /// これは **writing 固有の within-writing 挙動**であり (ADR-006 Decision 2)、
    /// 現行 `engine.rs` の `pending && !ime_open` と等価。
    fn applies_backspace_penalty(&self, ime_open: bool) -> bool {
        !ime_open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn features(f1: f64, f3: f64, f4: f64, f5: f64, f6: f64) -> Features {
        Features {
            f1_flight_time_median: f1,
            f3_correction_rate: f3,
            f4_burst_length: f4,
            f5_pause_count: f5,
            f6_pause_after_del_rate: f6,
            // Tier 0 (記録のみ・潜在軸には流れない)
            keys: 0,
            bs_keys: 0,
        }
    }

    /// ADR-006 下位決定 #2: `β_writing_latin` は旧 `β_coding` の**値そのまま**である。
    /// 改名は数値変更を伴わない = 抽出フェーズ (2a) で byte-equal が保たれる根拠。
    #[test]
    fn beta_writing_latin_keeps_former_coding_values() {
        assert_eq!(BETA_WRITING_LATIN_F1, 150.0);
        assert_eq!(BETA_WRITING_LATIN_F3, 0.06);
        assert_eq!(BETA_WRITING_LATIN_F4, 5.0);
        assert_eq!(BETA_WRITING_LATIN_F5, 2.0);
        assert_eq!(BETA_WRITING_LATIN_F6, 0.08);
    }

    /// `ime_open` は Writing head 内部の within-writing 信号として生きている
    /// (β が実際に切り替わる)。
    #[test]
    fn ime_open_switches_beta_within_writing_head() {
        let head = WritingHead::new();
        let f = features(200.0, 0.12, 6.0, 3.0, 0.15);

        let on = head.latent_axes(&f, true);
        let off = head.latent_axes(&f, false);

        assert_ne!(
            on, off,
            "β_writing と β_writing_latin は別の値なので二軸も一致しないはず"
        );
    }

    /// ⚠️ **数値 parity の局所ピン留め**（cross-review Codex 指摘・2026-08-18）。
    ///
    /// `verify-extraction-parity.sh` は「旧 HEAD と同じ出力か」しか見ないため、
    /// 旧実装ごと間違えていた場合は検出できない。ここでは φ の定義と重みから
    /// **手計算した期待値**を直接ピン留めし、重み・項順・β 割当のミスを局所的に拾う。
    ///
    /// φ(x, β) = clamp((x − β) / (2β), 0, 1)
    ///
    /// IME on (β_writing = 100 / 0.10 / 2.5 / 1.0 / 0.12):
    ///   φ1 = (200−100)/200   = 0.5
    ///   φ3 = (0.12−0.10)/0.20 = 0.1
    ///   φ4 = (6.0−2.5)/5.0   = 0.7
    ///   φ5 = (3.0−1.0)/2.0   = 1.0
    ///   φ6 = (0.15−0.12)/0.24 = 0.125
    ///   X = 0.30·0.1 + 0.25·0.125 + 0.25·0.5 + 0.20·1.0 = 0.38625
    ///   Y = 0.40·0.7 + 0.35·0.5  + 0.25·0.0             = 0.455
    ///
    /// IME off (β_writing_latin = 150 / 0.06 / 5.0 / 2.0 / 0.08):
    ///   φ1 = (200−150)/300   = 1/6
    ///   φ3 = (0.12−0.06)/0.12 = 0.5
    ///   φ4 = (6.0−5.0)/10.0  = 0.1
    ///   φ5 = (3.0−2.0)/4.0   = 0.25
    ///   φ6 = (0.15−0.08)/0.16 = 0.4375
    ///   X = 0.30·0.5 + 0.25·0.4375 + 0.25·(1/6) + 0.20·0.25
    ///   Y = 0.40·0.1 + 0.35·(5/6)  + 0.25·0.75
    #[test]
    fn latent_axes_match_hand_computed_values() {
        let head = WritingHead::new();
        let f = features(200.0, 0.12, 6.0, 3.0, 0.15);
        const EPS: f64 = 1e-12;

        let (x_on, y_on) = head.latent_axes(&f, true);
        assert!(
            (x_on - 0.38625).abs() < EPS,
            "IME on の X が手計算と違う: {x_on}"
        );
        assert!(
            (y_on - 0.455).abs() < EPS,
            "IME on の Y が手計算と違う: {y_on}"
        );

        let (x_off, y_off) = head.latent_axes(&f, false);
        let expect_x_off = 0.30 * 0.5 + 0.25 * 0.4375 + 0.25 * (1.0 / 6.0) + 0.20 * 0.25;
        let expect_y_off = 0.40 * 0.1 + 0.35 * (5.0 / 6.0) + 0.25 * 0.75;
        assert!(
            (x_off - expect_x_off).abs() < EPS,
            "IME off の X が手計算と違う: {x_off}"
        );
        assert!(
            (y_off - expect_y_off).abs() < EPS,
            "IME off の Y が手計算と違う: {y_off}"
        );
    }

    /// backspace ペナルティ gate は Writing 固有の挙動として維持される。
    #[test]
    fn backspace_penalty_is_gated_by_ime() {
        let head = WritingHead::new();
        assert!(head.applies_backspace_penalty(false));
        assert!(!head.applies_backspace_penalty(true));
    }
}
