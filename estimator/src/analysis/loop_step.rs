//! 分析ループの純粋関数化 (P0-1: Phase 0 Replay & Guard)
//!
//! `lib.rs` の分析スレッド本体を `step()` 関数として切り出したもの。
//! プロダクションの分析スレッドと `gse-replay` CLI の両方から呼び出される。
//!
//! # 設計
//!
//! - 本関数は `crate::input` / `tauri` に依存しない（architecture-rules ルール 6）。
//! - 時刻は `u64` ミリ秒で統一（replay が NDJSON の `t` をそのまま渡せるように）。
//!   production 側は `SystemTime::now()` ベース、replay 側は NDJSON の `t` を渡す。
//! - 戻り値は `logger::LogEntry` ではなく中立記録型 (`KeyRecord` / `FeatSnapshot`)。
//!   ログ変換は呼び出し側 (`lib.rs`) で行う。
//! - `ime_open` は引数化。production は `input::hook::IME_OPEN` を、replay は
//!   直前の `ime_state.on` を渡す。
//!
//! # 挙動不変性
//!
//! 旧 `lib.rs` 分析スレッド本体と 1:1 マップされる:
//!
//! - イベント受信 → リピート検出 (Windows のみ) → Key ログ記録 → pause 判定
//! - pause 中: `was_paused=true` にセットして 1Hz ゲートを skip
//! - buffer 追加 → pause→復帰検出 → silence-break protection → register_keystroke
//! - 1Hz ゲート: `now_ms >= next_engine_update_ms` で発火
//! - 沈黙 3 状態機構 (TypingPath / TransitionPath / SilencePath) で分岐
//! - Feat 記録
//!
//! 関連ドキュメント: `docs/architecture.md` §4.7、
//! `.steering/20260414-improvement-roadmap/design.md` §0.1。

use std::collections::HashSet;

use crate::analysis::engine::{CognitiveState, CognitiveStateEngine};
use crate::analysis::features::{FeatureExtractor, Features, InputEvent};

/// 1Hz 推論ゲートの周期 (ms)。
///
/// production は `SystemTime::now()` ベースでこの周期ごとに `step()` 内で
/// gate を発火させる。replay は仮想時刻を同じ周期で進める。
/// HMM 挙動の一部を構成するため、変更時は `engine::HMM_VERSION` を bump する。
pub const ENGINE_TICK_MS: u64 = 1000;

/// 分析ループのローカル状態。
///
/// `step()` をまたいで保持されるミュータブル状態をまとめたもの。
/// production では分析スレッドがこの構造体を 1 個所有し、
/// replay では CLI がセッション開始ごとに新規作成する。
pub struct AnalysisState {
    pub extractor: FeatureExtractor,
    /// 直近のキー入力時刻 (ms)。沈黙秒数の計算基準。
    pub last_event_time_ms: u64,
    /// 次の 1Hz ゲート発火時刻 (ms)。絶対時刻ベース増分で累積ドリフトを防止。
    pub next_engine_update_ms: u64,
    /// Silence-break protection: 10 秒沈黙後の連続打鍵カウント。
    pub presses_since_silence: u32,
    /// IME ポーズ→復帰遷移検出用。
    pub was_paused: bool,
    /// Windows リピートキー検出用 HashSet（判断 14 参照）。
    ///
    /// Windows の `WH_KEYBOARD_LL` にはリピートフラグがないため Press/Release
    /// ペアを追跡して検出する。macOS は CGEventTap の AUTOREPEAT フラグで
    /// フック層が判定するため本フィールドは空のまま使用されない。
    /// フィールド cfg 分岐は cfg 伝播の破綻リスクを避けるため適用しない。
    pub pressed_keys: HashSet<u32>,
    /// Phase 2a: 沈黙開始直前の features スナップショット。
    ///
    /// TypingPath を通るたびに上書き保存し、TransitionPath / SilencePath で
    /// 摩擦フロアの文脈として参照する（比例キャリーフォワード方式）。
    /// セッション境界 (`reset()`) で `None` にクリアする。
    pub pre_silence_features: Option<Features>,
}

