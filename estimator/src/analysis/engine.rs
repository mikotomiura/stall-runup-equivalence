use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::analysis::features::Features;
use crate::analysis::heads::{head_for, DomainInferenceHead, HeadId, TaskDomain};

// ============================================================================
// TUNE: HMM チューニング定数
// ============================================================================
//
// 意図的な HMM チューニングでこれらを変更する際は、
//   1. `#[cfg(test)] mod tests` のスナップショットテストも更新する
//   2. 変更根拠を `.steering/` に記録する
// これら以外の α 値・閾値もソース内に散在する。
// 全チューニングポイントの列挙は `grep -rn "TUNE:" src-tauri/src/` で確認可能
// （`analysis/` 以外に `lib.rs` の TransitionPath 摩擦フロアも含む）。
//
// **EWMA α 系の定数化ポリシー**:
//   通常 update() の α=0.3、update_silence() の 0.15/0.18/0.25、
//   ヒステリシス層の 0.40/0.60 はモジュール定数として抽出しない。
//   理由: IEEE 754 において `(1.0 - α)` はリテラル相補値と bit-equal に
//   ならない（例: `1.0 - 0.3 ≠ 0.7`）。定数化すると compute 経路の
//   浮動小数点結果が ULP レベルで変わり、既存セッションログの再現性を
//   損なう。これらは `// TUNE:` マーカーコメントで列挙可能にするのみとする。

/// HMM エンジンの挙動バージョン。
///
/// **このファイル内の以下の変更では必ず bump すること**:
/// - `INITIAL_PROBS` / `INITIAL_AXES_EWMA` / `BACKSPACE_PENALTY_THRESHOLD`
/// - `transition_matrix` / `emission_matrix` / `initial_probabilities` の値
/// - `update()` / `update_silence()` の内部 α 値・ヒステリシス α
/// - `update_silence()` の silence_secs → α 閾値
///
/// また `analysis/features.rs` の `phi` / `make_silence_observation` /
/// `calculate_features` の数値挙動変更、`analysis/loop_step.rs` の摩擦フロア
/// 変更も bump 対象。
///
/// SemVer 運用:
///
/// - MAJOR: 既存セッションログの確率列が大幅に変わる（状態優勢の逆転等）
/// - MINOR: 確率列は動くが同じラベル付けになる範囲の調整
/// - PATCH: ログ形式のみの変更（数値挙動不変）
/// - ★ MINOR: **既存ログの確率列は 1 ビットも変わらないが、新しい入力クラス
///   （`task_kind` 等）に対して挙動の分岐条件が増える場合**
///   （2026-08-18 追加。上の 3 つは「既存ログ」基準で書かれており、
///   「まだ存在しない入力クラスにだけ挙動が出る」ケースを覆っていなかった。
///   根拠は `.steering/20260818-adr006-head-extraction-wiring/decisions.md` D-4）
///
/// `logger.rs` の session_start meta エントリに書き出され、
/// `gse-replay` がゴールデンスナップショットと突き合わせる際に検証する。
///
/// # 変更履歴
///
/// - `0.4.0` (2026-08-18): ADR-006 Phase 2b。`task_kind` が domain head を選ぶようになり、
///   `vibe_coding` セッションが `ime_open` に関わらず Coding head の β で採点される。
///   ⚠️ **既存ログ（全て `argumentative`）の確率列は変わらない**ためゴールデン 4 本は
///   byte-equal のまま。挙動が変わるのは `task_kind=vibe_coding` の入力のみ。
pub const HMM_VERSION: &str = "0.5.0";

/// TUNE: 初期事前確率 [Flow, Incubation, Stuck]
///
/// Flow 優勢で開始することで、セッション開始直後のサイレンス観測（f1=2000）
/// による過渡的な Incubation 方向への引き込みを吸収する。
/// HMM は数秒以内に実データに収束する。
const INITIAL_PROBS: [f64; 3] = [0.80, 0.15, 0.05];

/// TUNE: 潜在軸 (X=Friction, Y=Engagement) の初期 EWMA 値
///
/// (0.1, 0.8) = Flow 領域 (obs=4 付近、Flow 優勢ビン)。
/// `INITIAL_PROBS` と整合的な開始位置。
const INITIAL_AXES_EWMA: (f64, f64) = (0.1, 0.8);

/// TUNE: Backspace 連打によるペナルティビン (obs=25) 発動閾値
///
/// IME の正常な変換修正操作（通常 8-12 連打）を超える値として 14 を設定。
/// この値以上で `has_pending_penalty` フラグが立ち、次回 `update()` で
/// observation を obs=25 に強制し、Stuck 方向へ強く引く。
const BACKSPACE_PENALTY_THRESHOLD: u32 = 14;

