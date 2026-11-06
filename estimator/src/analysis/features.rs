use std::collections::VecDeque;

/// TUNE: ポーズ判定閾値 2000 ms (F5, F6 共通)。
///
/// ⚠️★★★ **本定数は「単純 pause」(= 2 秒以上の無入力) の唯一の真実源 (SSOT) である。**
/// ★ オフライン側 (`analysis::feat_offline` / `gse-replay --features`) の `T0` は
/// **ここを参照する** (ADR-030 D3 逐語「**新しい定数を作らず、既存パラメータを対称に再利用する**」)。
/// ⚠️ **別の場所で 2000 を再定義しないこと。**
/// ★ 先例は `analysis::intervention::AWAY_IDLE_MS` → `OutcomeConfig::away_gap_ms` (ADR-024 D3 / AC-3)。
///
/// ★ 命名の経緯: 2026-09-15 に `.steering/20260914-mhd-feature-offline-tiers/decisions.md`
/// **判断 7** で、値を 1 ビットも変えない命名のみのリファクタとして抽出した
/// (⚠️ `HMM_VERSION` は bump しない —— `engine.rs:28-37` の bump 対象は
/// 「`phi` / `make_silence_observation` / `calculate_features` の**数値挙動変更**」であり、
/// `u64` 比較のリテラルを同値の定数へ置き換える操作はこれに当たらない。★ ゴールデン replay で実測確認)。
///
/// ⚠️★★★ **同じ閾値を秒で表した箇所が 1 つ残っている**: `make_silence_observation` の
/// `silence_secs < 2.0` (doc 逐語「ポーズとして認識しない」)。
/// ★ そこを本定数から導出しないのは、当該関数が `engine.rs` の bump 対象に**名指しされている**うえ
/// 浮動小数経路だからである (`engine.rs:19-26` の EWMA α 定数化ポリシーと同じ警戒)。
/// → ★ **代わりに結合をテストで固定する**: `tests::pause_threshold_matches_silence_gate`。
///
/// ⚠️ 値が同じ `2000` でも**別の knob** なので混同しないこと:
/// F1 Flight Time の外れ値キャップ (`ft < 2000.0`) と、その飽和値
/// (`make_silence_observation` の `f1 = 2000.0`) は本定数と無関係である。
pub const PAUSE_THRESHOLD_MS: u64 = 2_000;

/// TUNE: 特徴量計算ウィンドウ = 30 秒 (30,000 ms)。`FeatureExtractor::calculate_features` の
/// 「直近 N ms」の N。
///
/// ★ 命名の経緯: 2026-09-25 に `.steering/20260921-p1-prereg-cli-poles/` の issue 003 で、
/// **値を 1 ビットも変えない命名のみのリファクタ**として、名前の無い生リテラル `30_000` から
/// 抽出した (`PAUSE_THRESHOLD_MS` の抽出と同じ型)。
/// ⚠️ `HMM_VERSION` は bump しない —— `engine.rs` の bump 対象は数値挙動の変更であり、
/// `u64` の引き算のリテラルを同値の定数へ置き換える操作はこれに当たらない
/// (★ ゴールデン replay の byte-equal で確認する)。
///
/// ⚠️ **オフライン側の `T1a` の窓 (`StallOnsetConfig::t1a_window_ms`) はこの定数を参照しない。**
/// ★ 値が同じ 30 秒系列というだけで**別の knob** である (`T1a` の窓の SSOT は
/// `src-tauri/prereg/p1-episode.json`)。ここを変えても `T1a` の窓は動かず、逆も同じ。
pub const FEATURE_WINDOW_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub vk_code: u32,
    pub timestamp: u64, // ms
    pub is_press: bool,
    pub is_repeat: bool,
    pub is_backspace: bool,
    /// ペーストショートカット (Ctrl+V / Cmd+V) の押下フラグ。
    /// `input/` のフック層が修飾キー状態 (Windows: Atomic 追跡 / macOS: CGEvent flags) から
    /// 判定して立てる。**クリップボードの中身には一切触れない** (回数記録のみ、data-privacy 準拠)。
    /// FeatureExtractor は本フラグを読まないため FeatSnapshot に影響しない。
    pub is_paste: bool,
}