impl AnalysisState {
    /// セッション開始時刻を基準に新規作成する。
    ///
    /// `now_ms` は `last_event_time_ms` と `next_engine_update_ms` の初期値。
    pub fn new(now_ms: u64, buffer_capacity: usize) -> Self {
        Self {
            extractor: FeatureExtractor::new(buffer_capacity),
            last_event_time_ms: now_ms,
            next_engine_update_ms: now_ms,
            presses_since_silence: 0,
            was_paused: false,
            pressed_keys: HashSet::new(),
            pre_silence_features: None,
        }
    }

    /// `start_session` からのリセット要求を処理する。
    pub fn reset(&mut self, now_ms: u64, buffer_capacity: usize) {
        self.extractor = FeatureExtractor::new(buffer_capacity);
        self.last_event_time_ms = now_ms;
        self.next_engine_update_ms = now_ms;
        self.presses_since_silence = 0;
        self.pressed_keys.clear();
        self.pre_silence_features = None;
        // was_paused はそのまま保持（セッション境界でも pause 状態は engine 側が管理）
    }
}

/// `step()` への入力イベント。
///
/// production 側の分析スレッドは `Key` のみを生成する。
/// replay 側の CLI は NDJSON の ime_state エントリから `ImeActivate` も生成する。
#[derive(Debug, Clone)]
pub enum LoopInput {
    /// キーストロークイベント。
    Key(InputEvent),
    /// IME 候補ウィンドウの active 状態変化。
    ///
    /// production 側の IME Monitor スレッドが直接 `engine.set_paused()` を呼ぶため、
    /// production の `step()` 呼び出しではこれを使わない。
    /// replay では NDJSON の ime_state エントリから再構成する。
    ImeActivate { active: bool },
}

/// キーストローク 1 件の中立記録型。
///
/// `lib.rs` は `LogEntry::Key` へ変換してロガーに送信、
/// `gse-replay` は NDJSON 再出力に使用する。
#[derive(Debug, Clone)]
pub struct KeyRecord {
    pub vk_code: u32,
    pub timestamp: u64,
    pub is_press: bool,
}

/// 1Hz 推論 1 サイクルの中立記録型。
///
/// ゴールデンスナップショット (P0-10) の byte-equal 比較対象。
/// `Serialize` / `Deserialize` は `gse-replay` CLI が JSON snapshot を
/// 読み書きするために付与している。フィールド名は snake_case を維持し、
/// snapshot JSON の可読性と `grep` 容易性を確保する。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FeatSnapshot {
    pub timestamp: u64,
    pub f1: f64,
    pub f3: f64,
    pub f4: f64,
    pub f5: f64,
    pub f6: f64,
    pub p_flow: f64,
    pub p_inc: f64,
    pub p_stuck: f64,
    pub raw_x: f64,
    pub raw_y: f64,
    pub ewma_x: f64,
    pub ewma_y: f64,
    pub obs: usize,
    pub alpha: f64,
    /// ★ Tier 0 (記録のみ): 特徴量ウィンドウ内の総キー押下数。★ **本フィールド群の意味の正本。**
    ///
    /// # ⚠️★★★ これは「累積カウンタ」でも「毎秒の打鍵数」でもない
    ///
    /// 窓は `FeatureExtractor::calculate_features()` のもの、すなわち
    /// **`[最後の打鍵時刻 − 30 秒, 最後の打鍵時刻]`** である
    /// (`features.rs` の `cutoff = last_ts.saturating_sub(30_000)`。★ `last_ts` は
    /// **バッファ最後のイベント時刻**であって `now_ms` ではない)。
    ///
    /// ⚠️★★★ **したがって沈黙中は同じ値が毎秒くり返し出力される。**
    /// 実測 (`tests/snapshots/deep_silence.json`): `t=…008000` から `t=…051000` までの
    /// **44 行連続で `keys=12, bs_keys=2` が不変**。打鍵ゼロの 43 秒間、同じ 12 打が毎秒書かれている。
    /// ★ これは F1〜F6 が元から持つ性質を継承したもので、本配線が作った挙動ではない。
    /// ⚠️ ただし `keys` は**率でなく個数**なので、この性質は消費者にとって危険度が高い。
    ///
    /// > ★★★ **feat 行をまたいで足し上げてはいけない。**
    /// > 沈黙のたびに同じ打鍵を数十回**二重計上**する。
    /// > ★ 窓の右端 (= 最後の打鍵時刻) が要るなら、同じ NDJSON の `key` 行から復元すること。
    ///
    /// # その他の注意
    ///
    /// - ⚠️★ **推論には一切使われない。** HMM にも X/Y 合成式にも流さないため、本フィールドを
    ///   足しても `p_*` / `raw_*` / `ewma_*` / `obs` は 1 ビットも動かない
    ///   (`.steering/20260801-recovery-axis-grounding/tier0-wiring-design.md` §2。
    ///   ★ `engine_output_is_invariant_to_tier0_key_counts` テストで実行可能な契約にしてある)。
    /// - ⚠️ **推論経路 (TypingPath / TransitionPath / SilencePath) に依存しない実測値**である。
    ///   SilencePath の合成観測は打鍵を数えないため、`step()` は `calculate_features()` 側の
    ///   値を経路に依らず載せる。
    /// - ⚠️ **リングバッファ 600 イベントに律速される** (production の容量。press+release で
    ///   1 打 2 イベントなので実質 ~300 打が上限)。BS のオートリピート等でこれを超えると
    ///   **過小計上になり、同時に窓長も 30 秒未満に切り詰められる**。
    /// - ⚠️ **IME ポーズ中の打鍵は数えない** (`step()` がバッファに入れる前に return するため)。
    ///   日本語入力主体のセッションでは系統的に過小になる。
    /// - ⚠️ `--metrics` 経路で**新フィールドを持たない旧 NDJSON** を読んだ場合は 0 が入る
    ///   (`bin/replay.rs` の `LoggedFeatDto`)。「実測 0 打」と区別できない。
    /// - ⚠️ ここに `#[serde(default)]` は **付けない**。ゴールデン (`tests/snapshots/*.json`) は
    ///   Tier 0 配線のコミットで 4 本とも再生成済みで互換の必要が無く、欠けた golden を 0 で
    ///   黙って受けるより `missing field` で落ちるほうが安全である。
    pub keys: u64,
    /// ★ Tier 0 (記録のみ): 特徴量ウィンドウ内の BS/Del 押下数。
    ///
    /// `f3` は `{:.4}` に丸めて書かれるため `f3 × keys` は整数に戻らない。丸め誤差を
    /// 持ち込まないよう整数で別に持つ (案 B)。**窓の意味と注意点は `keys` の doc がすべて。**
    pub bs_keys: u64,
}

