//! `gse-replay --features` の計算本体 (ADR-030 D1: 検出器はオフライン側に置く)。
//!
//! セッション NDJSON から再構成した press/paste イベント列を、既存 `loop_step`
//! と同じ 1 Hz グリッド (`ENGINE_TICK_MS` 相当) で tick に分割し、Tier ごとの
//! 特徴行 [`FeatureRow`] を作る。**`features.rs` / `engine.rs` を変更しない**
//! (D1 逐語「検出器はオフライン側に置く」) —— 数値も `features.rs` の既存
//! パラメータ (`PAUSE_THRESHOLD_MS`) を `use` するだけで、新しい定数は作らない
//! (ADR-030 D3 逐語「新しい定数を作らず、既存パラメータを対称に再利用する」)。
//!
//! # tick への帰属規則 (SSOT はここ)
//!
//! - tick 幅は [`ENGINE_TICK_MS`] (= 1000ms・`loop_step` の 1 Hz グリッドの SSOT)。
//!   tick `k` は区間 `[grid_origin_ms + k*ENGINE_TICK_MS, grid_origin_ms + (k+1)*ENGINE_TICK_MS)`
//!   を表す (左閉右開)。`grid_origin_ms` はセッション内の最初のイベント時刻
//!   (**全チャネル**= press / paste / `ime_shadow` の最小値)。
//!   ⚠️★★★ **グリッドは「打鍵だけ」ではなく入力チャネル全部を覆う。**
//!   ★ そうしないと、打鍵より前/後にある `ime_shadow` 行が tick 配列の範囲外になる。
//!   ⚠️★★★ **グリッドの右端 (終端) は `max(最後の入力イベント, 最後の feat 行)` である**
//!   (issue 008)。★ そうしないと、末尾の沈黙で `stall_onset` の `t0` が入力イベントより
//!   後ろに来たとき、その onset の帰属先 tick がグリッドの外に出て黙って捨てられる
//!   (2026-09-15 実測: `code-reviewer` HIGH)。⚠️ **原点 (最小値) は変えない** —— 入力チャネル
//!   だけで決める (`feat` 行は原点の計算に加えない)。
//!   ⚠️ **副作用を承知して使うこと** (2026-09-15・`code-reviewer` MEDIUM・実測):
//!   同じ打鍵列でも **`ime_shadow` 行が 1 本あるかどうかで原点が動き、全行の `t_ms` がずれる**
//!   (実測では 100ms シフトした)。★ `ime_shadow` は `config_gated` (環境によって丸ごと出ない) なので、
//!   **同じセッションを計器条件の違う環境で解析すると tick 割りが揃わない**。
//!   → ★ 複数セッション/複数端末の行を突き合わせるときは `t_ms` の絶対値で join し、
//!   **tick の添字で揃えない**こと。回帰は
//!   `bin/replay.rs` の `ime_shadow_events_reach_feature_rows_through_the_collector` が張る。
//!   ⚠️ 本モジュールは **1000 を再宣言しない** —— `loop_step::ENGINE_TICK_MS` を `use` する
//!   (ADR-030 D3 逐語「新しい定数を作らず、既存パラメータを対称に再利用する」)。
//! - ★ `t_ms` は **絶対 epoch ms** (`grid_origin_ms + k*ENGINE_TICK_MS`) である。
//!   ⚠️ セッション相対オフセットにしない —— `FeatSnapshot::timestamp` /
//!   `metrics::StuckEpisode` / `--metrics` 系がすべて epoch 軸なので、
//!   相対にすると **判断 2 の「episode への集約は後から join する形で足せる」が
//!   この出力単体では成立しなくなる** (原点が行に載らないため)。
//!   ⚠️ ただし本モジュールは production のゲート状態を再現するものではなく、
//!   オフライン一括計算専用である (`loop_step` の `now_ms` の刻みとは別物)。
//! - **打鍵 (`keys` / `deletes`)** は press 時刻がその tick 区間に入るものを計上する。
//! - **paste (`pastes`)** も同様に paste 時刻がその tick 区間に入るものを計上する。
//! - **打鍵間隔由来の値 (`mean_iki_ms` / `pauses`)** は、連続する press ペアの
//!   間隔を「2 打目 (再開する側) の press が属する tick」に帰属させる
//!   (`features.rs` の F5/F6 が同じペア窓で「後続イベント側」を見ているのと対称)。
//!   ⚠️ したがって tick 境界をまたぐ間隔は、間隔の開始ではなく終了側の tick に乗る。
//!   ⚠️ セッション末尾が pause のまま閉じずに終わる場合 (最後の press の後ろに
//!   何も続かない)、その末尾の沈黙は「組」にならないため計上しない
//!   (未来の press が来て初めて間隔が確定する設計)。
//!
//! # 欠測の表現
//!
//! `mean_iki_ms` は間隔が 1 件も無い tick (press が 0 or 1 件) では `null`
//! (`Option::None`) を出す。**`0.0` にフォールバックしない** — 「間隔が無い」と
//! 「間隔が 0ms だった (同時刻打鍵)」を区別する。他の 5 フィールドは常に計上可能
//! (最小値 0) なので `Option` を使わない。
//!
//! ★ 後から足した 2 Tier も同じ原則に従う (⚠️ 0 で埋めない):
//! - `revision` (`T1c`): **件数は常に数** (0 を含む)、**最大値・率はその tick に閉じる単位が
//!   無ければ `null`**。閉じがグリッドの外に落ちた単位は黙って捨てる (`revision` モジュール doc)。
//! - `restart` (`T1d`): **`t0` が立っていない行は 7 フィールドとも `null`**。
//!   onset のある行で `restop_ms` が `null` なのは**観測区間の右端まで再停止を観測できなかった
//!   右側打ち切り**を意味する (⚠️ `censored` フィールドは持たない・判断 2 V4)。
//!   集計窓そのものが無い行では `post_t0_*` / `resumption_count` も `null` (判断 2 V5)。
//!
//! # Tier ごとの causal / retrospective の区分
//!
//! `T0` (activity) は **causal**: 各 tick の値は、その tick が閉じた時点までに
//! 実際に発生した入力だけを集計したものであり、セッション全体を見てから遡って
//! 付け直すラベルを一切含まない。将来追加する Tier (`T1a` の個人相対 surprise 等)
//! が **retrospective** (セッション全体の分布を要する事後ラベル) を持ち込む場合は、
//! `FeatureRow` の中でも causal な `T0` と混ぜず、別フィールド/別ドキュメントコメントで
//! 明示すること (「その時点で既知」と「後から付けたラベル」を tick 行の中で混ぜない)。

pub mod activity;
pub mod boundary;
pub mod dispersion;
pub mod idle_trim;
pub mod quality;
pub mod restart;
pub mod revision;
pub mod stall_onset;

pub use activity::T0Activity;
pub use boundary::{Boundary, ImeShadowEvent};
pub use dispersion::Dispersion;
pub use quality::{HookHealthEvent, Quality};
pub use restart::Restart;
pub use revision::Revision;
pub use stall_onset::{Pole, StallOnset, StallOnsetConfig};

use crate::analysis::loop_step::ENGINE_TICK_MS;
use crate::analysis::metrics::{is_backspace, is_text_producing, KeyPress};