/// B-1: 5特徴量を格納する構造体
/// F2 (Flight Time 分散) は HMM の潜在軸計算で未使用のため除去済み。
#[derive(Debug, Clone, Copy)]
pub struct Features {
    /// F1: Flight Time 中央値 (ms)
    pub f1_flight_time_median: f64,
    /// F3: 修正率 = (BS + Del) / 全キー数
    pub f3_correction_rate: f64,
    /// F4: バースト長 = 連続FT<200ms の平均文字数
    pub f4_burst_length: f64,
    /// F5: ポーズ回数 = 2秒以上の無入力回数
    pub f5_pause_count: f64,
    /// F6: 削除後停止率 = BS/Del直後2秒以上停止する割合
    pub f6_pause_after_del_rate: f64,
    /// ★ Tier 0 (記録のみ): **窓内の総キー押下数** = `f3_correction_rate` の**分母**。
    ///
    /// ⚠️★★★ **特徴量ではない。** HMM にも X/Y 合成式にも 1 バイトも流さない。
    /// ⚠️★★★ **窓の右端は「最後の打鍵時刻」であって「今」ではない。**
    /// **累積カウンタでもない。** 意味の正本は
    /// `.steering/20260801-recovery-axis-grounding/tier0-wiring-design.md` §2 と、
    /// `analysis/loop_step.rs` の `FeatSnapshot::keys` の doc を参照 (★ そこに実測つきで書いてある)。
    pub keys: u64,
    /// ★ Tier 0 (記録のみ): **窓内の BS/Del 押下数** = `f3_correction_rate` の**分子**。
    ///
    /// `f3` は `{:.4}` に丸めて NDJSON へ書かれるため `f3 × keys` は整数に戻らない。
    /// 丸め誤差を持ち込まないよう整数で別途持つ (案 B)。注意点は `keys` と同じ。
    pub bs_keys: u64,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            f1_flight_time_median: 0.0,
            f3_correction_rate: 0.0,
            f4_burst_length: 0.0,
            f5_pause_count: 0.0,
            f6_pause_after_del_rate: 0.0,
            keys: 0,
            bs_keys: 0,
        }
    }
}

/// B-2: 個人ベースライン正規化関数 φ(x, β) = clamp((x − β) / (κ · β), 0.0, 1.0)
///
/// TUNE: κ = 2.0 — 「β の 2 倍を上限」として正規化する感度パラメータ
pub fn phi(x: f64, beta: f64) -> f64 {
    const KAPPA: f64 = 2.0;
    if beta <= 0.0 {
        return 0.0;
    }
    ((x - beta) / (KAPPA * beta)).clamp(0.0, 1.0)
}

pub struct FeatureExtractor {
    buffer: VecDeque<InputEvent>,
    capacity: usize,
}