/// `step()` の出力。
#[derive(Debug, Default)]
pub struct StepOutput {
    /// イベント処理で生成された Key レコード（最大 1 件）。
    pub key: Option<KeyRecord>,
    /// 1Hz ゲート発火時の Feat レコード（最大 1 件）。
    pub feat: Option<FeatSnapshot>,
}

/// 分析ループ 1 サイクル。
///
/// # 引数
/// - `state`: ループローカル状態（mut 借用）
/// - `engine`: HMM エンジン（内部状態は `Arc<Mutex<_>>` 経由で更新される）
/// - `input`: 今回のティックで受信したイベント（なければ `None`）
/// - `now_ms`: 現在時刻 (ms)。production は `SystemTime::now()` ベース、
///   replay は NDJSON の `t` を渡す。
/// - `ime_open`: IME モード（true: 日本語モード → β_writing、false: 英数 → β_coding）
///
/// # 戻り値
/// - `StepOutput.key`: イベントがあれば Key レコード
/// - `StepOutput.feat`: 1Hz ゲート発火時の Feat レコード
///
/// # 挙動
/// 現行 `lib.rs:510-742` と 1:1。詳細は関数内部のコメントを参照。
pub fn step(
    state: &mut AnalysisState,
    engine: &CognitiveStateEngine,
    input: Option<LoopInput>,
    now_ms: u64,
    ime_open: bool,
) -> StepOutput {
    let mut out = StepOutput::default();

    // ── イベント処理 ───────────────────────────────────────────────
    match input {
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        Some(LoopInput::Key(mut event)) => {
            // Windows: 分析スレッド側でリピート検出。
            // フックコールバックは is_repeat=false 固定で送信されるため、
            // HashSet で Press/Release ペアを追跡する。
            // CLAUDE.md 制約 (3) 準拠（フックコールバック内 Mutex 排除）。
            #[cfg(target_os = "windows")]
            {
                if event.is_press {
                    if !state.pressed_keys.insert(event.vk_code) {
                        event.is_repeat = true;
                    }
                } else {
                    state.pressed_keys.remove(&event.vk_code);
                }
            }

            out.key = Some(KeyRecord {
                vk_code: event.vk_code,
                timestamp: event.timestamp,
                is_press: event.is_press,
            });

            // IME ポーズ中はバッファ蓄積を含め全処理をスキップ。
            // unpause 時に extractor.reset() で再開するため蓄積しても無駄。
            if engine.get_paused() {
                state.was_paused = true;
                return out;
            }

            // ⚠️★★★ **ゲート分割の内側**（ADR-016 **D8**・2026-08-23）。
            //
            // ★ ここから下が「特徴量へ流す」処理である。⚠️ **`out.key` は既に上で立てて
            //   あるので、このゲートに落ちたキーも *記録* はされる**（＝ 修飾キー / F1–F12 /
            //   Esc / 矢印を「種類だけ」NDJSON に残す）。
            //
            // ⚠️★★★ **本番と `gse-replay` の両方がこの関数を通る**ので、ゲートはここ 1 か所で足りる。
            //   ★ replay 側に書き足すと 2 つの真実ができ、片方だけ直して静かにずれる。
            //
            // ⚠️ **1Hz ゲート（下の `next_engine_update_ms` の節）へは落とさない** ——
            //   ★ `return` すると観測専用キーが来たティックで feat の発火が 1 回遅れる。
            //   ⚠️ それは「特徴量に流さない」ではなく「**出力のタイミングを変える**」であり、
            //   本ゲートの約束（特徴量は 1 バイトも変わらない）を破る。
            if crate::analysis::key_gate::is_feature_key(event.vk_code) {
                state.extractor.process_event(event);

                // ポーズ→復帰遷移: バッファをクリアして stale なフライトタイムを排除。
                if state.was_paused {
                    state.was_paused = false;
                    state.extractor.reset();
                    state.last_event_time_ms = now_ms;
                    state.presses_since_silence = 0;
                    tracing::debug!("IME pause ended — buffer cleared for clean restart");
                }

                if event.is_press {
                    // Silence-break protection (non-repeat only):
                    // 10 秒以上の沈黙後、3 打鍵以上の連続入力で初めて
                    // last_event_time をリセットし「タイピング復帰」と判定する。
                    if !event.is_repeat {
                        let elapsed_ms = now_ms.saturating_sub(state.last_event_time_ms);
                        if elapsed_ms > 10_000 {
                            state.presses_since_silence += 1;
                            if state.presses_since_silence >= 3 {
                                state.last_event_time_ms = now_ms;
                                state.presses_since_silence = 0;
                                // Phase 2a: 長沈黙からの復帰時に古い文脈をクリア
                                state.pre_silence_features = None;
                            }
                        } else {
                            state.last_event_time_ms = now_ms;
                            state.presses_since_silence = 0;
                        }
                    }

                    // Backspace ストリークカウント（1Hz gate の外側で全打鍵）
                    // リピートイベントも含める: BS 長押しによる一気消しでペナルティを発動させる
                    engine.register_keystroke(event.is_backspace);
                }
            }
        }
        Some(LoopInput::ImeActivate { active: _ }) => {
            // ⚠️★★★ **2026-08-23: ここでの `set_paused` を外した (研究者判断)。**
            //
            // ★ 旧コードは `engine.set_paused(active)` で NDJSON の `ime_state.active`
            //   から pause を再構成していた。⚠️ **live 側で pause をやめたので、
            //   ここに残すと replay だけが止まり Replay-as-Truth が壊れる。**
            //
            // ★ なぜ live で pause をやめたか: 実測で `IME_ACTIVE` は
            //   「候補リスト表示」ではなく **変換セッション全体** を覆い、
            //   ⚠️ pause すると日本語論述の推定窓が約 56〜60% 消えるため
            //   (`lib.rs` の IME Monitor スレッドのコメント / decisions D-10)。
            //
            // ⚠️★ **バリアント自体は残す** —— `ime_state` 行は読み飛ばさず、
            //   将来 pause を復活させるならここに戻す、という位置を保つ。
        }
        None => {}
    }

    // ── 独立 1Hz ゲート ──────────────────────────────────────────────
    if now_ms < state.next_engine_update_ms {
        return out;
    }

    // 絶対時刻ベースインクリメント (累積ドリフト防止)。
    // スレッドスタール後のキャッチアップ: 大幅に遅れた場合は現在時刻にリセット。
    state.next_engine_update_ms += ENGINE_TICK_MS;
    if state.next_engine_update_ms < now_ms {
        state.next_engine_update_ms = now_ms + ENGINE_TICK_MS;
    }

    if engine.get_paused() {
        state.was_paused = true;
        return out;
    }

    // 1Hz ゲートでのポーズ→復帰遷移検出（タイムアウトパス）
    if state.was_paused {
        state.was_paused = false;
        state.extractor.reset();
        state.last_event_time_ms = now_ms;
        state.presses_since_silence = 0;
        tracing::debug!("IME pause ended (1Hz gate) — buffer cleared for clean restart");
    }

    let silence_secs = (now_ms.saturating_sub(state.last_event_time_ms)) as f64 / 1000.0;
    let mut features = state.extractor.calculate_features();

    // ★ Tier 0 (記録のみ): 打鍵数は**推論経路に依らず実測値**を記録する。
    // SilencePath は合成観測 (`make_silence_observation`) を作って `log_features` に載せるが、
    // 合成観測は打鍵を数えていない。ここで実測値を退避し、下の Feat 記録でそのまま使う。
    // TransitionPath が `features` の f3/f5/f6 を書き換えても本値は影響を受けない。
    let (window_keys, window_bs_keys) = (features.keys, features.bs_keys);

    // ── 沈黙 3 状態機構 ──────────────────────────────────────────────
    // 詳細は docs/architecture.md §4.7。
    let (log_features, diag_opt) = if features.f1_flight_time_median > 0.0 && silence_secs < 2.0 {
        // === TypingPath: バッファ実データあり + 直近タイピング中 ===
        // Phase 2a: 沈黙用の文脈をキャプチャ（比例キャリーフォワード方式）
        state.pre_silence_features = Some(features);
        let d = engine.update(&features, ime_open);
        (features, d)
    } else if features.f1_flight_time_median > 0.0 {
        // === TransitionPath: バッファ実データあり + 2 秒以上沈黙中 ===
        // α=0.3 で古いウィンドウデータを連続入力すると Incubation にロックインするため、
        // α=0.15 の update_silence を使用する。
        if silence_secs >= 2.0 {
            features.f5_pause_count += silence_secs / 2.0;
        }
        // Phase 2a: 文脈ベース摩擦フロア（比例キャリーフォワード方式）
        // 直前 TypingPath の F3/F6 を文脈として使い、時間に比例して増幅する。
        // 文脈なし時は従来の時間ベースランプにフォールバック。
        // TUNE: TransitionPath onset 8s / ramp 30s — 線形式 (ctx × time_scale)。
        //   SilencePath (features.rs) は onset 2s の非線形式を使う。
        //   TransitionPath はバッファに実データがあるため onset を遅くし、
        //   バッファ値の F3/F6 を補完するフロアとして線形に適用する。
        if silence_secs >= 8.0 {
            if let Some(ref ctx) = state.pre_silence_features {
                let time_scale = ((silence_secs - 8.0) / 30.0).clamp(0.0, 1.0);
                let f6_floor = ctx.f6_pause_after_del_rate.clamp(0.0, 1.0) * time_scale;
                let f3_floor = ctx.f3_correction_rate.clamp(0.0, 1.0) * time_scale;
                features.f6_pause_after_del_rate = features.f6_pause_after_del_rate.max(f6_floor);
                features.f3_correction_rate = features.f3_correction_rate.max(f3_floor);
            } else {
                // フォールバック: 従来の時間ベースランプ
                let mut f6_floor = ((silence_secs - 8.0) / 30.0).clamp(0.0, 1.0);
                let mut f3_floor = ((silence_secs - 10.0) / 40.0).clamp(0.0, 1.0);
                if silence_secs >= 10.0 {
                    f6_floor = f6_floor.max(0.15);
                    f3_floor = f3_floor.max(0.20);
                }
                features.f6_pause_after_del_rate = features.f6_pause_after_del_rate.max(f6_floor);
                features.f3_correction_rate = features.f3_correction_rate.max(f3_floor);
            }
        }
        let d = engine.update_silence(&features, ime_open, silence_secs);
        (features, d)
    } else if let Some(sf) = state
        .extractor
        .make_silence_observation(silence_secs, state.pre_silence_features.as_ref())
    {
        // === SilencePath: バッファ空 → 合成特徴量で update_silence ===
        let d = engine.update_silence(&sf, ime_open, silence_secs);
        (sf, d)
    } else {
        // silence < 2s かつ f1=0 (データ不足) → スキップ
        return out;
    };

    // ── Feat 記録 ────────────────────────────────────────────────────
    let state_probs = engine.get_current_state();
    let p_flow = state_probs
        .get(&CognitiveState::Flow)
        .copied()
        .unwrap_or(0.0);
    let p_inc = state_probs
        .get(&CognitiveState::Incubation)
        .copied()
        .unwrap_or(0.0);
    let p_stuck = state_probs
        .get(&CognitiveState::Stuck)
        .copied()
        .unwrap_or(0.0);

    let (raw_x, raw_y, ewma_x, ewma_y, obs, alpha) = match &diag_opt {
        Some(d) => (d.raw_x, d.raw_y, d.ewma_x, d.ewma_y, d.obs, d.alpha),
        None => (0.0, 0.0, 0.0, 0.0, 0, 0.0),
    };

    out.feat = Some(FeatSnapshot {
        timestamp: now_ms,
        f1: log_features.f1_flight_time_median,
        f3: log_features.f3_correction_rate,
        f4: log_features.f4_burst_length,
        f5: log_features.f5_pause_count,
        f6: log_features.f6_pause_after_del_rate,
        p_flow,
        p_inc,
        p_stuck,
        raw_x,
        raw_y,
        ewma_x,
        ewma_y,
        obs,
        alpha,
        // ★ Tier 0: `log_features` ではなく実測値を使う (SilencePath の合成値を書かないため)。
        keys: window_keys,
        bs_keys: window_bs_keys,
    });

    out
}