/// HMM 更新時の内部診断データ。
/// ログに書き出してセッション後分析・HMM チューニングに使用する。
#[derive(Debug, Clone)]
pub struct UpdateDiagnostics {
    pub raw_x: f64,
    pub raw_y: f64,
    pub ewma_x: f64,
    pub ewma_y: f64,
    pub obs: usize,
    pub alpha: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CognitiveState {
    Flow,
    Incubation,
    Stuck,
}

#[derive(Clone)]
pub struct CognitiveStateEngine {
    // Manual HMM parameters
    transitions: Arc<[f64; 9]>,

    // 3 states × 26 observation bins
    //   obs = x_bin * 5 + y_bin  (0..24 natural bins)
    //   obs = 25                  (backspace streak penalty bin)
    //
    // X-axis (Friction):   0 = low friction  … 4 = high friction
    // Y-axis (Engagement): 0 = low engagement … 4 = high engagement
    emissions: Arc<[f64; 78]>,

    current_state_probs: Arc<Mutex<[f64; 3]>>,
    pub is_paused: Arc<AtomicBool>,
    pub backspace_streak: Arc<AtomicU32>,
    // ペナルティ保留フラグ: streak >= 14 に達した瞬間にtrueになり、
    // update() で消費されるまで保持される。
    // register_keystroke() で streak をリセットしてもペナルティは取りこぼさない。
    has_pending_penalty: Arc<AtomicBool>,

    // 2-axis EWMA: (X = Friction, Y = Engagement)
    // α = 0.3: 新値30%、前値70%のブレンド
    axes_ewma: Arc<Mutex<(f64, f64)>>,

    // Hysteresis layer: slow EMA of reported probabilities.
    // Prevents instant state flips (e.g. Cold-Start after window reset).
    // α = 0.40 for normal updates (~2.5s time-constant).
    // α = 0.60 for backspace-penalty bin (faster Stuck response).
    display_probs: Arc<Mutex<[f64; 3]>>,

    // ADR-006 Alternative C: β baseline は「1 関数内の 2 択 if」ではなく
    // **head の所有物**である。どの head を走らせるかは task_kind (宣言) が決め、
    // ime_open (観測) は Writing head 内部の within-writing 信号として使う。
    // task_kind 不明時 (既存 Phase1 ログ等) は Writing head = Phase1 と完全に同じ挙動。
    //
    // ⚠️ **Mutex で包む理由**: engine はアプリ起動時に 1 個だけ作られ、Tauri managed State と
    //   分析スレッドで Arc 共有される (lib.rs)。セッション開始のたびに作り直す経路が無く、
    //   task_kind は start_session の**後**に record_session_meta で届くため、
    //   構築時に固定すると本番では永久に Writing head のままになる。
    head: Arc<Mutex<Arc<dyn DomainInferenceHead>>>,
}

impl Default for CognitiveStateEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CognitiveStateEngine {
    /// 既定 (Writing head) で engine を作る。
    ///
    /// Phase1 は `task_kind = argumentative` 固定だったのでこれが唯一の経路だった。
    /// `task_kind` が分かる場合は [`Self::new_for_domain`] を使う。
    pub fn new() -> Self {
        Self::new_for_domain(TaskDomain::Writing)
    }

