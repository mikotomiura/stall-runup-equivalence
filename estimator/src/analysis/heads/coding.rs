//! Coding head v0 — エージェントコーディングドメインの推論ヘッド
//!
//! ADR-006 下位決定 #3 の Coding 側。**v0 は打鍵 β だけを持つ**。
//!
//! # なぜ β だけなのか
//!
//! ADR-006 #3 の逐語:
//!
//! > **VibeCoding**: Coding head が **現 `β_coding` を初期シードとして継承**し、以後は**自前で所有・
//! > 再チューニング**する。ただし Coding head の**主信号は打鍵 β でなく coding イベント**
//! > (accept/reject/test/revert/diff-review)。エージェントコーディングは AI 操作主体で
//! > 連続打鍵が薄いため、打鍵 β は弱い補助。
//!
//! 主信号である coding イベントの取り込みは Epic 4（`domain-adapter-boundary.md §Adapter B`）。
//! v0 が担うのは「**β の取り違えを止める**」ことだけである —— 配線前は
//! 「日本語コメントを書く coder が IME on になった瞬間 `β_writing` で採点される」という
//! 交絡が起きていた（`research-positioning.md §8`）。
//!
//! # `ime_open` を β 選択に使わない理由
//!
//! `ime_open` は **Writing head 内部の within-writing 信号**であって、ドメイン選択の役割からは
//! 切り離されている（ADR-006 Decision 2）。コーディング中に日本語コメントを打って IME が開いても、
//! 作業ドメインは coding のままなので baseline を切り替えてはならない。

use super::{DomainInferenceHead, HeadId};
use crate::analysis::features::{phi, Features};

// TUNE: β_coding — Coding head の初期シード
// Source: Dhakal et al. (2018) general population medians, coding-adjusted.
//
// ⚠️ **`writing.rs` の `BETA_WRITING_LATIN_*` と同じ数値だが、意図的に別定数として持つ。**
//   ADR-006 の帰結（下位決定 #2/#3）は「現 β_coding は v2 で 2 つの所有者に分岐する —
//   (i) Writing head の β_writing_latin（値そのまま）、(ii) Coding head の初期 β（以後独立に
//   再チューニング）。両者は同じ数値から出発するが概念的・進化的に別物」である。
//   共有定数にすると、Epic 4 で Coding 側を再チューニングした瞬間に Writing 側が巻き添えになる。
const BETA_CODING_F1: f64 = 150.0; // Flight Time median (ms)
const BETA_CODING_F3: f64 = 0.06; // Correction rate (6%)
const BETA_CODING_F4: f64 = 5.0; // Burst length (chars)
const BETA_CODING_F5: f64 = 2.0; // Pause count (per 30 s)
const BETA_CODING_F6: f64 = 0.08; // Pause-after-delete rate

/// エージェントコーディングドメインの推論ヘッド（v0 = 打鍵 β のみ）。
#[derive(Debug, Clone, Copy, Default)]
pub struct CodingHeadV0;

impl CodingHeadV0 {
    pub fn new() -> Self {
        Self
    }
}

impl DomainInferenceHead for CodingHeadV0 {
    fn head_id(&self) -> HeadId {
        HeadId::CodingV0
    }

    /// 潜在二軸 (X = Cognitive Friction, Y = Engagement) を算出する。
    ///
    /// 軸の合成式は Writing head と同一で、**β だけが違う**。
    /// v0 の時点で式まで変えると「β の取り違えを直した」のか「式を変えた」のかが
    /// 分離できなくなるため、式の変更は Epic 4 の再チューニングに送る。
    ///
    /// `ime_open` は **使わない**（上記モジュール doc 参照）。
    fn latent_axes(&self, features: &Features, _ime_open: bool) -> (f64, f64) {
        let phi1 = phi(features.f1_flight_time_median, BETA_CODING_F1);
        let phi3 = phi(features.f3_correction_rate, BETA_CODING_F3);
        let phi4 = phi(features.f4_burst_length, BETA_CODING_F4);
        let phi5 = phi(features.f5_pause_count, BETA_CODING_F5);
        let phi6 = phi(features.f6_pause_after_del_rate, BETA_CODING_F6);

        // X: Friction (高いほど「つまずき」)  重み合計 = 1.0
        let x = (0.30 * phi3 + 0.25 * phi6 + 0.25 * phi1 + 0.20 * phi5).clamp(0.0, 1.0);

        // Y: Engagement (高いほど「滑らかな出力」)  重み合計 = 1.0
        let y = (0.40 * phi4 + 0.35 * (1.0 - phi1) + 0.25 * (1.0 - phi5)).clamp(0.0, 1.0);

        (x, y)
    }