// ============================================================================
// Unit tests
// ============================================================================
//
// ⚠️★ 本モジュールは長らく無テストで、ゴールデン (`gse-replay --snapshot`) だけが
// 回帰保護だった。★ Tier 0 配線 (2026-08-16) で「推論経路に依らず実測打鍵数を書く」という
// 判断を入れた際、**現行ゴールデン 4 本ではその判断が観測できない**ことが code-reviewer の
// 指摘で判明したため、判断を直接固定するテストをここに置く。
//   - TypingPath / TransitionPath: `log_features == features` なので両者は一致し区別できない
//   - 既存ゴールデンの SilencePath 行はバッファが空で `keys=0` なのでやはり区別できない
// → ★ **バッファが空でない SilencePath** を作ると初めて差が出る。

#[cfg(test)]
mod tests {
    use super::*;

    fn press(vk: u32, t: u64, is_backspace: bool) -> InputEvent {
        InputEvent {
            vk_code: vk,
            timestamp: t,
            is_press: true,
            is_repeat: false,
            is_backspace,
            is_paste: false,
        }
    }

    fn release(vk: u32, t: u64) -> InputEvent {
        InputEvent {
            vk_code: vk,
            timestamp: t,
            is_press: false,
            is_repeat: false,
            is_backspace: false,
            is_paste: false,
        }
    }