    /// 走らせる domain head を指定して engine を作る (ADR-006 Phase 2b)。
    ///
    /// HMM 本体 (遷移行列・放出行列・初期確率) は domain 非依存で共通。
    /// domain によって変わるのは **β baseline = 潜在二軸の算出**だけである。
    pub fn new_for_domain(domain: TaskDomain) -> Self {
        // TUNE: 遷移確率行列 (row-major, 3×3)
        // FLOW -> FLOW: 0.75  (escape time 1/(1-0.75)=4s; reduced from 0.80 to mitigate Flow Gravity)
        // FLOW -> INCUBATION: 0.17
        // FLOW -> STUCK: 0.08
        // INCUBATION -> FLOW: 0.12
        // INCUBATION -> INCUBATION: 0.80  (Sio & Ormerod 2009)
        // INCUBATION -> STUCK: 0.08
        // STUCK -> FLOW: 0.06
        // STUCK -> INCUBATION: 0.18
        // STUCK -> STUCK: 0.76  (Hall et al. 2024)
        let transitions = [0.75, 0.17, 0.08, 0.12, 0.80, 0.08, 0.06, 0.18, 0.76];

        // TUNE: 放出確率行列 B (3 states × 26 bins)
        //
        // Grid layout: obs = x_bin * 5 + y_bin
        //
        //        x→  0(lo F)  1      2      3      4(hi F)
        //  y↓
        //  0(lo E)   [0]     [5]   [10]   [15]   [20]
        //  1         [1]     [6]   [11]   [16]   [21]
        //  2         [2]     [7]   [12]   [17]   [22]
        //  3         [3]     [8]   [13]   [18]   [23]
        //  4(hi E)   [4]     [9]   [14]   [19]   [24]
        //  penalty   [25]
        //
        // Flow:       peaks at low Friction (x=0,1) × high Engagement (y=3,4)
        // Incubation: peaks at low-mid Friction (x=0,1,2) × low Engagement (y=0,1)
        // Stuck:      peaks at high Friction (x=3,4) × low Engagement (y=0,1)
        //
        // Penalty bin (obs=25): backspace_streak ≥ 14 → near-certain Stuck.
        //
        // 全ビンに最小値 0.01 を設定し、確率の完全消滅を防止。
        // 旧実装の EMISSION_FLOOR (+0.05 一律加算) を廃止:
        //   - 旧: ペナルティビン Stuck:Flow 比 = 1.04/0.05 = 20.8x（鈍い応答）
        //   - 新: ペナルティビン Stuck:Flow 比 = 0.99/0.01 = 99x（鋭い応答）
        // EWMA (α=0.3) とヒステリシス層 (α=0.25/0.50) が安定性を維持する。
        #[rustfmt::skip]
        let emissions: [f64; 78] = [
            // ── Flow (State 0) ─────────────────────────── non-penalty sum ≈ 1.03
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.01, 0.02, 0.08, 0.12, 0.14,
            //  x=1            y: 0     1     2     3     4
                               0.01, 0.02, 0.08, 0.12, 0.13,
            //  x=2            y: 0     1     2     3     4
                               0.01, 0.01, 0.02, 0.01, 0.08,
            //  x=3            y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  penalty bin
                               0.01,

            // ── Incubation (State 1) ──────────────────── non-penalty sum ≈ 1.12
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.15, 0.10, 0.04, 0.03, 0.02,
            //  x=1            y: 0     1     2     3     4
                               0.14, 0.08, 0.04, 0.03, 0.02,
            //  x=2            y: 0     1     2     3     4
                               0.10, 0.08, 0.03, 0.02, 0.01,
            //  x=3            y: 0     1     2     3     4
                               0.05, 0.04, 0.01, 0.01, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.04, 0.03, 0.01, 0.01, 0.01,
            //  penalty bin
                               0.01,

            // ── Stuck (State 2) ─────────────────────────
            //  x=0 (low F)    y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=1            y: 0     1     2     3     4
                               0.01, 0.01, 0.01, 0.01, 0.01,
            //  x=2            y: 0     1     2     3     4
                               0.02, 0.04, 0.04, 0.05, 0.01,
            //  x=3            y: 0     1     2     3     4
                               0.10, 0.16, 0.05, 0.07, 0.01,
            //  x=4 (high F)   y: 0     1     2     3     4
                               0.16, 0.22, 0.12, 0.05, 0.02,
            //  penalty bin  (backspace streak ≥14 → near-certain Stuck)
                               0.99,
        ];

        Self {
            transitions: Arc::new(transitions),
            emissions: Arc::new(emissions),
            current_state_probs: Arc::new(Mutex::new(INITIAL_PROBS)),
            is_paused: Arc::new(AtomicBool::new(false)),
            backspace_streak: Arc::new(AtomicU32::new(0)),
            has_pending_penalty: Arc::new(AtomicBool::new(false)),
            axes_ewma: Arc::new(Mutex::new(INITIAL_AXES_EWMA)),
            // display_probs は INITIAL_PROBS と同値で初期化
            display_probs: Arc::new(Mutex::new(INITIAL_PROBS)),
            // ADR-006 2b: task_kind (宣言) が選んだ head を積む。
            head: Arc::new(Mutex::new(head_for(domain))),
        }
    }