impl FeatureExtractor {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// セッション開始時にバッファをクリアする。
    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    pub fn process_event(&mut self, event: InputEvent) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(event);
    }

    /// B-1: 直近30秒のバッファから5特徴量を算出する
    pub fn calculate_features(&self) -> Features {
        if self.buffer.is_empty() {
            return Features::default();
        }

        let last_ts = self.buffer.back().unwrap().timestamp;
        // TUNE: 特徴量計算ウィンドウ = FEATURE_WINDOW_MS (30 秒)
        let cutoff = last_ts.saturating_sub(FEATURE_WINDOW_MS);

        // 直近30秒のイベントを収集
        let events: Vec<&InputEvent> = self
            .buffer
            .iter()
            .filter(|e| e.timestamp >= cutoff)
            .collect();

        if events.is_empty() {
            return Features::default();
        }

        // --- ウィンドウ内のフライトタイムを計算 (F1) ---
        // キーリピートを除外して正確なタイピングリズムを測定する。
        // リピート判定はフック層の is_repeat フラグを使用する。
        // 注: 非タイピングキーはフック層 (hook.rs) で既にフィルタ済み。
        let mut window_fts: Vec<f64> = Vec::new();
        let mut last_release: Option<u64> = None;

        for event in &events {
            if event.is_repeat {
                continue;
            }
            if event.is_press {
                if let Some(rel) = last_release {
                    if event.timestamp >= rel {
                        let ft = (event.timestamp - rel) as f64;
                        // TUNE: Flight Time の上限キャップ 2000 ms (これ以上は外れ値として除外)
                        if ft < 2000.0 {
                            window_fts.push(ft);
                        }
                    }
                }
            } else {
                last_release = Some(event.timestamp);
            }
        }

        // --- F1: Flight Time 中央値 (30秒ウィンドウ内) ---
        // F2 除去済みのため in-place ソート (clone不要)
        let f1 = if window_fts.is_empty() {
            0.0
        } else {
            window_fts
                .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let len = window_fts.len();
            if len.is_multiple_of(2) {
                (window_fts[len / 2 - 1] + window_fts[len / 2]) / 2.0
            } else {
                window_fts[len / 2]
            }
        };

        // --- キー押下イベントのみ抽出 ---
        let press_events: Vec<&&InputEvent> = events.iter().filter(|e| e.is_press).collect();

        let total_keys = press_events.len();

        // --- F3: 修正率 = (BS + Del) / 全キー押下数 ---
        let correction_keys = press_events.iter().filter(|e| e.is_backspace).count();

        let f3 = if total_keys > 0 {
            correction_keys as f64 / total_keys as f64
        } else {
            0.0
        };

        // --- F4: バースト長 = 連続FT<200ms のチャンクの平均文字数 ---
        // リピートイベントを除外して真のタイピングバーストを測定する。
        let mut burst_lengths: Vec<usize> = Vec::new();
        let mut current_burst: usize = 0;
        let mut last_rel_for_burst: Option<u64> = None;

        for event in &events {
            if event.is_repeat {
                continue;
            }
            if event.is_press {
                if let Some(rel) = last_rel_for_burst {
                    let ft = event.timestamp.saturating_sub(rel);
                    // TUNE: バースト判定閾値 200 ms (これ未満の連続打鍵をバーストとみなす)
                    if ft < 200 {
                        current_burst += 1;
                    } else {
                        if current_burst > 0 {
                            burst_lengths.push(current_burst);
                        }
                        current_burst = 1;
                    }
                } else {
                    current_burst = 1;
                }
            } else {
                last_rel_for_burst = Some(event.timestamp);
            }
        }
        if current_burst > 0 {
            burst_lengths.push(current_burst);
        }

        let f4 = if !burst_lengths.is_empty() {
            burst_lengths.iter().sum::<usize>() as f64 / burst_lengths.len() as f64
        } else {
            0.0
        };

        // --- F5: ポーズ回数 = 連続キー押下間で2秒以上の間隔の数 ---
        let press_ts: Vec<u64> = events
            .iter()
            .filter(|e| e.is_press)
            .map(|e| e.timestamp)
            .collect();

        // TUNE: ポーズ判定閾値は `PAUSE_THRESHOLD_MS` (F5, F6 共通・SSOT)
        let f5 = press_ts
            .windows(2)
            .filter(|w| w[1].saturating_sub(w[0]) >= PAUSE_THRESHOLD_MS)
            .count() as f64;

        // --- F6: 削除後停止率 = BS/Del直後に2秒以上停止する割合 ---
        let del_count = press_events.iter().filter(|e| e.is_backspace).count();

        // 連続するキー押下ペアで、先頭がBS/Del かつ間隔>=2sのものを数える
        let del_followed_by_pause = press_ts
            .windows(2)
            .zip(press_events.windows(2))
            .filter(|(ts_win, ev_win)| {
                let is_del = ev_win[0].is_backspace;
                let long_pause = ts_win[1].saturating_sub(ts_win[0]) >= PAUSE_THRESHOLD_MS;
                is_del && long_pause
            })
            .count();

        let f6 = if del_count > 0 {
            del_followed_by_pause as f64 / del_count as f64
        } else {
            0.0
        };

        Features {
            f1_flight_time_median: f1,
            f3_correction_rate: f3,
            f4_burst_length: f4,
            f5_pause_count: f5,
            f6_pause_after_del_rate: f6,
            // ★ Tier 0 (記録のみ): 上で F3 のために既に数え終えている値をそのまま載せる。
            // 新規の走査・計算はゼロ。
            keys: total_keys as u64,
            bs_keys: correction_keys as u64,
        }
    }

    /// 沈黙 3 状態機構の **SilencePath** 用合成観測値を生成する（バッファ空時に使用）。
    ///
    /// 呼び出し元: `loop_step.rs` の SilencePath ブランチ（バッファ空時）。
    /// TransitionPath（バッファあり + 沈黙）は独自の摩擦フロアを持ち、
    /// 本関数を呼ばない — 閾値の不一致は設計上の分離（詳細は loop_step.rs のコメント参照）。
    ///
    /// # Phase 2a: 比例キャリーフォワード方式
    ///
    /// `pre_silence` が `Some` の場合、直前のタイピング特徴量 (F3/F6) を文脈として
    /// 沈黙中の合成摩擦値に比例反映する。クリーンなタイピング後は摩擦 ≈ 0、
    /// 修正の多いタイピング後は高い摩擦がキャリーフォワードされる。
    /// `pre_silence` が `None` の場合（セッション冒頭等）は従来の時間ベースランプにフォールバック。
    ///
    /// `silence_secs < 2.0` の場合は None を返す（ポーズとして認識しない）。
    pub fn make_silence_observation(
        &self,
        silence_secs: f64,
        pre_silence: Option<&Features>,
    ) -> Option<Features> {
        if silence_secs < 2.0 {
            return None;
        }

        // TUNE: F1 = 2000 ms — 「現在タイピングしていない = Flight Time が事実上無限大」の意味付け
        let f1 = 2000.0_f64;

        let (f3_synthetic, f6_synthetic) = match pre_silence {
            Some(ctx) => {
                // Phase 2a: 比例キャリーフォワード
                // 直前の F3/F6 を基準に、時間とともに緩やかに増幅する。
                // 式: ctx_val + time_scale × (1.0 - ctx_val) × ctx_val
                //   ctx_val=0 → 常に 0 (摩擦なし → 沈黙中も摩擦なし)
                //   ctx_val=0.5, t=1.0 → 0.75 (中程度 → 緩やかに増加)
                //   ctx_val=1.0 → 常に 1.0 (最大摩擦 → そのまま維持)
                // TUNE: time_scale ramp = 30s (onset 2s、32s で 1.0)
                // 注: TransitionPath (loop_step.rs) は onset 8s の線形式 (ctx × time_scale) を使う。
                //   SilencePath はバッファ空のため onset を早め (2s) にし、非線形増幅で
                //   長時間沈黙の Stuck 移行を加速する。TransitionPath はバッファデータが
                //   あるため onset を遅め (8s) に抑え、線形でバッファ値を補完する設計。
                let time_scale = ((silence_secs - 2.0) / 30.0).clamp(0.0, 1.0);
                let f3_ctx = ctx.f3_correction_rate.clamp(0.0, 1.0);
                let f6_ctx = ctx.f6_pause_after_del_rate.clamp(0.0, 1.0);
                let f3 = (f3_ctx + time_scale * (1.0 - f3_ctx) * f3_ctx).clamp(0.0, 1.0);
                let f6 = (f6_ctx + time_scale * (1.0 - f6_ctx) * f6_ctx).clamp(0.0, 1.0);
                (f3, f6)
            }
            None => {
                // フォールバック: 従来の時間ベースランプ (セッション冒頭等の文脈なし時)
                //   F6: onset 10s / ramp 40s / floor 0.15 (≥ 10s 時)
                //   F3: onset 15s / ramp 50s / floor 0.20 (≥ 10s 時)
                let f6_fb = if silence_secs >= 10.0 {
                    (((silence_secs - 10.0) / 40.0).clamp(0.0, 1.0)).max(0.15)
                } else {
                    ((silence_secs - 10.0) / 40.0).clamp(0.0, 1.0)
                };
                let f3_fb = if silence_secs >= 10.0 {
                    (((silence_secs - 15.0) / 50.0).clamp(0.0, 1.0)).max(0.20)
                } else {
                    ((silence_secs - 15.0) / 50.0).clamp(0.0, 1.0)
                };
                (f3_fb, f6_fb)
            }
        };

        Some(Features {
            f1_flight_time_median: f1,
            f3_correction_rate: f3_synthetic,
            f4_burst_length: 0.0,
            f5_pause_count: silence_secs / 2.0,
            f6_pause_after_del_rate: f6_synthetic,
            // ★ Tier 0: 本関数は**合成観測**であり、打鍵を数えていない。
            // 0 を「実測 0 打」の意味で使わないよう、呼び出し側 (`loop_step::step`) が
            // `calculate_features()` の実測値で必ず上書きする。ここでの 0 は
            // FeatSnapshot に載らない (loop_step.rs の Feat 記録部を参照)。
            keys: 0,
            bs_keys: 0,
        })
    }
}