    /// ★★★ 本配線の中核判断の回帰保護。
    ///
    /// **SilencePath はバッファ空を意味しない。** 分岐条件は `f1 == 0.0` であり、
    /// 打鍵があってもフライトタイムが 1 つも採れなければ (ここでは全 FT ≥ 2000ms で
    /// `features.rs` の上限キャップに弾かれる) `f1 == 0.0` になる。
    /// このとき合成観測 (`make_silence_observation`) は `keys = 0` を返すので、
    /// ⚠️ `log_features.keys` をそのまま書くと **実際に打鍵があるのに 0 と記録される**。
    #[test]
    fn silence_path_with_nonempty_buffer_records_measured_key_counts() {
        let engine = CognitiveStateEngine::new();
        let mut state = AnalysisState::new(0, 600);

        // FT がすべて 2000ms 以上になるよう press/release を離す → window_fts が空 → f1 = 0。
        // BS を 1 打混ぜて bs_keys も 0 でないことを確認する。
        for (i, (vk, is_bs)) in [(0x41, false), (0x08, true), (0x42, false)]
            .into_iter()
            .enumerate()
        {
            let t = i as u64 * 2_500;
            state.extractor.process_event(press(vk, t, is_bs));
            state.extractor.process_event(release(vk, t + 10));
        }

        // 前提の確認: この状態が本当に f1 == 0 かつ打鍵ありであること。
        let measured = state.extractor.calculate_features();
        assert_eq!(
            measured.f1_flight_time_median, 0.0,
            "前提が崩れている: f1 != 0 だと SilencePath に入らない"
        );
        assert_eq!((measured.keys, measured.bs_keys), (3, 1));

        // 2 秒以上の沈黙にして 1Hz ゲートを踏ませる (last_event_time_ms は 0 のまま)。
        let out = step(&mut state, &engine, None, 10_000, false);
        let feat = out.feat.expect("1Hz ゲートが発火して feat が出るはず");

        // SilencePath を通ったこと (合成観測の F1 = 2000.0) を確認したうえで…
        assert_eq!(feat.f1, 2000.0, "SilencePath を通っていない");
        // ★★★ …打鍵数は合成観測の 0 ではなく実測値であること。
        assert_eq!(
            (feat.keys, feat.bs_keys),
            (3, 1),
            "SilencePath で合成観測の keys=0 が書かれている (実測値の退避が効いていない)"
        );
    }