/// `build_feature_rows` の `stall_onset` (issue 008) 用の追加入力をまとめた構造体。
///
/// ★ `keys` / `paste_ts` / `ime_events` / `max_ticks` に加えて `feat_ts` /
/// `task_kind_declarations` / `config` を引数で渡すと `clippy::too_many_arguments`
/// に抵触するため、`stall_onset` 固有の入力だけを 1 つにまとめた。
pub struct StallOnsetInputs<'a> {
    /// 1Hz 特徴列 (`feat` 行) の時刻。値は読まない — 観測区間の判定にだけ使う
    /// (ADR-030 D8 (b-2))。
    pub feat_ts: &'a [u64],
    /// `session_meta.task_kind` の宣言列 (`(ts, task_kind)`。順不同でよい)。
    pub task_kind_declarations: &'a [(u64, String)],
    pub config: StallOnsetConfig,
}

/// **極ごとの追加入力** (ADR-030 D3・[`Pole`])。[`build_feature_rows_with_pole_inputs`] だけが受ける。
///
/// ⚠️ [`StallOnsetInputs`] に足さない —— あちらは `..` 無しで全フィールドを列挙するリテラルが
/// 既存テストに多数あり (`dispersion.rs` / `restart.rs` ほか)、フィールドを足すたびに壊れる。
/// ★ 極の入力は `Default` を持つこの構造体に置く (極 B の `pointer_active_ts` もここ)。
#[derive(Debug, Clone, Copy, Default)]
pub struct PoleInputs<'a> {
    /// 極 M+ の**修飾キーの押下区間** ([`stall_onset::modifier_press_intervals`] の戻り値・両端を含む)。
    /// ⚠️ `None` は「渡されていない」であり「区間が 0 本」(`Some(&[])`) とは別 —— 極 M+ で `None` なら
    /// `Err` にする (★ 入力を渡し忘れて baseline の値を極 M+ の名前で返さない)。
    pub modifier_intervals: Option<&'a [(u64, u64)]>,
    /// 極 B の**ポインタ活動の行の時刻** (`pointer_bin` 行の `t`・**ビンの終端時刻**・順不同でよい)。
    /// ★ 行の**存在だけ**を使う (値は読まない)。抑制区間は [`stall_onset::pointer_suppression_intervals`] が作る。
    /// ⚠️ `None` は「渡されていない」であり「活動が 0 行」(`Some(&[])`) とは別 —— 極 B で `None` なら
    /// `Err` にする (★ 入力を渡し忘れて baseline の値を極 B の名前で返さない)。
    /// ★ `capabilities` に `pointer_bin` の宣言が無い入力の扱い (= 適用不可) は**呼び出し側**が決める
    /// (`analysis/` に `capabilities` の意味を持ち込まない)。適用不可なら `Some(&[])` を渡すと baseline と恒等になる。
    ///
    /// ⚠️ **抑制区間→評価区間の構築は 1 か所に畳まれていない** (`bin/replay.rs` が封筒
    /// `provenance.observed_intervals` 用にもう一度計算する) が、**両者は同じ 2 関数
    /// (`stall_onset::pointer_suppression_intervals` / `stall_onset::evaluation_intervals`) と
    /// 同じ定数 (`ENGINE_TICK_MS`) を呼ぶ**ので、値が独立に乖離することはない
    /// (`replay.rs` 側の呼び出し箇所のコメント参照。code-reviewer LOW・issue 006)。
    pub pointer_active_ts: Option<&'a [u64]>,
}

/// `build_feature_rows` の `quality` (issue 007) 用の追加入力をまとめた構造体。
/// `stall_onset_inputs` と同じ理由 (`clippy::too_many_arguments` 回避) で 1 つにまとめる。
/// ⚠️ `observability_mask.feat` の判定には**この構造体の値を使わない** —— 008 の
/// `stall_onset_inputs.feat_ts` / `stall_onset_inputs.config.hole_gap_ms` をそのまま使う
/// (feat 時刻を別に読み直さない・issue 007 Allowed Files 節)。
pub struct QualityInputs<'a> {
    /// `meta` 行に `capabilities` の宣言があったか。
    ///
    /// ⚠️★★★ **値そのものは受け取らない** (2026-09-20・判断 3)。★ `capabilities` は
    /// セッション不変の事実なので、`gse-replay --features` の出力では**封筒**
    /// (`bin/replay.rs::FeatureProvenance::capabilities`) が 1 回だけ持つ。
    /// ここで要るのは「宣言があったか」という真偽だけで、それが tick ごとの
    /// `missing_reasons` に `"capability_undeclared"` を立てる。
    /// ★ これにより `Arc` 共有 (2026-09-18・判断 24・code-reviewer M4) も不要になった。
    pub capabilities_declared: bool,
    /// `hook_health` 行の列 (順不同でよい)。
    pub hook_health: &'a [quality::HookHealthEvent],
}

/// 1 tick ぶんの特徴行。`activity` は `T0` (Tier 0)、`boundary` は `T1b` (境界)、
/// `dispersion` は `T1a` (分布・個人相対)。
///
/// ⚠️ `dispersion` はモジュール doc 「Tier ごとの causal / retrospective の区分」が
/// 予告した retrospective 側の Tier —— `surprise` はセッション全体の IKI baseline を
/// 要するため、`activity` (causal) と混ぜず別フィールドに置く。
/// ★ `dispersion.pause_*` は `t1a_window_ms` の**因果窓**で集約した値である (issue 003・
/// `dispersion::pause_ms_by_window`)。⚠️ tick 帰属ではなく窓なので、上の「tick への帰属規則」の
/// `pauses` (件数・tick 帰属) とは集計単位が違う。
#[derive(Debug, Clone, serde::Serialize)]
pub struct FeatureRow {
    /// tick 開始時刻 (**絶対 epoch ms**)。モジュール doc の「tick への帰属規則」参照。
    pub t_ms: u64,
    pub activity: T0Activity,
    pub boundary: Boundary,
    pub dispersion: Dispersion,
    /// `stall onset`（`t0`）検出器 (ADR-030 D2-D6)。`activity` と同じく **causal**
    /// (`stall_onset` モジュール doc 参照)。onset が起きた tick だけ値が埋まる。
    /// ⚠️ **ただし `ime_overlap` だけは retrospective** (`t0` より後に書かれる `ime_shadow` 行に依存しうる・issue 007)。
    pub stall_onset: StallOnset,
    /// `Q` (evidence quality・issue 007)。⚠️ 状態特徴ではなく出力にだけ載せる —
    /// 他 Tier (`activity` / `boundary` / `dispersion` / `stall_onset`) の計算へは渡さない
    /// (`quality` モジュール doc 参照)。
    pub quality: Quality,
    /// `T1c` = **削除と挿入の遷移の形** (issue 004)。`activity` と同じく **causal** —
    /// 各 tick には「その tick までに閉じが確定した単位」しか載らない (`revision` モジュール doc の U3)。
    /// ⚠️ 「修正 (revision) の位相」とは呼ばない (★ 識別子だけが `revision` である)。
    pub revision: Revision,
    /// `T1d` = **再開** (issue 006)。⚠️★★★ **`restart` は retrospective (転帰側) である** ——
    /// `stall onset` の `t0` より**後**を見る量なので、⚠️ **detection 側 (`activity` / `stall_onset`)
    /// の入力に渡さない** (ADR-017 D8 / ADR-024 §Migration 逐語「再開後の動態は *アウトカム* であって、
    /// 決して *入力* にしない」・issue 006 AC7)。★ 配線も `stall_onset` → `restart` の一方向だけである。
    ///
    /// ★ **打ち切りの読み方** (⚠️ `censored` フィールドは持たない・判断 2 V4):
    /// `stall_onset.t0_ms != null` かつ `restart.restop_ms == null` の行は、
    /// **観測区間の右端まで再停止を観測できなかった (右側打ち切り)** を意味する。
    /// ⚠️ 追跡時間そのものは載せていない (★ 生存解析に使うなら `blockers.md` 起票 20)。
    pub restart: Restart,
}