    /// 現在の head を取り出す (Arc の clone なので安い)。
    ///
    /// A-3 と同じく Mutex ポイズニングでも停止しない (`into_inner` で続行する)。
    fn head(&self) -> Arc<dyn DomainInferenceHead> {
        match self.head.lock() {
            Ok(h) => Arc::clone(&h),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// この engine がどの head で採点しているか。`session_meta` に刻むために使う。
    pub fn head_id(&self) -> HeadId {
        self.head().head_id()
    }

    /// 走らせる domain head を差し替える (ADR-006 Phase 2b)。
    ///
    /// `record_session_meta` (= 宣言された `task_kind` が届く場所) から呼ぶ。
    ///
    /// ⚠️ **`start_session` → `record_session_meta` の順で呼ばれるため、両者の間に挟まる
    /// 1Hz tick は直前の `reset()` が戻した Writing head で採点される。**
    /// `App.tsx` は `start_session` 成功直後に `record_session_meta` を invoke するので
    /// 実測上この窓は 1 秒未満だが、ゼロではない。窓を消すにはフロントの呼び出し順を
    /// 変えるか、`start_session` 自体に task_kind を持たせる必要がある (本タスクの射程外)。
    /// 戻り値は**反映後の** `head_id`。呼び出し側が `set_domain` → `head_id()` と
    /// 2 回ロックを取ると、その隙間に別の差し替えが入って「記録した head_id と
    /// 実際に走る head が違う」状態になりうるため、1 回で返す
    /// (security-checker 指摘・2026-08-18)。
    pub fn set_domain(&self, domain: TaskDomain) -> HeadId {
        let next = head_for(domain);
        let id = next.head_id();
        match self.head.lock() {
            Ok(mut h) => *h = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
        id
    }

    /// HMM確率・EWMA・backspace_streakを初期値に戻す。
    /// セッション開始時に呼び出され、前回セッションの状態をリセットする。
    pub fn reset(&self) {
        match self.current_state_probs.lock() {
            Ok(mut p) => *p = INITIAL_PROBS,
            Err(poisoned) => *poisoned.into_inner() = INITIAL_PROBS,
        }
        match self.display_probs.lock() {
            Ok(mut p) => *p = INITIAL_PROBS,
            Err(poisoned) => *poisoned.into_inner() = INITIAL_PROBS,
        }
        match self.axes_ewma.lock() {
            Ok(mut e) => *e = INITIAL_AXES_EWMA,
            Err(poisoned) => *poisoned.into_inner() = INITIAL_AXES_EWMA,
        }
        self.backspace_streak.store(0, Ordering::Relaxed);
        self.has_pending_penalty.store(false, Ordering::Release);
        self.is_paused.store(false, Ordering::Release);
        // ⚠️ head も既定 (Writing) に戻す。engine は起動時 1 個をセッション間で使い回すため、
        //   戻さないと「前のセッションが vibe_coding だった」だけで次のセッションが
        //   Coding head で採点され続ける (task_kind 未宣言のセッションで顕在化する)。
        let _ = self.set_domain(TaskDomain::Writing);
    }

    pub fn set_paused(&self, paused: bool) {
        self.is_paused.store(paused, Ordering::Release);
    }

    /// 全キー押下時に呼び出し、Backspaceストリークをリアルタイムでカウントする。
    /// 1Hz gate の外側（全打鍵）で呼ぶことで、高速Backspace連打を正確に検知する。
    ///
    /// streak >= 14 に達した時点で has_pending_penalty フラグを立てる。
    /// 閾値14はIMEの正常操作範囲（8-12連打）を超えた連打のみ検出する。
    /// 非BSキーで streak がリセットされても、フラグは update() で消費されるまで保持される。
    /// これにより「BS×12 → 即Enter」のようなケースでもペナルティを取りこぼさない。
    ///
    /// `is_backspace` の判定は呼び出し元（OS固有のフック処理）で行う。
    /// Windows: vk_code == 0x08 (VK_BACK), macOS: keycode == 0x33 (kVK_Delete)
    pub fn register_keystroke(&self, is_backspace: bool) {
        if is_backspace {
            let new_streak = self.backspace_streak.fetch_add(1, Ordering::Relaxed) + 1;
            if new_streak >= BACKSPACE_PENALTY_THRESHOLD {
                self.has_pending_penalty.store(true, Ordering::Release);
            }
        } else {
            self.backspace_streak.store(0, Ordering::Relaxed);
        }
    }

    /// IMEポーズ中かどうかを安全に取得する
    pub fn get_paused(&self) -> bool {
        self.is_paused.load(Ordering::Acquire)
    }

    /// B-5: HMM Update (2軸 Friction × Engagement、25+1ビン モデル)
    ///
    /// `ime_open`: true = Japanese IME is in Japanese-input mode (あ/カ).
    /// Head 内部で context-appropriate な β の選択に使われる
    /// (Writing head: β_writing / β_writing_latin の切替)。
    pub fn update(&self, features: &Features, ime_open: bool) -> Option<UpdateDiagnostics> {
        if self.get_paused() {
            return None;
        }

        // F1がゼロ（データ不足）の場合はスキップ。
        // ただし保留中のペナルティがある場合は処理を続行し、ペナルティビンを適用する。
        if features.f1_flight_time_median <= 0.0
            && !self.has_pending_penalty.load(Ordering::Acquire)
        {
            return None;
        }

        // --- Backspace Streak Logic ---
        // register_keystroke() が streak >= 14 到達時に has_pending_penalty を true に設定済み。
        // ここではフラグを消費（swap false）し、ペナルティビン（obs=25）を適用する。
        // IMEガード: ime_open=true 時はペナルティを無効化。IME変換修正のBS連打は
        // 認知的Stuckではなく正常操作のため。
        // ⚠️ head は **1 tick に 1 回だけ**取る。2 回に分けて取ると、その隙間に
        //   `set_domain` が入ったとき BS ペナルティ gate と潜在二軸が**別 head 由来**になる
        //   (security-checker 指摘・2026-08-18)。
        let head = self.head();

        let pending = self.has_pending_penalty.swap(false, Ordering::AcqRel);
        let apply_backspace_penalty = pending && head.applies_backspace_penalty(ime_open);

        let (raw_x, raw_y) = head.latent_axes(features, ime_open);

        // TUNE: EWMA 平滑化 α = 0.3 (通常 update)
        // s_t = 0.3 * raw_t + 0.7 * s_{t-1}
        // 注: IEEE 754 の (1.0 - 0.3) ≠ 0.7 のため、定数抽出せずリテラル維持。
        let (x, y) = match self.axes_ewma.lock() {
            Ok(mut ewma) => {
                ewma.0 = 0.3 * raw_x + 0.7 * ewma.0;
                ewma.1 = 0.3 * raw_y + 0.7 * ewma.1;
                (ewma.0, ewma.1)
            }
            Err(poisoned) => {
                let mut ewma = poisoned.into_inner();
                ewma.0 = 0.3 * raw_x + 0.7 * ewma.0;
                ewma.1 = 0.3 * raw_y + 0.7 * ewma.1;
                (ewma.0, ewma.1)
            }
        };

        // (X, Y) → 5×5 グリッド → 観測インデックス [0..24]
        let x_bin = (x * 5.0).floor().min(4.0) as usize;
        let y_bin = (y * 5.0).floor().min(4.0) as usize;
        let mut obs = x_bin * 5 + y_bin;

        // Backspace Streak Override → ペナルティビン (obs=25)
        // 高Friction・低Engagementの極端ケースとして強制マップ
        if apply_backspace_penalty {
            obs = 25;
        }

        // A-3: Mutex ポイズニング対策
        let mut current = match self.current_state_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let old_probs = *current;
        let mut new_probs = [0.0; 3];
        // ⚠️★★★ **配列長から取ること** (2026-09-12・code-reviewer MEDIUM)。
        //   ★ 旧 `for j in 0..n_states` は `n_states` を増やすと `new_probs[j]` で**即パニック**した。
        //   ⚠️ `iter_mut()` にした今は**配列長どおり静かに 3 回しか回らない**ので、
        //     `n_states` だけを直すと `transitions[i * n_states + j]` のストライドがずれ、
        //     **無音で誤った確率**を返す。→ ★ 両者を 1 つの真実 (配列長) に縛る。
        let n_states = new_probs.len();
        debug_assert_eq!(old_probs.len(), n_states);

        // Forward Algorithm Step
        // 放射確率は emission テーブルに最小値 0.01 を組み込み済み。
        // 旧 EMISSION_FLOOR (+0.05 一律加算) は廃止。
        let mut sum_prob = 0.0;

        // ⚠️★★★ **加算の順序を変えないこと** (2026-09-12)。
        //   ★ 浮動小数の加算は結合則を満たさないので、`trans_sum` / `sum_prob` の
        //     足し込む順番を変えると**確率列の下位ビットが動き、ゴールデンが割れる**。
        //   → ★ `iter().sum()` や `fold` には書き換えない。`enumerate()` で添字を取るだけに留める。
        for (j, new_prob) in new_probs.iter_mut().enumerate() {
            let mut trans_sum = 0.0;
            for (i, &old_prob) in old_probs.iter().enumerate() {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_prob * t_prob;
            }

            // 3 states × 26 bins: index = j * 26 + obs
            let e_prob = self.emissions[j * 26 + obs];
            *new_prob = trans_sum * e_prob;
            sum_prob += *new_prob;
        }

        // Normalize
        if sum_prob > 0.0 {
            for new_prob in &mut new_probs {
                *new_prob /= sum_prob;
            }
        }
        // 合計が0になった場合は以前の確率を維持する (フォールバック)

        *current = new_probs;
        drop(current); // MutexGuard 早期解放 — display_probs ロック前にデッドロックリスクを排除

        // ── Hysteresis Layer ──────────────────────────────────────────────
        // display_probs に EMA を適用し、ウィンドウリセット時の
        // Cold-Start 瞬間遷移 (Stuck→Flow in 1ms) を防ぐ。
        //
        // TUNE: ヒステリシス α
        //   α=0.40 (通常): 時定数 ≈ 2.5 更新 ≈ 2.5 秒
        //     emission floor 削除 (0.05→0.01) で HMM 応答が鋭くなったため、
        //     旧 α=0.25 (4秒) から引き上げて表示の追従速度を改善。
        //   α=0.60 (ペナルティ): Backspace 連続時は素早く Stuck に収束
        let display_alpha = if apply_backspace_penalty { 0.60 } else { 0.40 };

        let mut display = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut disp_sum = 0.0;
        for i in 0..n_states {
            display[i] = display_alpha * new_probs[i] + (1.0 - display_alpha) * display[i];
            disp_sum += display[i];
        }
        if disp_sum > 0.0 {
            for v in display.iter_mut() {
                *v /= disp_sum;
            }
        }

        Some(UpdateDiagnostics {
            raw_x,
            raw_y,
            ewma_x: x,
            ewma_y: y,
            obs,
            alpha: 0.3,
        })
    }

    /// 無入力期間（サイレンス）用のHMM更新。
    /// 通常の `update()` (α=0.3) より低い α=0.15 で EWMA を緩やかに更新する。
    /// 深い沈黙 (≥15s) では α=0.25 に引き上げ、Stuck領域への到達を加速する。
    ///
    /// EWMA を完全凍結すると observation bin が固定され、HMM が遷移確率のみに依存して
    /// Stuck に到達できない（均衡点固着問題）。摩擦フロア付きの特徴量で緩やかに
    /// EWMA を更新することで、沈黙の深刻度を observation bin に反映する。
    pub fn update_silence(
        &self,
        features: &Features,
        ime_open: bool,
        silence_secs: f64,
    ) -> Option<UpdateDiagnostics> {
        if self.get_paused() {
            return None;
        }

        let (raw_x, raw_y) = self.head().latent_axes(features, ime_open);

        // TUNE: サイレンス時の動的 EWMA α — 沈黙深さに応じて段階的加速
        //   α=0.15: silence < 15s   (初期サイレンス、緩やかに推移)
        //   α=0.18: 15s ≤ silence < 25s  (中間ギア、Nudge 領域)
        //   α=0.25: silence ≥ 25s   (完全に固まった状態、Stuck 領域へ加速)
        let alpha = if silence_secs >= 25.0 {
            0.25
        } else if silence_secs >= 15.0 {
            0.18
        } else {
            0.15
        };

        // EWMA を緩やかに更新
        let (x, y) = match self.axes_ewma.lock() {
            Ok(mut ewma) => {
                ewma.0 = alpha * raw_x + (1.0 - alpha) * ewma.0;
                ewma.1 = alpha * raw_y + (1.0 - alpha) * ewma.1;
                (ewma.0, ewma.1)
            }
            Err(poisoned) => {
                let mut ewma = poisoned.into_inner();
                ewma.0 = alpha * raw_x + (1.0 - alpha) * ewma.0;
                ewma.1 = alpha * raw_y + (1.0 - alpha) * ewma.1;
                (ewma.0, ewma.1)
            }
        };

        let x_bin = (x * 5.0).floor().min(4.0) as usize;
        let y_bin = (y * 5.0).floor().min(4.0) as usize;
        let obs = x_bin * 5 + y_bin;

        // サイレンス中はペナルティビンを適用しない

        let mut current = match self.current_state_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let old_probs = *current;
        let mut new_probs = [0.0; 3];
        // ⚠️ `update()` 側と同じ理由で**配列長から取る** (★ ストライドと反復回数を 1 つに縛る)。
        let n_states = new_probs.len();
        debug_assert_eq!(old_probs.len(), n_states);

        let mut sum_prob = 0.0;
        // ⚠️★★★ `update()` 側と同じ制約 —— **加算の順序を変えないこと**。
        for (j, new_prob) in new_probs.iter_mut().enumerate() {
            let mut trans_sum = 0.0;
            for (i, &old_prob) in old_probs.iter().enumerate() {
                let t_prob = self.transitions[i * n_states + j];
                trans_sum += old_prob * t_prob;
            }
            let e_prob = self.emissions[j * 26 + obs];
            *new_prob = trans_sum * e_prob;
            sum_prob += *new_prob;
        }

        if sum_prob > 0.0 {
            for new_prob in &mut new_probs {
                *new_prob /= sum_prob;
            }
        }

        *current = new_probs;
        drop(current); // MutexGuard 早期解放

        // TUNE: ヒステリシス α=0.40 (update_silence は常に通常値)
        let mut display = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut disp_sum = 0.0;
        for i in 0..n_states {
            display[i] = 0.40 * new_probs[i] + 0.60 * display[i];
            disp_sum += display[i];
        }
        if disp_sum > 0.0 {
            for v in display.iter_mut() {
                *v /= disp_sum;
            }
        }

        Some(UpdateDiagnostics {
            raw_x,
            raw_y,
            ewma_x: x,
            ewma_y: y,
            obs,
            alpha,
        })
    }

    pub fn get_current_state(&self) -> HashMap<CognitiveState, f64> {
        // display_probs (ヒステリシス層) を返す。
        // 生の current_state_probs は瞬間値; display_probs は遅い EMA により
        // 短期スパイクを平滑化した値。UI・ログはこちらを使用する。
        let probs = match self.display_probs.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut map = HashMap::new();
        map.insert(CognitiveState::Flow, probs[0]);
        map.insert(CognitiveState::Incubation, probs[1]);
        map.insert(CognitiveState::Stuck, probs[2]);
        map
    }
}

// ============================================================================
// Unit tests (diagnostic — snapshot tests for tuning parameters)
// ============================================================================
//
// これらのテストは「意図しないパラメータドリフト」を検出するために存在する。
// 意図的な HMM チューニングでパラメータを変更した際は、テストも合わせて更新する。
// テスト更新を伴うチューニングは `.steering/` に根拠を残すこと。

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    // ---------- Transition / Emission / Initial Probs Snapshots ----------

    #[test]
    fn transition_matrix_snapshot() {
        // 9 要素 (行優先): Flow/Incubation/Stuck からの遷移確率。
        // 各行の合計 = 1.0 であることも同時に検証する。
        let e = CognitiveStateEngine::new();
        let t = e.transitions.as_ref();
        let expected: [f64; 9] = [
            0.75, 0.17, 0.08, // Flow →
            0.12, 0.80, 0.08, // Incubation →
            0.06, 0.18, 0.76, // Stuck →
        ];
        for (i, (a, b)) in t.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1e-9,
                "transitions[{}] drift: got {}, expected {}",
                i,
                a,
                b
            );
        }
        // 行和 = 1.0 の確認
        for row in 0..3 {
            let sum: f64 = t[row * 3..(row + 1) * 3].iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-9,
                "transition row {} does not sum to 1.0: {}",
                row,
                sum
            );
        }
    }