    /// バッファが空の SilencePath では `keys = 0` が正しい (実測 0 打)。
    #[test]
    fn silence_path_with_empty_buffer_records_zero_key_counts() {
        let engine = CognitiveStateEngine::new();
        let mut state = AnalysisState::new(0, 600);

        let out = step(&mut state, &engine, None, 10_000, false);
        let feat = out.feat.expect("feat が出るはず");
        assert_eq!(feat.f1, 2000.0, "SilencePath を通っていない");
        assert_eq!((feat.keys, feat.bs_keys), (0, 0));
    }

    /// TypingPath でも実測値がそのまま載ること (経路によらず同じ意味の値であること)。
    #[test]
    fn typing_path_records_measured_key_counts() {
        let engine = CognitiveStateEngine::new();
        let mut state = AnalysisState::new(0, 600);

        // FT < 2000ms のペアを作って f1 > 0 にする。
        for (i, (vk, is_bs)) in [(0x41, false), (0x08, true), (0x42, false), (0x43, false)]
            .into_iter()
            .enumerate()
        {
            let t = i as u64 * 200;
            state.extractor.process_event(press(vk, t, is_bs));
            state.extractor.process_event(release(vk, t + 50));
        }
        state.last_event_time_ms = 700; // 沈黙 < 2s にして TypingPath へ

        let out = step(&mut state, &engine, None, 1_000, false);
        let feat = out.feat.expect("feat が出るはず");
        assert!(
            feat.f1 > 0.0 && feat.f1 < 2000.0,
            "TypingPath を通っていない"
        );
        assert_eq!((feat.keys, feat.bs_keys), (4, 1));
    }