/// 連続する press 間の間隔 1 件。`resuming_ts` はその間隔を終える (2 打目の) press 時刻。
#[derive(Debug, Clone, Copy)]
struct PressInterval {
    resuming_ts: u64,
    gap_ms: u64,
}

/// press 時刻列 (任意順でよい) から連続間隔の列を作る内部ヘルパ。
/// 昇順ソート後に `windows(2)` を取るため、入力の行順 (=時刻逆行を含む NDJSON) に依らない。
fn press_intervals(press_ts: &[u64]) -> Vec<PressInterval> {
    let mut sorted: Vec<u64> = press_ts.to_vec();
    sorted.sort_unstable();
    sorted
        .windows(2)
        .map(|w| PressInterval {
            resuming_ts: w[1],
            gap_ms: w[1].saturating_sub(w[0]),
        })
        .collect()
}

/// ★ 共有ヘルパ: 「単純 pause」(連続する press の間隔が `features.rs::PAUSE_THRESHOLD_MS`
/// 以上) を抽出し、pause を終える (再開する) press 時刻の列を返す。
///
/// 後続 issue (T1c/T1d/stall onset 等) はこの関数を再利用し、独自の pause 定義を
/// 作らないこと (Codex TASK-PRE 指摘 5)。値は `features.rs::PAUSE_THRESHOLD_MS`
/// を `use` するのみで、ここでは 1 つも数値を導入しない。
///
/// ⚠️★★★ **issue 005 (`stall onset`) がそのまま条件 B に使える関数ではない** ——
/// ★ 本関数が見るのは **press ペアの間隔**だけである。
/// 1. ⚠️ **閉じない末尾の沈黙を返さない** (再開する 2 打目が無いと「組」にならない)。
///    ADR-030 D3 は逐語「**沈黙こそ最長の停滞である**」として評価時刻に
///    `{key の t + W}` を含めるので、**005 はそこを自前で補う必要がある**。
/// 2. ⚠️ **`feat` 行の穴 (削除区間・一時停止・計器欠損) を知らない** ——
///    ADR-030 D8 (b-2) は原因を問わず穴で条件 B の連続を切ることを要求する。
///    ★ 本関数を素通しで使うと **被験者が削除を求めた位置に必ず onset が立つ**。
///    ⚠️ ただし**原因ラベルを作る方向で直さないこと** (`redaction.rs` の約束「何を消したかは
///    書かない」・ADR-030 却下リスト 9)。穴の**有無**だけを 005 側で見る。
pub fn extract_pause_resumptions(press_ts: &[u64]) -> Vec<u64> {
    use crate::analysis::features::PAUSE_THRESHOLD_MS;
    press_intervals(press_ts)
        .into_iter()
        .filter(|iv| iv.gap_ms >= PAUSE_THRESHOLD_MS)
        .map(|iv| iv.resuming_ts)
        .collect()
}

/// キー 1 件ぶんの net production 寄与 (+1 挿入 / -1 削除 / 0 その他)。
fn key_net_delta(k: &KeyPress) -> i64 {
    if is_backspace(k.vk) {
        -1
    } else if is_text_producing(k.vk) {
        1
    } else {
        0
    }
}

/// ★ 共有ヘルパ: net production = 文字生成キー press 数 − backspace/delete press 数
/// (挿入系 − 削除系。`metrics::is_text_producing` / `metrics::is_backspace` を再利用し、
/// 新しい文字分類を作らない)。削除超過の tick では負になりうる (符号付き)。
pub fn net_production(keys: &[KeyPress]) -> i64 {
    keys.iter().map(key_net_delta).sum()
}

/// tick ループ: press/paste 列から 1 Hz tick 単位の [`FeatureRow`] 列を作る。
///
/// `keys` は `bin/replay.rs` の `collect_key_events` が返す形式、`paste_ts` は
/// **すべての paste 行の時刻**(⚠️ 文字数が欠落した行も含む。`T0` は件数のみ扱うため)。
/// どちらも timestamp 昇順ソート済みを仮定しないが、内部で扱いを吸収する。
/// 空入力 (打鍵も paste も無い) では空の `Vec` を返す (境界入力: 空ログ)。
///
/// ⚠️★★★ **`max_ticks` は呼び出し側が渡す** —— tick 数は**入力のバイト数ではなく
/// 時刻スパンに比例する**ので、`MAX_SESSION_BYTES` では抑えられない。
/// ★ 実測 (2026-09-15): **251 バイト・2 行**の NDJSON に `t=0` と `t=1e8` を並べるだけで
/// **100,001 行**が出る。ADR-030 **D9**「壁時計の前方跳躍」は実際に起こりうるので、
/// 巨大ギャップは**黙って潰さず** `Err` で止める (⚠️ `saturating_sub` で吸収すると
/// D9 の危険側に落ちる)。★ 上限値そのものは `bin/replay.rs` が所有する
/// (⚠️ 本モジュールに新しい数値定数を置かないため・AC 6)。
///
/// ⚠️★★★ **`stall_onset_inputs.config.pole` が未実装の極なら `Err`** ([`Pole::is_implemented`])。
/// ★ 極の中身が無いのに `baseline` の値を返すと、出力が `pole = pointer_b` と名乗ったまま
/// 別の計算の結果を載せる (**出力が嘘をつく**)。⚠️ **入力が空でも先に検査する** ——
/// 空の `Vec` を返すと「その極で計算した結果が 0 行だった」と読まれる。
///
/// ⚠️ 本関数は**極の入力を持たない** ([`PoleInputs::default`])。★ 極 M+ は押下区間が要るので
/// `Err` になる —— [`build_feature_rows_with_pole_inputs`] を使うこと。
pub fn build_feature_rows(
    keys: &[KeyPress],
    paste_ts: &[u64],
    ime_events: &[ImeShadowEvent],
    max_ticks: usize,
    stall_onset_inputs: &StallOnsetInputs,
    quality_inputs: &QualityInputs,
) -> Result<Vec<FeatureRow>, String> {
    build_feature_rows_with_pole_inputs(
        keys,
        paste_ts,
        ime_events,
        max_ticks,
        stall_onset_inputs,
        quality_inputs,
        &PoleInputs::default(),
    )
}