    /// ⚠️ **Writing head と同じく `!ime_open` で gate する**（`.steering/` D-3）。
    ///
    /// 2b の目的は β の取り違えを直すことであって BS 挙動を変えることではない。
    /// 日本語コメントを打つ coder の IME 変換修正を誤ペナルティしない、という
    /// 現行 gate の利点もそのまま残す。coding の「書き直し = 摩擦」を別途拾うかは、
    /// coding イベント（revert / diff-review）が入る Epic 4 で再検討する。
    fn applies_backspace_penalty(&self, ime_open: bool) -> bool {
        !ime_open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::heads::writing::WritingHead;

    fn features(f1: f64, f3: f64, f4: f64, f5: f64, f6: f64) -> Features {
        Features {
            f1_flight_time_median: f1,
            f3_correction_rate: f3,
            f4_burst_length: f4,
            f5_pause_count: f5,
            f6_pause_after_del_rate: f6,
            keys: 0,
            bs_keys: 0,
        }
    }

    /// ★ 2b の中核: **IME の on/off に関わらず β が切り替わらない**。
    ///
    /// 配線前は `ime_open` だけで β を選んでいたため、日本語コメントを書く coder が
    /// IME を開いた瞬間に `β_writing` で採点されていた（`research-positioning.md §8` の交絡）。
    #[test]
    fn vibe_coding_uses_coding_beta_regardless_of_ime() {
        let head = CodingHeadV0::new();
        let f = features(200.0, 0.12, 6.0, 3.0, 0.15);

        let on = head.latent_axes(&f, true);
        let off = head.latent_axes(&f, false);

        assert_eq!(
            on, off,
            "Coding head は ime_open で β を切り替えてはならない (ADR-006 Decision 2)"
        );
    }

    /// Coding head の β は Writing head の latin サブ baseline と**同じ数値から出発する**
    /// (ADR-006 #3)。したがって IME off の Writing と数値が一致する。
    ///
    /// ⚠️ このテストは「今は一致している」ことの記録であって、**一致し続けるべき制約ではない**。
    /// ただし落ちたときに**何が変わったのか**で扱いが 2 つに分かれる:
    ///
    /// - **β の値が分岐した** → 想定内。Epic 4 の Coding 再チューニング（ADR-006 #3）。
    ///   「別物になった」ことを示す形にテストを書き換える
    /// - **潜在二軸の式（重み・項順・clamp）が変わった** → ⚠️ **想定外。レビュー対象。**
    ///   本テストは Writing / Coding が同じ式を持つことに依存しているので、
    ///   片方の式だけを触ると落ちる。**これが式のドリフトを拾う実質的な guard になっている**
    ///   （code-reviewer 指摘・2026-08-18）
    #[test]
    fn coding_beta_starts_from_the_same_seed_as_writing_latin() {
        let coding = CodingHeadV0::new();
        let writing = WritingHead::new();
        let f = features(200.0, 0.12, 6.0, 3.0, 0.15);

        assert_eq!(
            coding.latent_axes(&f, false),
            writing.latent_axes(&f, false),
            "v0 の Coding β は β_writing_latin と同じシードから出発する"
        );
    }

    /// IME on の Writing とは**違う**（＝ 2b が実際に挙動を変えている）。
    #[test]
    fn coding_differs_from_writing_when_ime_is_open() {
        let coding = CodingHeadV0::new();
        let writing = WritingHead::new();
        let f = features(200.0, 0.12, 6.0, 3.0, 0.15);

        assert_ne!(
            coding.latent_axes(&f, true),
            writing.latent_axes(&f, true),
            "IME on では Writing が β_writing、Coding が β_coding を使うので一致しないはず"
        );
    }

    /// backspace ペナルティ gate は Writing と同じ（D-3）。
    #[test]
    fn backspace_penalty_gate_matches_writing_head() {
        let coding = CodingHeadV0::new();
        let writing = WritingHead::new();
        for ime_open in [true, false] {
            assert_eq!(
                coding.applies_backspace_penalty(ime_open),
                writing.applies_backspace_penalty(ime_open)
            );
        }
    }
}