    #[test]
    fn emission_matrix_snapshot() {
        // 3 states × 26 bins = 78 要素。
        // ドリフト検出のため全値を固定比較する。
        // 意図的チューニング時はここも更新。
        let e = CognitiveStateEngine::new();
        let em = e.emissions.as_ref();
        #[rustfmt::skip]
        let expected: [f64; 78] = [
            // Flow (state 0), bins 0..25
            0.01, 0.02, 0.08, 0.12, 0.14,
            0.01, 0.02, 0.08, 0.12, 0.13,
            0.01, 0.01, 0.02, 0.01, 0.08,
            0.01, 0.01, 0.01, 0.01, 0.01,
            0.01, 0.01, 0.01, 0.01, 0.01,
            0.01, // penalty bin
            // Incubation (state 1)
            0.15, 0.10, 0.04, 0.03, 0.02,
            0.14, 0.08, 0.04, 0.03, 0.02,
            0.10, 0.08, 0.03, 0.02, 0.01,
            0.05, 0.04, 0.01, 0.01, 0.01,
            0.04, 0.03, 0.01, 0.01, 0.01,
            0.01, // penalty bin
            // Stuck (state 2)
            0.01, 0.01, 0.01, 0.01, 0.01,
            0.01, 0.01, 0.01, 0.01, 0.01,
            0.02, 0.04, 0.04, 0.05, 0.01,
            0.10, 0.16, 0.05, 0.07, 0.01,
            0.16, 0.22, 0.12, 0.05, 0.02,
            0.99, // penalty bin: near-certain Stuck
        ];
        for (i, (a, b)) in em.iter().zip(expected.iter()).enumerate() {
            assert!(
                (a - b).abs() < 1e-9,
                "emissions[{}] drift: got {}, expected {}",
                i,
                a,
                b
            );
        }
    }