/// [`build_feature_rows`] に**極ごとの入力** ([`PoleInputs`]) を足した版。
///
/// ★ 極が変えるのは**検出 (`stall_onset`) と、その条件 A の辺・挿入系の写像を使う `restart` だけ**である。
/// ⚠️ `T0` / `T1a` / `T1b` / `T1c` / `Q` は極に依らない (baseline と同じ `keys` を見る)。
/// ★ 極 M+ は検出へ渡す `key` 列の側で表す ([`stall_onset::apply_modifier_m_plus`]) —— press の時刻は
/// 1 本も変わらないので θ の標本は極に依らず、変わるのは挿入系 (`+1`) の数え方だけ。
///
/// ⚠️★★★ 極 M+ で `pole_inputs.modifier_intervals` が `None` なら `Err` (入力が空でも先に検査する)。
pub fn build_feature_rows_with_pole_inputs(
    keys: &[KeyPress],
    paste_ts: &[u64],
    ime_events: &[ImeShadowEvent],
    max_ticks: usize,
    stall_onset_inputs: &StallOnsetInputs,
    quality_inputs: &QualityInputs,
    pole_inputs: &PoleInputs,
) -> Result<Vec<FeatureRow>, String> {
    let pole = stall_onset_inputs.config.pole;
    if !pole.is_implemented() {
        return Err(format!(
            "pole `{}` is not implemented yet (refusing to return baseline values under another pole's name)",
            pole.as_str()
        ));
    }
    // ⚠️ 検出へ渡す `key` 列。極 M+ だけが作り直す (baseline は借りるだけ・1 バイトも複製しない)。
    let detection_keys: std::borrow::Cow<[KeyPress]> = match pole {
        Pole::ModifierMPlus => match pole_inputs.modifier_intervals {
            Some(intervals) => {
                std::borrow::Cow::Owned(stall_onset::apply_modifier_m_plus(keys, intervals))
            }
            None => {
                return Err(format!(
                "pole `{}` needs its modifier intervals (PoleInputs::modifier_intervals is None; \
                     refusing to return baseline values under this pole's name)",
                pole.as_str()
            ))
            }
        },
        // ★ ワイルドカードなしの網羅列挙 (極を足すと、ここで判定を決めるまでコンパイルが
        //   通らない。`pole_applicability` (`bin/replay.rs`) と同じ規律)。
        //   極 B は `key` 列を変えない (変えるのは評価区間の側)。
        Pole::Baseline | Pole::PointerB => std::borrow::Cow::Borrowed(keys),
    };
    // ⚠️ 極 B の**抑制区間**。極 B 以外は空 (= 観測区間そのまま)。入力が空でも先に検査する (上と同じ理由)。
    let pointer_suppression: Vec<(u64, u64)> = match pole {
        Pole::PointerB => match pole_inputs.pointer_active_ts {
            // ★ `pointer_bin` の 1 ビン幅は 1 秒 (= `ENGINE_TICK_MS`)。写しであってリンクではない (`pointer_suppression_intervals` の doc)。
            Some(ts) => stall_onset::pointer_suppression_intervals(ts, ENGINE_TICK_MS),
            None => {
                return Err(format!(
                    "pole `{}` needs its pointer activity (PoleInputs::pointer_active_ts is None; \
                     refusing to return baseline values under this pole's name)",
                    pole.as_str()
                ))
            }
        },
        Pole::Baseline | Pole::ModifierMPlus => Vec::new(),
    };
    if keys.is_empty() && paste_ts.is_empty() && ime_events.is_empty() {
        return Ok(Vec::new());
    }

    let key_ts = keys.iter().map(|k| k.timestamp);
    let paste_ts_iter = paste_ts.iter().copied();
    let ime_ts_iter = ime_events.iter().map(|e| e.t_ms);
    // ⚠️ 原点 (最小値) は入力チャネルだけで決める。feat 行は加えない (mod.rs doc 参照)。
    // unwrap: 上の early-return で空入力を排除済みなので、少なくとも1件は必ずある。
    let grid_origin_ms = key_ts
        .clone()
        .chain(paste_ts_iter.clone())
        .chain(ime_ts_iter.clone())
        .min()
        .unwrap();
    // ★ 右端 (終端) は入力チャネルと feat 行の max (issue 008)。
    // ⚠️ どちらが右端を決めたかをエラーで区別できるよう、別々に max を取っておく
    // (LOW-6: 値の大きさは出してよいが、行の中身は出さない)。
    let t_last_events = key_ts
        .chain(paste_ts_iter)
        .chain(ime_ts_iter)
        .max()
        .unwrap();
    let t_last_feat = stall_onset_inputs.feat_ts.iter().copied().max();
    let t_last = match t_last_feat {
        Some(f) => t_last_events.max(f),
        None => t_last_events,
    };

    let tick_of =
        |t: u64| -> usize { (t.saturating_sub(grid_origin_ms) / ENGINE_TICK_MS) as usize };
    let tick_count = tick_of(t_last) + 1;
    if tick_count > max_ticks {
        // ⚠️ 行の中身 (feat の値・key の vk 等) は出さない —— 出すのは時刻の大きさだけ。
        let (t_last_source, t_last_source_ms) = match t_last_feat {
            Some(f) if f > t_last_events => ("the last feat row", f),
            _ => ("the last input event", t_last_events),
        };
        return Err(format!(
            "time span too large: {} ticks exceeds limit {} ticks \
             (first input event {} ms, grid ends at {} ms, determined by {} at {} ms). \
             The log likely contains a wall-clock jump or concatenated sessions (ADR-030 D9).",
            tick_count, max_ticks, grid_origin_ms, t_last, t_last_source, t_last_source_ms
        ));
    }

    let mut keys_by_tick: Vec<Vec<KeyPress>> = vec![Vec::new(); tick_count];
    for k in keys {
        keys_by_tick[tick_of(k.timestamp)].push(*k);
    }
    let mut paste_counts: Vec<u64> = vec![0; tick_count];
    for t in paste_ts {
        paste_counts[tick_of(*t)] += 1;
    }

    let all_press_ts: Vec<u64> = keys.iter().map(|k| k.timestamp).collect();

    let mut iki_ms_by_tick: Vec<Vec<u64>> = vec![Vec::new(); tick_count];
    for iv in press_intervals(&all_press_ts) {
        iki_ms_by_tick[tick_of(iv.resuming_ts)].push(iv.gap_ms);
    }

    let mut pause_count_by_tick: Vec<u64> = vec![0; tick_count];
    for resuming_ts in extract_pause_resumptions(&all_press_ts) {
        pause_count_by_tick[tick_of(resuming_ts)] += 1;
    }

    let mut ime_by_tick: Vec<Vec<ImeShadowEvent>> = vec![Vec::new(); tick_count];
    for ev in ime_events {
        ime_by_tick[tick_of(ev.t_ms)].push(*ev);
    }
    let mut boundary_pairs_by_tick: Vec<Vec<boundary::PauseBoundaryPair>> =
        vec![Vec::new(); tick_count];
    for pair in boundary::pause_boundary_pairs(keys) {
        boundary_pairs_by_tick[tick_of(pair.resuming_ts)].push(pair);
    }

    // ★ `T1a` の pause 分位点は tick ではなく `t1a_window_ms` の因果窓で集約する (issue 003・判断 4)。
    //   ⚠️ 窓は観測区間 (`feat` 行の穴) をまたがない (起票 6) —— 観測区間は **dispersion 用に**
    //   ここで 1 度だけ作って渡す (`dispersion` から `feat` 行を読み直さない。⚠️ `quality_column`
    //   は別の呼び出しで自分の観測区間を作り直しており、これはそちらへは渡っていない)。
    //   `iki_cv` / `surprise` には窓を掛けない。
    //   ⚠️ `feat` 行が 1 本も無ければ `observed` は空で、`pause_*` は全行 `null` になる
    //   (観測できないものを値で埋めない・issue 008・判断 9。onset と同じ向き)。
    let observed = stall_onset::observed_intervals(
        stall_onset_inputs.feat_ts,
        stall_onset_inputs.config.hole_gap_ms,
    );
    let pause_ms_by_tick = dispersion::pause_ms_by_window(
        &dispersion::pause_durations(&all_press_ts),
        tick_count,
        grid_origin_ms,
        ENGINE_TICK_MS,
        stall_onset_inputs.config.t1a_window_ms,
        &observed,
    );
    // ★ baseline はセッション全体の IKI から一度だけ作り、tick ごとに作り直さない
    // (AC5: surprise はセッション相対)。
    let all_iki_ms: Vec<u64> = iki_ms_by_tick.iter().flatten().copied().collect();
    let iki_baseline = dispersion::SessionIkiBaseline::from_session_iki(&all_iki_ms);

    // ★ 条件 A の時系列は `restart` (T1d・issue 006 V2) が使う。⚠️ 検出を 2 回走らせないよう、
    //   tick 列と同じ 1 回の走査から受け取る (issue 008 AC18 / AC18b の計算量の門)。
    //   ★ 検出と `restart` だけが `detection_keys` (極 M+ では挿入系を置き換えた列) を見る。
    let (stall_onset_col, condition_a_timeline) = stall_onset::stall_onset_column_with_timeline(
        &detection_keys,
        stall_onset_inputs.feat_ts,
        &pointer_suppression,
        // ★ `ime_shadow` 行は onset の `ime_overlap` を埋めるためだけに渡す (issue 007)。
        //   ⚠️ 検出は読まない —— 検出値 (s / t0 / θ / n) を変えない (数えるだけで抑制しない)。
        //   ★ ただし `ime_events` は上の grid_origin_ms / t_last には既に入っている (issue 002 から・本 issue は変えない)。
        ime_events,
        stall_onset_inputs.task_kind_declarations,
        tick_count,
        grid_origin_ms,
        ENGINE_TICK_MS,
        &stall_onset_inputs.config,
    );
    let revision_col = revision::revision_column(keys, tick_count, grid_origin_ms, ENGINE_TICK_MS);
    // ⚠️★★★ 向きは `stall_onset` → `restart` の一方向だけ (issue 006 AC7)。
    //   ★ 転帰側 (`restart`) の値は detection 側 (`activity` / `stall_onset`) の入力に渡さない。
    let restart_col = restart::restart_column(
        &stall_onset_col,
        &condition_a_timeline,
        &detection_keys,
        &stall_onset_inputs.config,
    );
    // ⚠️ observability_mask.feat は 008 の feat_ts / hole_gap_ms をそのまま使う
    // (quality_inputs には feat 側の入力を持たせない・issue 007 Allowed Files 節)。
    let quality_col = quality::quality_column(
        tick_count,
        grid_origin_ms,
        ENGINE_TICK_MS,
        quality_inputs.capabilities_declared,
        quality_inputs.hook_health,
        stall_onset_inputs.feat_ts,
        stall_onset_inputs.config.hole_gap_ms,
    );

    Ok((0..tick_count)
        .map(|idx| FeatureRow {
            t_ms: grid_origin_ms + idx as u64 * ENGINE_TICK_MS,
            activity: activity::compute_t0_activity(
                &keys_by_tick[idx],
                paste_counts[idx],
                &iki_ms_by_tick[idx],
                pause_count_by_tick[idx],
            ),
            boundary: boundary::compute_boundary(
                &keys_by_tick[idx],
                &boundary_pairs_by_tick[idx],
                &ime_by_tick[idx],
            ),
            dispersion: dispersion::compute_dispersion(
                &pause_ms_by_tick[idx],
                &iki_ms_by_tick[idx],
                iki_baseline.as_ref(),
            ),
            stall_onset: stall_onset_col[idx].clone(),
            quality: quality_col[idx].clone(),
            revision: revision_col[idx].clone(),
            restart: restart_col[idx].clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_at(ms: u64) -> KeyPress {
        KeyPress {
            timestamp: ms,
            vk: 0x41, // 'A' — text-producing
        }
    }

    /// テストの既定上限。実運用の値は `bin/replay.rs::MAX_FEATURE_TICKS` が所有する。
    const TEST_MAX_TICKS: usize = 10_000;

    fn test_stall_onset_inputs() -> StallOnsetInputs<'static> {
        StallOnsetInputs {
            feat_ts: &[],
            task_kind_declarations: &[],
            config: StallOnsetConfig::default(),
        }
    }

    fn test_quality_inputs() -> QualityInputs<'static> {
        QualityInputs {
            capabilities_declared: false,
            hook_health: &[],
        }
    }

    fn rows_of(keys: &[KeyPress], paste_ts: &[u64]) -> Vec<FeatureRow> {
        build_feature_rows(
            keys,
            paste_ts,
            &[],
            TEST_MAX_TICKS,
            &test_stall_onset_inputs(),
            &test_quality_inputs(),
        )
        .expect("within tick limit")
    }

    /// issue 008 AC2 (判断 9): **`feat` 行が 1 本も無い入力では `T1a` の `pause_*` が全行 `null`** —— 観測区間が
    /// 確定しないので値で埋めない (onset と同じ向き)。⚠️ `iki_cv` / `surprise` は窓を掛けない列なので、
    /// `feat` 行の有無に依らず**同じ値**になる。★ 陽性対照: 同じ打鍵列に `feat` 行を足すと `pause_*` が出る。
    #[test]
    fn t1a_is_null_without_feat_rows() {
        // pause は 2 本 (0.1 s → 3.0 s の 2.9 s・3.1 s → 6.5 s の 3.4 s)。
        let keys: Vec<KeyPress> = [0, 100, 3_000, 3_100, 6_500, 6_600]
            .iter()
            .map(|&ms| key_at(ms))
            .collect();
        let feat_ts: Vec<u64> = (0..=7).map(|i| i * 1_000).collect();
        let run = |feat: &[u64]| {
            let inputs = StallOnsetInputs {
                feat_ts: feat,
                task_kind_declarations: &[],
                config: StallOnsetConfig::default(),
            };
            build_feature_rows(
                &keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &inputs,
                &test_quality_inputs(),
            )
            .expect("within tick limit")
        };
        let without = run(&[]);
        let with = run(&feat_ts);

        assert!(!without.is_empty());
        for (i, r) in without.iter().enumerate() {
            let d = &r.dispersion;
            assert_eq!(
                (d.pause_p50_ms, d.pause_p90_ms, d.pause_max_ms),
                (None, None, None),
                "tick {i}: feat 行 0 本なのに pause_* に値がある"
            );
        }
        // ★ 陽性対照: feat 行があれば pause_* は出る (= 上の null は入力が pause を持たないからではない)。
        assert!(
            with.iter().any(|r| r.dispersion.pause_max_ms.is_some()),
            "feat 行を足しても pause_max_ms が全行 null (入力が pause を試せていない)"
        );
        // ⚠️ 窓を掛けない列は feat 行の有無で動かない (行数は feat 行が右端を延ばしうるので共通部分だけ比べる)。
        for (i, (a, b)) in without.iter().zip(&with).enumerate() {
            assert_eq!(
                a.dispersion.iki_cv, b.dispersion.iki_cv,
                "tick {i}: iki_cv が feat 行の有無で動いた"
            );
            assert_eq!(
                a.dispersion.surprise, b.dispersion.surprise,
                "tick {i}: surprise が feat 行の有無で動いた"
            );
        }
    }

    /// issue 006: 極 B は `pointer_active_ts` を渡さなければ `Err` (★ baseline の値を極 B の名前で返さない)。
    /// 活動が 0 行 (`Some(&[])`) なら baseline と**恒等**で、長い沈黙の途中に活動があると評価区間が割れる。
    #[test]
    fn pole_b_needs_pointer_activity_and_idle_pointer_equals_baseline() {
        let keys = [key_at(1_000), key_at(1_100)];
        let feat: Vec<u64> = (0..=90).map(|i| i * 1_000).collect();
        let inputs = |pole: Pole| StallOnsetInputs {
            feat_ts: &feat,
            task_kind_declarations: &[],
            config: StallOnsetConfig {
                pole,
                ..StallOnsetConfig::default()
            },
        };
        let run = |pole: Pole, active: Option<&[u64]>| {
            build_feature_rows_with_pole_inputs(
                &keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &inputs(pole),
                &test_quality_inputs(),
                &PoleInputs {
                    pointer_active_ts: active,
                    ..PoleInputs::default()
                },
            )
        };
        let onset_rows =
            |rows: &[FeatureRow]| rows.iter().filter(|r| r.stall_onset.is_onset).count();

        let err = run(Pole::PointerB, None).expect_err("活動の入力を渡さない極 B は Err");
        assert!(err.contains("pointer_b"), "極の名前が出る: {err}");

        let baseline = run(Pole::Baseline, None).expect("baseline は入力が要らない");
        let idle = run(Pole::PointerB, Some(&[])).expect("活動 0 行の極 B");
        assert_eq!(
            serde_json::to_value(&baseline).expect("serialize"),
            serde_json::to_value(&idle).expect("serialize"),
            "活動が 0 行なら極 B は baseline と恒等（★ 値で埋めない）"
        );

        // 沈黙 (1100 → 90000) の途中 (49000, 50000] に活動 → 片が 2 つになり onset が 1 → 2 本。
        let active = run(Pole::PointerB, Some(&[50_000])).expect("活動ありの極 B");
        assert_eq!(onset_rows(&baseline), 1);
        assert_eq!(onset_rows(&active), 2, "抑制が検出へ配線されていない");
        // ⚠️ 極 B が変えるのは検出 (`stall_onset`) と条件 A に依る `restart` だけ。他の Tier は極に依らない。
        for (b, a) in baseline.iter().zip(&active) {
            assert_eq!(
                serde_json::to_value(b.activity).expect("serialize"),
                serde_json::to_value(a.activity).expect("serialize")
            );
            assert_eq!(
                serde_json::to_value(&b.dispersion).expect("serialize"),
                serde_json::to_value(&a.dispersion).expect("serialize")
            );
        }
    }

    /// issue 004: 入力の無い極は**入力が空でも** `Err` (黙って baseline の値を返さない)。
    /// ⚠️ 陰性対照 (`baseline`) が通ることも同じテストで固定する —— 常に `Err` を返す実装を通さない。
    #[test]
    fn unimplemented_pole_is_refused_even_on_empty_input() {
        let with_pole = |pole: Pole| StallOnsetInputs {
            feat_ts: &[],
            task_kind_declarations: &[],
            config: StallOnsetConfig {
                pole,
                ..StallOnsetConfig::default()
            },
        };
        let run = |pole: Pole, keys: &[KeyPress]| {
            build_feature_rows(
                keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &with_pole(pole),
                &test_quality_inputs(),
            )
        };

        // ★ 未実装の極はもう無い (極 M+ は issue 005・極 B は issue 006 で開通した)。ここでは
        //   「入力を渡さない (`build_feature_rows` は `PoleInputs::default()`) 極 B は Err」を見る。
        for keys in [&[][..], &[key_at(0)][..]] {
            let err = run(Pole::PointerB, keys).expect_err("入力を渡さない極 B は Err");
            assert!(err.contains("pointer_b"), "極の名前が出る: {err}");
        }
        assert!(run(Pole::Baseline, &[])
            .expect("baseline は通る")
            .is_empty());
        assert_eq!(
            run(Pole::Baseline, &[key_at(0)])
                .expect("baseline は通る")
                .len(),
            1
        );
    }

    /// issue 005: 極 M+ は**押下区間が渡されなければ** `Err` (入力が空でも)。
    /// ★ `build_feature_rows` (極の入力を持たない) も同じく `Err` —— 渡し忘れて baseline の値を
    /// 極 M+ の名前で返さない。⚠️ 陰性対照: 区間を渡せば (空でも) 通る。
    #[test]
    fn modifier_m_plus_requires_its_intervals() {
        let inputs = StallOnsetInputs {
            feat_ts: &[],
            task_kind_declarations: &[],
            config: StallOnsetConfig {
                pole: Pole::ModifierMPlus,
                ..StallOnsetConfig::default()
            },
        };
        let quality = test_quality_inputs();
        for keys in [&[][..], &[key_at(0)][..]] {
            let err = build_feature_rows(keys, &[], &[], TEST_MAX_TICKS, &inputs, &quality)
                .expect_err("区間の無い極 M+ は Err");
            assert!(err.contains("modifier_m_plus"), "極の名前が出る: {err}");
            let err = build_feature_rows_with_pole_inputs(
                keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &inputs,
                &quality,
                &PoleInputs::default(),
            )
            .expect_err("modifier_intervals が None なら Err");
            assert!(err.contains("modifier"), "{err}");
        }
        let empty = PoleInputs {
            modifier_intervals: Some(&[]),
            ..PoleInputs::default()
        };
        let rows = build_feature_rows_with_pole_inputs(
            &[key_at(0)],
            &[],
            &[],
            TEST_MAX_TICKS,
            &inputs,
            &quality,
            &empty,
        )
        .expect("区間 0 本は有効な入力");
        assert_eq!(rows.len(), 1);
    }

    /// issue 005: 極 M+ が動かすのは**検出 (`stall_onset` / `restart`) だけ**。`T0` / `T1a` / `T1b` / `T1c` / `Q` は
    /// baseline と一致する (⚠️ 極が Tier をまたいで漏れない)。★ 押下区間が空なら検出も恒等。
    #[test]
    fn modifier_m_plus_leaves_the_other_tiers_alone() {
        let ctrl = KeyPress {
            timestamp: 300,
            vk: 0xA2,
        };
        let keys = vec![
            key_at(0),
            key_at(100),
            key_at(200),
            ctrl,
            key_at(400),
            key_at(500),
        ];
        let feat_ts: Vec<u64> = (0..=60).map(|i| i * 1000).collect();
        let run = |pole: Pole, intervals: &[(u64, u64)]| {
            build_feature_rows_with_pole_inputs(
                &keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &StallOnsetInputs {
                    feat_ts: &feat_ts,
                    task_kind_declarations: &[],
                    config: StallOnsetConfig {
                        pole,
                        ..StallOnsetConfig::default()
                    },
                },
                &test_quality_inputs(),
                &PoleInputs {
                    modifier_intervals: Some(intervals),
                    ..PoleInputs::default()
                },
            )
            .expect("within tick limit")
        };
        let baseline = run(Pole::Baseline, &[]);
        // ★ 区間 [300, 600] は 400 / 500 の文字キーを挿入系から外す。
        let m_plus = run(Pole::ModifierMPlus, &[(300, 600)]);
        assert_eq!(baseline.len(), m_plus.len());
        fn json<T: serde::Serialize>(v: &T) -> serde_json::Value {
            serde_json::to_value(v).expect("serialize")
        }
        for (b, m) in baseline.iter().zip(&m_plus) {
            assert_eq!(b.t_ms, m.t_ms);
            assert_eq!(json(&b.activity), json(&m.activity), "T0 は極に依らない");
            assert_eq!(json(&b.boundary), json(&m.boundary), "T1b は極に依らない");
            assert_eq!(
                json(&b.dispersion),
                json(&m.dispersion),
                "T1a は極に依らない"
            );
            assert_eq!(json(&b.revision), json(&m.revision), "T1c は極に依らない");
            assert_eq!(json(&b.quality), json(&m.quality), "Q は極に依らない");
        }
        // ★ 陰性対照: 区間が空の極 M+ は検出も baseline と恒等（`not_applicable` の根拠）。
        let m_plus_empty = run(Pole::ModifierMPlus, &[]);
        for (b, m) in baseline.iter().zip(&m_plus_empty) {
            assert_eq!(json(&b.stall_onset), json(&m.stall_onset));
            assert_eq!(json(&b.restart), json(&m.restart));
        }
    }

    /// Codex r1a-3: 極 M+ が `detection_keys`（挿入系を置き換えた列）を検出へ**実際に配線している**
    /// ことを固定する（★ 押下区間を渡しても検出結果が baseline と変わらないなら、
    /// `build_feature_rows_with_pole_inputs` が `detection_keys` を検出へ渡していない疑いがある）。
    ///
    /// 手計算（`modifier_m_plus_leaves_the_other_tiers_alone` と同じ fixture・`W`=30 秒・
    /// `q`=0.99・`net_diff_min`=1・warmup 無し）:
    /// - 打鍵は 100ms 間隔で `0/100/200/300(Ctrl)/400/500`（Ctrl 以外の 5 本が挿入系 +1）。
    /// - baseline: 5 本すべてが窓に寄与するので、最後の `500` が窓から抜ける `30_500` で
    ///   条件 A が立つ → onset s=30_500。
    /// - M+ (区間 `[300,600]` が `400`/`500` を覆う): 寄与するのは `0/100/200` の 3 本だけになり、
    ///   最後の `200` が窓から抜ける `30_200` で条件 A が立つ → onset s=30_200。
    /// - ★ `detection_keys` が本番経路に配線されていなければ（常に baseline の `keys` を検出へ
    ///   渡していれば）、M+ も `s=30_500` になり baseline と一致してしまう。
    #[test]
    fn modifier_m_plus_intervals_actually_change_detection() {
        let ctrl = KeyPress {
            timestamp: 300,
            vk: 0xA2,
        };
        let keys = vec![
            key_at(0),
            key_at(100),
            key_at(200),
            ctrl,
            key_at(400),
            key_at(500),
        ];
        let feat_ts: Vec<u64> = (0..=60).map(|i| i * 1000).collect();
        let run = |pole: Pole, intervals: &[(u64, u64)]| {
            build_feature_rows_with_pole_inputs(
                &keys,
                &[],
                &[],
                TEST_MAX_TICKS,
                &StallOnsetInputs {
                    feat_ts: &feat_ts,
                    task_kind_declarations: &[],
                    config: StallOnsetConfig {
                        pole,
                        ..StallOnsetConfig::default()
                    },
                },
                &test_quality_inputs(),
                &PoleInputs {
                    modifier_intervals: Some(intervals),
                    ..PoleInputs::default()
                },
            )
            .expect("within tick limit")
        };
        let onset_s = |rows: &[FeatureRow]| -> Vec<u64> {
            rows.iter()
                .filter(|r| r.stall_onset.is_onset)
                .filter_map(|r| r.stall_onset.s_ms)
                .collect()
        };

        let baseline = run(Pole::Baseline, &[]);
        let m_plus = run(Pole::ModifierMPlus, &[(300, 600)]);

        assert_eq!(
            onset_s(&baseline),
            vec![30_500],
            "baseline は 500 まで届いて抜ける 30_500 で立つ"
        );
        assert_eq!(
            onset_s(&m_plus),
            vec![30_200],
            "M+ は 400/500 を数えないので 200 が抜ける 30_200 で立つ \
             （★ detection_keys が配線されていない場合は 30_500 になる）"
        );
        assert_ne!(
            onset_s(&baseline),
            onset_s(&m_plus),
            "極 M+ の押下区間を渡しても検出結果が変わっていない（detection_keys の配線漏れの疑い）"
        );
    }

    /// 境界入力で `build_feature_rows` が panic しないことを確認する (AC10)。
    #[test]
    fn tick_loop_handles_degenerate_inputs() {
        // 空ログ
        assert!(rows_of(&[], &[]).is_empty());

        // 単一 key 行
        let rows = rows_of(&[key_at(0)], &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].activity.keys, 1);
        assert_eq!(rows[0].activity.mean_iki_ms, None);

        // 同一 timestamp の多発
        let same_ts = vec![key_at(500), key_at(500), key_at(500)];
        let rows = rows_of(&same_ts, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].activity.keys, 3);
        // 間隔 0ms が2件 → 平均0、pause にはならない
        assert_eq!(rows[0].activity.mean_iki_ms, Some(0.0));
        assert_eq!(rows[0].activity.pauses, 0);

        // 時刻逆行 (入力順が時刻降順) でも panic せず、昇順として扱われる
        let reversed = vec![key_at(3000), key_at(1000), key_at(0)];
        let rows = rows_of(&reversed, &[]);
        assert_eq!(rows.len(), 4); // ticks 0..=3 (origin=0, t_last=3000)
        let total_keys: u64 = rows.iter().map(|r| r.activity.keys).sum();
        assert_eq!(total_keys, 3);

        // tick 境界ちょうどの打鍵 (0, 1000, 2000 の3点)
        let boundary = vec![key_at(0), key_at(1000), key_at(2000)];
        let rows = rows_of(&boundary, &[]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].activity.keys, 1);
        assert_eq!(rows[1].activity.keys, 1);
        assert_eq!(rows[2].activity.keys, 1);

        // 最後の pause が閉じないまま終わるログ: 3000ms 空けて終了しても pause は計上されない
        // (再開する2打目が無いため)
        let unclosed = vec![key_at(0), key_at(100)];
        let rows = rows_of(&unclosed, &[]);
        let total_pauses: u64 = rows.iter().map(|r| r.activity.pauses).sum();
        assert_eq!(total_pauses, 0);

        // paste だけの入力 (keys が空) —— tick グリッドは paste 時刻からも張られる
        let rows = rows_of(&[], &[0, 2500]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].activity.pastes, 1);
        assert_eq!(rows[2].activity.pastes, 1);
        assert_eq!(rows[0].activity.keys, 0);
    }

    /// ⚠️★★★ 時刻スパンに比例した確保を上限で止める (2026-09-15・レビュー HIGH)。
    /// ★ 入力は 2 件でも、時刻差が大きければ tick 数は跳ね上がる。
    #[test]
    fn tick_loop_rejects_time_span_beyond_limit() {
        // 0ms と 1,000,000ms → tick 0..=1000 の 1001 tick が要る
        let huge = vec![key_at(0), key_at(1_000_000)];
        let inputs = test_stall_onset_inputs();
        let q_inputs = test_quality_inputs();
        assert!(build_feature_rows(&huge, &[], &[], 1001, &inputs, &q_inputs).is_ok());
        let err = build_feature_rows(&huge, &[], &[], 1000, &inputs, &q_inputs)
            .expect_err("上限超過は Err");
        assert!(err.contains("time span too large"), "err = {}", err);
    }

    /// MEDIUM-3: `feat` 行が最初の入力イベントより前・最後の入力イベントより後に
    /// またがる入力で、原点は入力イベント (`feat` では動かない) のまま、最後の行は
    /// 最後の `feat` 行を覆うこと (issue 008・mod.rs doc「tick への帰属規則」)。
    #[test]
    fn grid_end_extends_to_feat_but_origin_stays_at_first_input_event() {
        let epoch = 1_700_000_000_000_u64;
        let keys = vec![key_at(epoch + 5_000), key_at(epoch + 5_100)];
        // feat 行は入力イベントより前 (epoch) と後 (epoch+20_000) の両方にまたがる。
        let feat_ts = vec![epoch, epoch + 20_000];
        let inputs = StallOnsetInputs {
            feat_ts: &feat_ts,
            task_kind_declarations: &[],
            config: StallOnsetConfig::default(),
        };
        let rows = build_feature_rows(
            &keys,
            &[],
            &[],
            TEST_MAX_TICKS,
            &inputs,
            &test_quality_inputs(),
        )
        .expect("within tick limit");

        assert_eq!(
            rows[0].t_ms,
            epoch + 5_000,
            "原点は最初の入力イベント時刻のまま (feat 行では動かない)"
        );
        let last_row_end = rows.last().unwrap().t_ms + ENGINE_TICK_MS;
        assert!(
            last_row_end > epoch + 20_000,
            "最後の行は最後の feat 行 (epoch+20000) を覆うはず: last_row_end={last_row_end}"
        );
    }

    /// ★ `t_ms` が**絶対 epoch ms** であること (⚠️ セッション相対にしない)。
    #[test]
    fn t_ms_is_absolute_epoch_not_session_relative() {
        let epoch = 1_700_000_000_000_u64;
        let rows = rows_of(&[key_at(epoch), key_at(epoch + 1500)], &[]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].t_ms, epoch);
        assert_eq!(rows[1].t_ms, epoch + ENGINE_TICK_MS);
    }

    /// ★ 中心規則: tick 境界をまたぐ間隔は**終了側 (再開側) の tick** に乗る。
    #[test]
    fn interval_is_attributed_to_the_resuming_tick() {
        // ⚠️ グリッド原点は**最初のイベント時刻**なので、0ms から張って境界をまたがせる。
        // 0 / 900 / 1100 → 間隔 900ms は tick 0、tick 境界をまたぐ 200ms は tick 1 (= 再開側)。
        let rows = rows_of(&[key_at(0), key_at(900), key_at(1100)], &[]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].activity.mean_iki_ms, Some(900.0));
        assert_eq!(rows[1].activity.mean_iki_ms, Some(200.0));

        // 0ms と 2500ms の 2 打 → pause は再開側の tick 2 に 1 件
        let rows = rows_of(&[key_at(0), key_at(2500)], &[]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].activity.pauses, 0);
        assert_eq!(rows[2].activity.pauses, 1);
    }

    #[test]
    fn extract_pause_resumptions_uses_ssot_threshold() {
        use crate::analysis::features::PAUSE_THRESHOLD_MS;
        let press_ts = vec![0, PAUSE_THRESHOLD_MS, PAUSE_THRESHOLD_MS + 500];
        let resumptions = extract_pause_resumptions(&press_ts);
        // 0 -> PAUSE_THRESHOLD_MS は閾値ちょうどで pause、続く +500 は pause でない
        assert_eq!(resumptions, vec![PAUSE_THRESHOLD_MS]);
    }

    #[test]
    fn net_production_counts_insert_minus_delete() {
        let backspace = KeyPress {
            timestamp: 0,
            vk: 0x08,
        };
        let letter = KeyPress {
            timestamp: 1,
            vk: 0x41,
        };
        let arrow = KeyPress {
            timestamp: 2,
            vk: 0x25, // navigation, どちらでもない
        };
        assert_eq!(net_production(&[letter, letter, backspace]), 1);
        assert_eq!(net_production(&[backspace, backspace]), -2);
        assert_eq!(net_production(&[arrow]), 0);
    }

    /// issue 007: `build_feature_rows` が渡された `ime_events` を `stall_onset.ime_overlap` へ配線する。
    /// ★ 引数を落としても onset の集合は変わらない (数えるだけで抑制しない) ので、**値そのもの**を固定する。
    #[test]
    fn ime_events_reach_stall_onset_overlap_without_changing_onsets() {
        // onset は s = 30_200, t0 = 30_300 (`stall_onset` の小さい合成入力と同じ)。
        let keys = [key_at(0), key_at(100), key_at(200)];
        let feat: Vec<u64> = (0..=41).map(|i| i * 1_000).collect();
        let inputs = StallOnsetInputs {
            feat_ts: &feat,
            task_kind_declarations: &[],
            config: StallOnsetConfig::default(),
        };
        let run = |ime_events: &[ImeShadowEvent]| {
            build_feature_rows(
                &keys,
                &[],
                ime_events,
                TEST_MAX_TICKS,
                &inputs,
                &test_quality_inputs(),
            )
            .expect("within tick limit")
        };
        let overlapping = ImeShadowEvent {
            t_ms: 30_250,
            dwell_ms: 100,
            nav: 0,
            abandon: false,
        };
        let with = run(&[overlapping]);
        let without = run(&[]);

        let onset_of = |rows: &[FeatureRow]| {
            rows.iter()
                .map(|r| {
                    (
                        r.stall_onset.is_onset,
                        r.stall_onset.s_ms,
                        r.stall_onset.t0_ms,
                    )
                })
                .collect::<Vec<_>>()
        };
        // ★ 行は打鍵の範囲の内側にあり、tick グリッドの原点・終端を動かさない (動かす入力は本テストの対象外)。
        assert_eq!(
            onset_of(&with),
            onset_of(&without),
            "ime_shadow 行の有無で onset の集合が変わった (抑制してしまっている)"
        );
        let idx = with
            .iter()
            .position(|r| r.stall_onset.is_onset)
            .expect("onset が立つ");
        assert_eq!(with[idx].stall_onset.ime_overlap, Some(true));
        assert_eq!(without[idx].stall_onset.ime_overlap, Some(false));
        assert!(
            with.iter()
                .filter(|r| !r.stall_onset.is_onset)
                .all(|r| r.stall_onset.ime_overlap.is_none()),
            "onset でない行は null"
        );
    }
}