// ============================================================================
// Unit tests (diagnostic — capture current behavior to prevent regressions)
// ============================================================================
//
// Phase 0 リファクタの一環として既存挙動を捕獲する。
// 将来のチューニングで意図的にパラメータを変更した際は、テスト側も一緒に更新する。
// - phi(): 境界値・ゼロ除算ガード
// - make_silence_observation(): 閾値前後のフロア/ランプ挙動
// - calculate_features(): 既知入力列 → 期待特徴量

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < EPS
    }

    // ---------- phi() ----------

    #[test]
    fn phi_at_baseline_returns_zero() {
        assert!(approx_eq(phi(0.06, 0.06), 0.0));
    }

    #[test]
    fn phi_below_baseline_clamps_to_zero() {
        // x < β は下限クランプで 0.0
        assert!(approx_eq(phi(0.03, 0.06), 0.0));
    }

    #[test]
    fn phi_midpoint_returns_half() {
        // φ(x=2β, β) = (2β − β) / (2β) = 0.5
        assert!(approx_eq(phi(0.12, 0.06), 0.5));
    }

    #[test]
    fn phi_at_triple_baseline_clamps_to_one() {
        // φ(x=3β, β) = (3β − β) / (2β) = 1.0（上限クランプ境界）
        assert!(approx_eq(phi(0.18, 0.06), 1.0));
    }

    #[test]
    fn phi_way_above_baseline_clamps_to_one() {
        assert!(approx_eq(phi(10.0, 0.06), 1.0));
    }

    #[test]
    fn phi_with_zero_beta_returns_zero() {
        // β=0 ゼロ除算ガード
        assert!(approx_eq(phi(1.0, 0.0), 0.0));
    }

    #[test]
    fn phi_with_negative_beta_returns_zero() {
        assert!(approx_eq(phi(1.0, -0.1), 0.0));
    }

    // ---------- make_silence_observation() ----------
    // Phase 2a: pre_silence=None のテストはフォールバック互換、
    //           pre_silence=Some のテストは比例キャリーフォワード検証。

    fn extractor() -> FeatureExtractor {
        FeatureExtractor::new(600)
    }

    // --- フォールバック (pre_silence=None): 従来の時間ベースランプと同一 ---

    #[test]
    fn silence_below_2s_returns_none() {
        assert!(extractor().make_silence_observation(1.9, None).is_none());
        assert!(extractor().make_silence_observation(0.0, None).is_none());
    }

    /// ★ `PAUSE_THRESHOLD_MS` (SSOT) と `make_silence_observation` の秒単位ゲートが
    /// **同じ境界**であることを固定する。
    ///
    /// ⚠️ 当該ゲート (`silence_secs < 2.0`) は浮動小数経路かつ `engine.rs` の
    /// `HMM_VERSION` bump 対象に名指しされているため、定数から導出する形にしていない。
    /// → ★ 代わりに本テストが結合を保証する (`decisions.md` 判断 7)。
    /// ⚠️ 片方だけを変えると赤になる。
    #[test]
    fn pause_threshold_matches_silence_gate() {
        let threshold_secs = PAUSE_THRESHOLD_MS as f64 / 1000.0;
        // 閾値ちょうどはポーズとして認識される
        assert!(extractor()
            .make_silence_observation(threshold_secs, None)
            .is_some());
        // 閾値の直下は認識されない
        assert!(extractor()
            .make_silence_observation(threshold_secs - 0.001, None)
            .is_none());
    }

    #[test]
    fn silence_at_2s_has_no_friction_floors() {
        let f = extractor().make_silence_observation(2.0, None).unwrap();
        assert!(approx_eq(f.f1_flight_time_median, 2000.0));
        assert!(approx_eq(f.f3_correction_rate, 0.0));
        assert!(approx_eq(f.f4_burst_length, 0.0));
        assert!(approx_eq(f.f5_pause_count, 1.0));
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.0));
    }

    #[test]
    fn silence_just_before_10s_has_no_floors() {
        let f = extractor().make_silence_observation(9.99, None).unwrap();
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.0));
        assert!(approx_eq(f.f3_correction_rate, 0.0));
    }

    #[test]
    fn silence_at_10s_applies_both_floors() {
        let f = extractor().make_silence_observation(10.0, None).unwrap();
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.15));
        assert!(approx_eq(f.f3_correction_rate, 0.20));
        assert!(approx_eq(f.f5_pause_count, 5.0));
        assert!(approx_eq(f.f1_flight_time_median, 2000.0));
    }

    #[test]
    fn silence_at_30s_f6_ramps_past_floor() {
        let f = extractor().make_silence_observation(30.0, None).unwrap();
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.5));
        assert!(approx_eq(f.f3_correction_rate, 0.3));
        assert!(approx_eq(f.f5_pause_count, 15.0));
    }

    #[test]
    fn silence_at_50s_f6_saturates_f3_near_saturation() {
        let f = extractor().make_silence_observation(50.0, None).unwrap();
        assert!(approx_eq(f.f6_pause_after_del_rate, 1.0));
        assert!(approx_eq(f.f3_correction_rate, 0.7));
    }

    #[test]
    fn silence_at_100s_both_saturate() {
        let f = extractor().make_silence_observation(100.0, None).unwrap();
        assert!(approx_eq(f.f6_pause_after_del_rate, 1.0));
        assert!(approx_eq(f.f3_correction_rate, 1.0));
    }

    // --- 比例キャリーフォワード (pre_silence=Some) ---

    fn clean_features() -> Features {
        Features {
            f1_flight_time_median: 95.0,
            f3_correction_rate: 0.02,
            f4_burst_length: 5.0,
            f5_pause_count: 0.0,
            f6_pause_after_del_rate: 0.00,
            keys: 100,
            bs_keys: 2,
        }
    }

    fn high_friction_features() -> Features {
        Features {
            f1_flight_time_median: 300.0,
            f3_correction_rate: 0.40,
            f4_burst_length: 2.0,
            f5_pause_count: 3.0,
            f6_pause_after_del_rate: 0.30,
            keys: 50,
            bs_keys: 20,
        }
    }

    #[test]
    fn context_clean_typing_keeps_friction_near_zero() {
        // クリーンなタイピング後の沈黙 → F3/F6 ≈ 0
        let ctx = clean_features();
        let f = extractor()
            .make_silence_observation(10.0, Some(&ctx))
            .unwrap();
        // F3: 0.02 + (8/30) × 0.98 × 0.02 ≈ 0.025
        assert!(f.f3_correction_rate < 0.03);
        // F6: 0.0 + anything × 0 = 0.0
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.0));
    }

    #[test]
    fn context_high_friction_carries_forward() {
        // 高摩擦タイピング後の沈黙 → F3/F6 が高い値をキャリーフォワード
        let ctx = high_friction_features();
        let f = extractor()
            .make_silence_observation(10.0, Some(&ctx))
            .unwrap();
        // F3: 0.40 + (8/30) × 0.60 × 0.40 ≈ 0.464
        assert!(f.f3_correction_rate > 0.40);
        assert!(f.f3_correction_rate < 0.50);
        // F6: 0.30 + (8/30) × 0.70 × 0.30 ≈ 0.356
        assert!(f.f6_pause_after_del_rate > 0.30);
        assert!(f.f6_pause_after_del_rate < 0.40);
    }

    #[test]
    fn context_at_max_time_scale_amplifies() {
        // 32s (time_scale=1.0): 最大増幅
        let ctx = high_friction_features();
        let f = extractor()
            .make_silence_observation(32.0, Some(&ctx))
            .unwrap();
        // F3: 0.40 + 1.0 × 0.60 × 0.40 = 0.64
        assert!((f.f3_correction_rate - 0.64).abs() < 0.001);
        // F6: 0.30 + 1.0 × 0.70 × 0.30 = 0.51
        assert!((f.f6_pause_after_del_rate - 0.51).abs() < 0.001);
    }

    #[test]
    fn context_at_2s_equals_raw_context_values() {
        // 2s (time_scale=0.0): 増幅なし、文脈値そのまま
        let ctx = high_friction_features();
        let f = extractor()
            .make_silence_observation(2.0, Some(&ctx))
            .unwrap();
        assert!(approx_eq(f.f3_correction_rate, 0.40));
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.30));
    }

    #[test]
    fn context_below_2s_still_returns_none() {
        let ctx = high_friction_features();
        assert!(extractor()
            .make_silence_observation(1.5, Some(&ctx))
            .is_none());
    }

    // ---------- calculate_features() ----------

    fn key_press(vk: u32, t: u64) -> InputEvent {
        InputEvent {
            vk_code: vk,
            timestamp: t,
            is_press: true,
            is_repeat: false,
            is_backspace: false,
            is_paste: false,
        }
    }

    fn key_release(vk: u32, t: u64) -> InputEvent {
        InputEvent {
            vk_code: vk,
            timestamp: t,
            is_press: false,
            is_repeat: false,
            is_backspace: false,
            is_paste: false,
        }
    }

    fn backspace_press(t: u64) -> InputEvent {
        InputEvent {
            vk_code: 0x08, // VK_BACK
            timestamp: t,
            is_press: true,
            is_repeat: false,
            is_backspace: true,
            is_paste: false,
        }
    }

    #[test]
    fn calculate_features_empty_buffer_returns_defaults() {
        let e = extractor();
        let f = e.calculate_features();
        assert!(approx_eq(f.f1_flight_time_median, 0.0));
        assert!(approx_eq(f.f3_correction_rate, 0.0));
        assert!(approx_eq(f.f4_burst_length, 0.0));
        assert!(approx_eq(f.f5_pause_count, 0.0));
        assert!(approx_eq(f.f6_pause_after_del_rate, 0.0));
        // ★ Tier 0: バッファ空は「実測 0 打」なので 0 が正しい意味を持つ。
        assert_eq!(f.keys, 0);
        assert_eq!(f.bs_keys, 0);
    }

    #[test]
    fn calculate_features_flight_time_median_from_press_release_pairs() {
        // press/release シーケンス:
        //   A press@0    release@50
        //   B press@150  release@200  → FT = 150 − 50 = 100
        //   C press@350  release@400  → FT = 350 − 200 = 150
        //   D press@500  release@550  → FT = 500 − 400 = 100
        // fts = [100, 150, 100] sorted = [100, 100, 150], len=3, median = index 1 = 100
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(key_release(0x41, 50));
        e.process_event(key_press(0x42, 150));
        e.process_event(key_release(0x42, 200));
        e.process_event(key_press(0x43, 350));
        e.process_event(key_release(0x43, 400));
        e.process_event(key_press(0x44, 500));
        e.process_event(key_release(0x44, 550));
        let f = e.calculate_features();
        assert!(approx_eq(f.f1_flight_time_median, 100.0));
    }

    #[test]
    fn calculate_features_correction_rate_counts_backspace_ratio() {
        // Press 3 keys: A, BS, B → F3 = 1/3
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(backspace_press(100));
        e.process_event(key_press(0x42, 200));
        let f = e.calculate_features();
        assert!(approx_eq(f.f3_correction_rate, 1.0 / 3.0));
    }

    // ---------- Tier 0: keys / bs_keys (記録のみ) ----------

    #[test]
    fn tier0_key_counts_are_the_f3_numerator_and_denominator() {
        // ★ keys = F3 の分母、bs_keys = F3 の分子。定義の一致を固定する。
        // Press 5 keys: A, BS, B, BS, C → bs_keys=2 / keys=5 → f3 = 0.4
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(backspace_press(100));
        e.process_event(key_press(0x42, 200));
        e.process_event(backspace_press(300));
        e.process_event(key_press(0x43, 400));
        let f = e.calculate_features();
        assert_eq!(f.keys, 5);
        assert_eq!(f.bs_keys, 2);
        // 丸めのない整数から f3 を再構成できること (案 B の根拠)
        assert!(approx_eq(
            f.f3_correction_rate,
            f.bs_keys as f64 / f.keys as f64
        ));
        // 純産出 (progress_proxy) が整数で得られること
        assert_eq!(f.keys - f.bs_keys, 3);
    }

    #[test]
    fn tier0_key_counts_exclude_releases_but_include_repeats() {
        // ★ 意味の固定: press のみを数え、release は数えない。
        // ⚠️ リピート (BS 長押し) は **数える** — F3 の分母と同じ母集団に揃えるため。
        //    (engine.register_keystroke も同じ理由でリピートを含める)
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(key_release(0x41, 50)); // 数えない
        let mut repeat = backspace_press(100);
        repeat.is_repeat = true;
        e.process_event(repeat); // 数える
        e.process_event(backspace_press(200));
        let f = e.calculate_features();
        assert_eq!(f.keys, 3);
        assert_eq!(f.bs_keys, 2);
    }

    #[test]
    fn tier0_key_counts_use_the_30s_window_ending_at_the_last_event() {
        // ⚠️★★★ 窓の右端は **バッファ最後のイベント時刻**であって「今」ではない。
        // cutoff = last_ts − 30_000 なので、last_ts が古いままなら窓も古いまま動かない。
        let mut e = extractor();
        e.process_event(key_press(0x41, 0)); // 窓外になる
        e.process_event(key_press(0x42, 1_000)); // 窓外になる
        e.process_event(key_press(0x43, 40_000));
        e.process_event(backspace_press(41_000));
        // cutoff = 41_000 − 30_000 = 11_000 → 先頭 2 件が落ちる
        let f = e.calculate_features();
        assert_eq!(f.keys, 2);
        assert_eq!(f.bs_keys, 1);
    }

    #[test]
    fn tier0_key_counts_freeze_while_silent_because_the_window_follows_the_last_event() {
        // ⚠️★★★ **沈黙が何秒続いても値は減らない。** `calculate_features()` は
        // 「今」を知らないので、打鍵が止まった時点の窓をそのまま返し続ける。
        // ★ これが「feat 行をまたいで keys を足し上げてはいけない」理由である
        // (実測: tests/snapshots/deep_silence.json は 44 行連続で keys=12)。
        let mut e = extractor();
        e.process_event(key_press(0x41, 1_000));
        e.process_event(backspace_press(2_000));
        e.process_event(key_press(0x42, 3_000));
        let first = e.calculate_features();
        assert_eq!((first.keys, first.bs_keys), (3, 1));
        // 打鍵を 1 つも足さずに何度呼んでも同じ値が返る (＝毎秒の feat 行が同じ値を書く)。
        let again = e.calculate_features();
        assert_eq!((again.keys, again.bs_keys), (3, 1));
    }

    #[test]
    fn tier0_synthetic_silence_observation_carries_no_key_counts() {
        // ★ 合成観測は打鍵を数えていない。呼び出し側 (loop_step) が実測値で上書きする契約。
        let ctx = high_friction_features();
        let f = extractor()
            .make_silence_observation(10.0, Some(&ctx))
            .unwrap();
        assert_eq!(f.keys, 0);
        assert_eq!(f.bs_keys, 0);
    }

    #[test]
    fn calculate_features_pause_count_counts_2s_gaps() {
        // 3 presses at t=0, 2500, 5100 → 2 pauses (>= 2000 ms gap)
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(key_press(0x42, 2500));
        e.process_event(key_press(0x43, 5100));
        let f = e.calculate_features();
        assert!(approx_eq(f.f5_pause_count, 2.0));
    }

    #[test]
    fn calculate_features_ignores_repeats_in_flight_time() {
        // is_repeat=true のイベントは FT 計算から除外される
        let mut e = extractor();
        e.process_event(key_press(0x41, 0));
        e.process_event(key_release(0x41, 50));
        let mut repeat = key_press(0x41, 100);
        repeat.is_repeat = true;
        e.process_event(repeat);
        e.process_event(key_press(0x42, 200));
        e.process_event(key_release(0x42, 250));
        // リピート除外により FT は 200 − 50 = 150 の 1 本のみ → median = 150
        let f = e.calculate_features();
        assert!(approx_eq(f.f1_flight_time_median, 150.0));
    }
}