    #[test]
    fn initial_probabilities_snapshot() {
        // 初期事前確率 [Flow, Incubation, Stuck] = [0.80, 0.15, 0.05]
        // Flow 優勢で開始することでセッション開始直後のフリッカーを防止する。
        let e = CognitiveStateEngine::new();
        let probs = e.current_state_probs.lock().unwrap();
        assert!((probs[0] - 0.80).abs() < 1e-9, "Flow prior drifted");
        assert!((probs[1] - 0.15).abs() < 1e-9, "Incubation prior drifted");
        assert!((probs[2] - 0.05).abs() < 1e-9, "Stuck prior drifted");
        // 確率の合計 = 1.0
        let sum: f64 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "initial probs do not sum to 1.0");
    }

    #[test]
    fn emission_penalty_bin_near_certain_stuck() {
        // ペナルティビン (obs=25) は Stuck:Flow 比 ≈ 99× の鋭い応答を維持する。
        // EMISSION_FLOOR (+0.05) 廃止後の設計意図を確認。
        let e = CognitiveStateEngine::new();
        let em = e.emissions.as_ref();
        // 各 state の penalty bin index = state * 26 + 25
        let flow_penalty = em[25];
        let inc_penalty = em[26 + 25];
        let stuck_penalty = em[52 + 25];
        assert!((flow_penalty - 0.01).abs() < 1e-9);
        assert!((inc_penalty - 0.01).abs() < 1e-9);
        assert!((stuck_penalty - 0.99).abs() < 1e-9);
        // Stuck:Flow 比 ≈ 99
        assert!((stuck_penalty / flow_penalty - 99.0).abs() < 1e-6);
    }

    // ---------- register_keystroke() ----------

    #[test]
    fn register_keystroke_non_backspace_resets_streak() {
        let e = CognitiveStateEngine::new();
        e.register_keystroke(true); // BS
        e.register_keystroke(true); // BS
        e.register_keystroke(true); // BS
        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 3);
        e.register_keystroke(false); // non-BS
        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn register_keystroke_streak_at_13_does_not_trigger_penalty() {
        // 境界値: 13 連打ではフラグが立たない
        let e = CognitiveStateEngine::new();
        for _ in 0..13 {
            e.register_keystroke(true);
        }
        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 13);
        assert!(!e.has_pending_penalty.load(Ordering::Acquire));
    }

    #[test]
    fn register_keystroke_streak_at_14_triggers_pending_penalty() {
        // 閾値: 14 連打到達でフラグが立つ
        let e = CognitiveStateEngine::new();
        for _ in 0..14 {
            e.register_keystroke(true);
        }
        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 14);
        assert!(e.has_pending_penalty.load(Ordering::Acquire));
    }

    #[test]
    fn register_keystroke_penalty_flag_persists_through_non_bs() {
        // BS×14 で立ったフラグは、非 BS キーでストリークがリセットされても保持される
        // （次の update() で消費されるまで）。
        let e = CognitiveStateEngine::new();
        for _ in 0..14 {
            e.register_keystroke(true);
        }
        assert!(e.has_pending_penalty.load(Ordering::Acquire));
        e.register_keystroke(false); // non-BS → streak リセット
        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 0);
        // フラグは依然立っている
        assert!(e.has_pending_penalty.load(Ordering::Acquire));
    }

    #[test]
    fn reset_clears_all_state() {
        // reset() は全ての可変状態を初期値に戻す
        let e = CognitiveStateEngine::new();
        for _ in 0..14 {
            e.register_keystroke(true);
        }
        e.set_paused(true);
        assert!(e.has_pending_penalty.load(Ordering::Acquire));
        assert!(e.get_paused());

        e.reset();

        assert_eq!(e.backspace_streak.load(Ordering::Relaxed), 0);
        assert!(!e.has_pending_penalty.load(Ordering::Acquire));
        assert!(!e.get_paused());
        let probs = e.current_state_probs.lock().unwrap();
        assert!((probs[0] - 0.80).abs() < 1e-9);
        assert!((probs[1] - 0.15).abs() < 1e-9);
        assert!((probs[2] - 0.05).abs() < 1e-9);
    }

    // ---------- ADR-006 Phase 2a: 抽出が挙動を変えていないことのガード ----------

    /// `HMM_VERSION` のピン留め。HMM 挙動を変えたら**必ずここも更新する**
    /// （更新を忘れると、同じ版で 2 つの挙動が存在することになり
    /// `gse-replay` の版検証が意味を失う）。
    ///
    /// - `0.3.0`: Phase1。ADR-006 Phase 2a (抽出) は挙動不変なので据え置いた
    /// - `0.4.0`: ADR-006 Phase 2b。`task_kind` → head 配線 + Coding head v0
    /// - `0.5.0`: ⚠️★★★ **IME による推論の一時停止をやめた** (2026-08-23・研究者判断)。
    ///   ★ `lib.rs` の IME Monitor と `analysis/loop_step.rs` の両方から
    ///   `set_paused` を外したので、**同じ入力 NDJSON から出る `FeatSnapshot` 列が変わる**。
    ///   ⚠️ 実データ fixture `phase1_ime_heavy` は **6 → 98 スナップショット**
    ///   (＝旧挙動は推論窓の **94%** を潰していた)。
    ///   ★ 理由: 実測でこの信号は「候補リスト表示」ではなく**変換セッション全体**を覆う
    ///   (`.steering/20260823-ime-active-cloak-rewiring/decisions.md` D-10)。
    #[test]
    fn hmm_version_is_0_5_0() {
        assert_eq!(HMM_VERSION, "0.5.0");
    }

    /// 既定の engine は Writing head を積む = Phase1 と同一経路。
    #[test]
    fn default_engine_uses_writing_head() {
        let e = CognitiveStateEngine::new();
        assert_eq!(e.head_id(), crate::analysis::heads::HeadId::Writing);
    }

    /// ⚠️ **`reset()` は head を既定 (Writing) に戻す。**
    ///
    /// engine は起動時 1 個をセッション間で使い回すので、戻さないと
    /// 「前のセッションが vibe_coding だった」だけで次のセッションが Coding head で
    /// 採点され続ける (task_kind 未宣言のセッションで顕在化する)。
    #[test]
    fn reset_restores_writing_head() {
        use crate::analysis::heads::HeadId;

        let e = CognitiveStateEngine::new_for_domain(TaskDomain::Coding);
        assert_eq!(e.head_id(), HeadId::CodingV0);

        e.reset();

        assert_eq!(
            e.head_id(),
            HeadId::Writing,
            "reset() 後に前セッションの head が残っている"
        );
    }

    /// `set_domain()` は**反映後の** `head_id` を返す。
    ///
    /// 呼び出し側が `set_domain` → `head_id()` と 2 回ロックを取ると、その隙間に
    /// 別の差し替えが入って「記録した head_id と実際に走る head が違う」状態になりうる。
    #[test]
    fn set_domain_returns_the_applied_head_id() {
        use crate::analysis::heads::HeadId;

        let e = CognitiveStateEngine::new();
        for (domain, expected) in [
            (TaskDomain::Coding, HeadId::CodingV0),
            (TaskDomain::WritingFallback, HeadId::WritingFallback),
            (TaskDomain::Writing, HeadId::Writing),
        ] {
            let applied = e.set_domain(domain);
            assert_eq!(applied, expected);
            assert_eq!(e.head_id(), expected, "戻り値と実際の head がずれている");
        }
    }

    /// backspace ペナルティ gate は head 経由になっても `!ime_open` のままである。
    #[test]
    fn backspace_penalty_gate_is_delegated_to_head() {
        let e = CognitiveStateEngine::new();
        assert!(e.head().applies_backspace_penalty(false));
        assert!(!e.head().applies_backspace_penalty(true));
    }
}