    /// ★★★ **契約を実行可能にするテスト**:
    /// `keys` / `bs_keys` **以外が同一**の 2 つの `Features` を HMM に通し、
    /// 出力 (`raw_x` / `raw_y` / `ewma_x` / `ewma_y` / `obs` / `p_*`) が
    /// **bit-identical** であることを確認する。
    ///
    /// ⚠️ これが落ちたら `engine.rs` が Tier 0 の値を読み始めたということであり、
    /// 「記録するだけで推論に流さない」という契約が破れている
    /// (`.steering/20260801-recovery-axis-grounding/tier0-wiring-design.md` §2)。
    #[test]
    fn engine_output_is_invariant_to_tier0_key_counts() {
        let base = Features {
            f1_flight_time_median: 180.0,
            f3_correction_rate: 0.25,
            f4_burst_length: 3.0,
            f5_pause_count: 2.0,
            f6_pause_after_del_rate: 0.4,
            keys: 0,
            bs_keys: 0,
        };
        let loud = Features {
            keys: 9_999,
            bs_keys: 4_321,
            ..base
        };

        let run = |f: &Features| {
            let engine = CognitiveStateEngine::new();
            let d = engine.update(f, false).expect("diagnostics");
            let probs = engine.get_current_state();
            (
                d.raw_x.to_bits(),
                d.raw_y.to_bits(),
                d.ewma_x.to_bits(),
                d.ewma_y.to_bits(),
                d.obs,
                d.alpha.to_bits(),
                probs[&CognitiveState::Flow].to_bits(),
                probs[&CognitiveState::Incubation].to_bits(),
                probs[&CognitiveState::Stuck].to_bits(),
            )
        };

        assert_eq!(
            run(&base),
            run(&loud),
            "Tier 0 の打鍵数が HMM 出力を動かしている = 記録と推論の分離が破れている"
        );
    }
}
