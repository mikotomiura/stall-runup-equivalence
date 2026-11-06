//! Phase 1 Task 2: UserLabel × FeatSnapshot の突き合わせメトリクス算出。
//!
//! すべて純粋関数で構成されており、副作用を持たない (analysis/ 純粋性維持)。
//! `recovered_at` 等の派生値はログ記録時ではなく本モジュールで計算する設計のため、
//! 閾値変更時に過去セッションを再処理するだけで再計算できる。

use serde::Serialize;

use crate::analysis::intervention::AWAY_IDLE_MS;
use crate::analysis::loop_step::FeatSnapshot;

/// Stuck ラベル時刻以降に `p_flow > 0.5` への遷移を探す既定ウィンドウ: 15 分。
///
/// ⚠️ `OutcomeConfig::pflow_exit_window_ms` (既定 **5 分**) とは**別の値**である。
/// こちらを使うのは `compute_session_metrics` (GT ラベル経路) と
/// `intervention_eval` の defer/DC 経路だけで、`OutcomeMetrics` の算出には使わない。
pub const DEFAULT_PFLOW_EXIT_WINDOW_MS: u64 = 15 * 60 * 1000;

/// 回復判定の既定閾値: `p_flow > 0.5`。
pub const DEFAULT_P_FLOW_THRESHOLD: f64 = 0.5;

/// HMM argmax とラベル時刻を突き合わせる近接窓: ±1 秒。
pub const LABEL_MATCH_WINDOW_MS: u64 = 1_000;

/// リード/ラグ探索窓: ラベル時刻から ±60 秒。
pub const LEAD_LAG_WINDOW_MS: u64 = 60_000;

/// GT ラベル種別（replay.rs の ReplayEvent::UserLabel から復号される）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelClass {
    Flow,
    Incubation,
    Stuck,
}

impl LabelClass {
    /// NDJSON の `label` フィールド文字列から変換。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flow" => Some(Self::Flow),
            "incubation" => Some(Self::Incubation),
            "stuck" => Some(Self::Stuck),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Flow => 0,
            Self::Incubation => 1,
            Self::Stuck => 2,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Flow => "Flow",
            Self::Incubation => "Incubation",
            Self::Stuck => "Stuck",
        }
    }
}

const CLASSES: [LabelClass; 3] = [LabelClass::Flow, LabelClass::Incubation, LabelClass::Stuck];

/// UserLabel の一点イベント表現（replay.rs から渡される）。
#[derive(Debug, Clone, Copy)]
pub struct UserLabelEvent {
    pub timestamp: u64,
    pub label: LabelClass,
}

/// 3×3 混同行列。`matrix[i][j]` は「正解クラス i、推定クラス j」の件数。
/// 行列の順序は [Flow, Incubation, Stuck]。
#[derive(Debug, Clone, Serialize)]
pub struct ConfusionMatrix {
    pub matrix: [[u32; 3]; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassMetrics {
    pub class: LabelClass,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub support: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMetrics {
    pub total_labels: u32,
    pub confusion: ConfusionMatrix,
    pub per_class: [ClassMetrics; 3],
    pub macro_f1: f64,
    pub micro_f1: f64,
    /// HMM の検出リード (正 = HMM が先行) / ラグ (負 = 遅延) の中央値（ms）。
    /// ラベルごとに ±60 秒窓内で「同クラスが argmax になった最初の時刻」を求める。
    pub detection_lead_ms_median: Option<i64>,
    /// Stuck → p_flow > 0.5 到達までの時間の中央値（ms）。
    pub stuck_recovery_ms_median: Option<u64>,
    /// v0.4 アウトカム指標 (a)AI依存度 / (b)自力解決率。`--metrics` 経路で算出して
    /// 付与する。HMM 精度メトリクス (上記) とは独立で、key/paste 行が無い経路
    /// (`compute_range_metrics` 等) では `None`。additive・後方互換 (既存利用側不変)。
    pub outcome: Option<OutcomeMetrics>,
    pub warnings: Vec<String>,
}

// ── v0.4 Outcome metrics (a)AI依存度 / (b)自力解決率 ────────────────────────
//
// 設計: `.steering/20260623-outcome-metrics/design.md` (v2 採用)。
// 「Stuck エピソード」を基盤抽象にし、その上に (a)(b) を載せる。トリガは
// `StuckTrigger` で外出しし (既定 = p_stuck 閾値)、研究設計フォークをコードに
// 焼き込まない。すべて content-free (vk コード / 文字数 / 確率のみ。Unicode 内容に
// 触れない = data-privacy ルール1)。

/// Stuck 候補の判定トリガ。既定は `PStuckThreshold` (全セッションで算出可能)。
/// `UserLabel` / `InterventionFired` は再処理で切替可能だが既定にしない
/// (`.steering/20260623-outcome-metrics/decisions.md` CD-OM1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StuckTrigger {
    /// `p_stuck >= p_stuck_trigger` の feat を Stuck とみなす (既定)。
    PStuckThreshold,
    /// `user_label == stuck` を Stuck とみなす (GT。較正セッションのみ)。
    UserLabel,
    /// 介入発火 (`intervention_decision.action != none`) を Stuck とみなす。
    InterventionFired,
}

/// アウトカム指標の算出パラメータ。既定値は研究設計上の暫定値であり、
/// 再処理で調整可能 (metrics.rs の「閾値変更時に過去セッションを再処理するだけで
/// 再計算できる」哲学に従う)。ユーザー承認 2026-06-23。
#[derive(Debug, Clone, Copy)]
pub struct OutcomeConfig {
    /// Stuck 候補の判定トリガ。
    pub trigger: StuckTrigger,
    /// `PStuckThreshold` 時の閾値。Wall トリガ (p_stuck>0.70) より緩め = 候補を広く拾い
    /// 回復判定で絞る。
    pub p_stuck_trigger: f64,
    /// 連続 Stuck feat をひとつのエピソードに統合する最大間隙 (ms)。
    /// 1 ローリングウィンドウ = 30s。瞬間的な p_stuck 低下を同一エピソードに吸収する。
    pub merge_gap_ms: u64,
    /// エピソード後、`p_flow > 0.5` への遷移を探す窓 (ms)。handoff 指定の 5 分。
    /// ⚠️ 定数 `DEFAULT_PFLOW_EXIT_WINDOW_MS` (15 分) とは別値。
    pub pflow_exit_window_ms: u64,
    /// 「net_diff 正進行」とみなす回復窓内 manual_insert_chars 純増の下限 (文字)。
    /// 既定 1 = 最小解釈 (純増 1 文字以上で前進)。ユーザー承認 2026-06-23。
    pub net_diff_min: u64,
    /// 「large paste」とみなす external_paste_chars の下限 (文字)。
    pub large_threshold: u64,
    /// 「after_stuck」= エピソード後この窓 (ms) 内の paste を stuck 起因とみなす。
    pub after_window_ms: u64,
    /// 「離席候補」とみなす隣接打鍵間隙の下限 (ms)。`receipt_free_resumption` 専用。
    ///
    /// ⚠️★★★ **既定値の SSOT は `analysis::intervention::AWAY_IDLE_MS` である。**
    /// ★ ここで 180_000 を再定義しない (ADR-024 AC-3・`away_gap_default_matches_intervention_ssot`)。
    /// ⚠️ 系が「離席した」と判定するわけではない (**ADR-018 D1**)。事後の層別境界にすぎない。
    pub away_gap_ms: u64,
}

impl Default for OutcomeConfig {
    fn default() -> Self {
        Self {
            trigger: StuckTrigger::PStuckThreshold,
            p_stuck_trigger: 0.60,
            merge_gap_ms: 30_000,
            pflow_exit_window_ms: 5 * 60 * 1000,
            net_diff_min: 1,
            large_threshold: 50,
            after_window_ms: 120_000,
            away_gap_ms: AWAY_IDLE_MS,
        }
    }
}

/// content-free な paste イベント (文字数のみ。中身は持たない)。
#[derive(Debug, Clone, Copy)]
pub struct PasteEvent {
    pub timestamp: u64,
    pub chars: u64,
}

/// content-free な key press (vk コードのみ。data-privacy ルール1 で記録許可)。
#[derive(Debug, Clone, Copy)]
pub struct KeyPress {
    pub timestamp: u64,
    pub vk: u32,
}

/// Stuck エピソード = `p_stuck >= 閾値` (または他トリガ) が `merge_gap_ms` 以内の
/// 中断を挟んで連続する区間。`end_ms` 以降に回復を探す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StuckEpisode {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// v0.4 アウトカム指標。すべて数値のみ (本文・キーストローク内容を含まない)。
/// 算出不能 (分母 0) の比率は `None` (NaN を出さない)。
#[derive(Debug, Clone, Serialize)]
pub struct OutcomeMetrics {
    // ── (b) 自力解決率 ──
    /// 文字生成キー press − backspace/delete press (0 下限)。content-free な近似 (proxy)。
    /// **注意 (cross-review AGREE-3)**: NDJSON の key 行は修飾キー同時押し状態を持たないため、
    /// `Ctrl+V` / `Cmd+V` の `V` (0x56) も 1 文字として計上される。external paste 1 回ごとに
    /// manual を +1 程度過大評価し、`manual_contribution_ratio` を僅かに manual 寄りに偏らせる。
    /// 解析時は外れ値として扱う前提 (capture-schema-freeze CD5 A3 と同様の注意)。
    pub manual_insert_chars: u64,
    /// external_paste_chars の総和 (capture 済 paste のみ)。
    pub external_paste_chars_total: u64,
    /// `manual / (manual + paste)`。分母 0 → None。
    pub manual_contribution_ratio: Option<f64>,
    /// `p_flow` が 0.5 を超えたエピソードのうち、「受領 (external paste) 無し
    /// かつ manual_insert_chars 純増 >= net_diff_min」だった割合。
    /// 分母 = `pflow_exit_episode_count`。0 → None。
    ///
    /// ⚠️★★★ **推定器依存 (ADR-023 D7 / ADR-024)**: 「抜けた」の判定は
    /// 3 クラス事後確率 `p_flow` であり、トリガー (`p_stuck >= 0.60`) と
    /// **同じ 3 次元単体の相補的な領域**である。★ 報告時は `receipt_free_resumption`
    /// および `manual_contribution_ratio` と必ず併記すること。
    pub pflow_exit_manual_gain_rate: Option<f64>,
    // ── (a) AI依存度 ──
    /// capture 済 external paste (external_paste_chars が Some) の件数。
    pub external_paste_count: u32,
    /// 文字数欠落 (旧 NDJSON, capture-schema-freeze 以前) のため outcome 集計から
    /// 除外した paste の件数。これが大きいセッションは manual_contribution_ratio が
    /// manual 寄りに偏るため、研究者が偏りに気づけるよう数値で残す
    /// (code-reviewer MEDIUM 対応・収集時に算出して付与)。
    pub excluded_paste_count: u32,
    /// `external_paste_chars >= large_threshold` の paste 割合。paste 0 → None。
    pub large_paste_rate: Option<f64>,
    /// エピソード後 after_window 内に paste があったエピソードの割合。episode 0 → None。
    pub paste_after_stuck_rate: Option<f64>,
    /// `pflow_exit_window_ms` 内に受領 (external paste) 無しで `p_flow > 0.5` へ
    /// 抜けたエピソードの割合 (分母 = **全エピソード**)。episode 0 → None。
    ///
    /// ⚠️★★★ **旧名 `self_recovery_without_paste`** (ADR-024 D2 で改名)。
    /// ★ 定義は変えていない。名前が「助けを借りずに解いた」と読めることが害だった
    /// (ADR-023 D6 / [P126])。
    ///
    /// ⚠️★★★ **本指標は副次である。主要アウトカムは `receipt_free_resumption`。**
    /// 推定器依存が **2 経路**ある (ADR-024 §計器の問題):
    /// 1. 「抜けた」の判定が `p_flow`(推定器) であり、トリガー `p_stuck` と相補。
    /// 2. ⚠️★★★ **受領の有無を見る窓 `[end_ms, rec_ts]` の右端そのものが `p_flow` の
    ///    立ち上がり速度で決まる** —— p_flow が速く上がるほど窓が縮み、
    ///    paste を拾う機会が減って本指標が**機械的に上がる**。
    pub pflow_exit_without_receipt: Option<f64>,
    /// 補助指標: 1 時間あたりの external paste 回数。span 0 → None。
    pub paste_count_per_hour: Option<f64>,
    // ── 透明性のための内訳 ──
    /// トリガー (`p_stuck >= p_stuck_trigger`) で切り出したエピソード数。
    /// ★ `pflow_exit_without_receipt` と `paste_after_stuck_rate` の分母。
    /// ⚠️ `pflow_exit_manual_gain_rate` の分母は **`pflow_exit_episode_count` のほう**である
    /// (★ ADR-023 の指摘 C がまさにこの違いを問題にしていた)。
    pub stuck_episode_count: u32,
    /// うち `pflow_exit_window_ms` 内に `p_flow > 0.5` へ抜けたエピソード数。
    /// ⚠️ 旧名 `recovered_episode_count` (ADR-024 D2)。★ 「解決した」ではない。
    pub pflow_exit_episode_count: u32,
    /// ★★★ **主要アウトカム (ADR-024)**: 離席候補明けの再開を起点に、
    /// 受領 (external paste) の累積発生関数 (CIF) を competing risks で推定したもの。
    /// ⚠️★★★ **推定器 (`p_flow`) を一切通らない。**
    pub receipt_free_resumption: ReceiptFreeResumption,
}

/// vk コードが「文字生成キー」(印字可能文字を生む) かを content-free に判定する。
/// Windows VK 規約 (macOS はフック層で変換済み)。Unicode 内容には一切触れない。
/// 含む: 英数 (0x30-0x39, 0x41-0x5A) / Space / Enter / Tab / テンキー (0x60-0x6F) /
/// OEM 句読点 (0xBA-0xC0, 0xDB-0xDF, 0xE2)。除外: 修飾・ナビゲーション・ファンクション・
/// backspace/delete。エピソード/比率の分子分母を安定させるための保守的集合
/// (`.steering/20260623-outcome-metrics/decisions.md` CD-OM2)。
pub fn is_text_producing(vk: u32) -> bool {
    matches!(vk,
        0x30..=0x39   // '0'-'9'
        | 0x41..=0x5A // 'A'-'Z'
        | 0x20        // VK_SPACE
        | 0x0D        // VK_RETURN (改行も文字)
        | 0x09        // VK_TAB
        | 0x60..=0x6F // テンキー 0-9 と * + . - / separator
        | 0xBA..=0xC0 // OEM_1..OEM_3 (; = , - . / `)
        | 0xDB..=0xDF // OEM_4..OEM_8 ([ \ ] ')
        | 0xE2        // OEM_102 (\ / <>)
    )
}

/// backspace / delete か (replay.rs `is_backspace` と同義。文字を減らすキー)。
pub fn is_backspace(vk: u32) -> bool {
    vk == 0x08 || vk == 0x2E // VK_BACK / VK_DELETE
}

/// feat 列を Stuck エピソードに分割する。**現状 `PStuckThreshold` のみ実装** している。
/// `feats` は timestamp 昇順を仮定。`merge_gap_ms` 以内の中断は同一エピソードに統合。
///
/// `UserLabel` / `InterventionFired` は feat だけでは判定できず (ラベル時刻列・介入発火列を
/// 要する) 将来拡張のため予約済み。それらが指定された場合は p_stuck の結果を黙って返さず
/// **空を返す** (cross-review AGREE-1: 仕様前走り防止)。トリガ別の時刻列から組む共通部は
/// `episodes_from_sorted_ts` に切り出してあり、将来 `segment_episodes_from_ts(&label_ts, cfg)`
/// 等を足せる。
pub fn segment_stuck_episodes(feats: &[FeatSnapshot], cfg: &OutcomeConfig) -> Vec<StuckEpisode> {
    if cfg.trigger != StuckTrigger::PStuckThreshold {
        return Vec::new();
    }
    let stuck_ts: Vec<u64> = feats
        .iter()
        .filter(|f| f.p_stuck >= cfg.p_stuck_trigger)
        .map(|f| f.timestamp)
        .collect();
    episodes_from_sorted_ts(&stuck_ts, cfg.merge_gap_ms)
}

/// 昇順タイムスタンプ列を merge_gap で統合してエピソード化する (トリガ非依存の共通部)。
fn episodes_from_sorted_ts(sorted_ts: &[u64], merge_gap_ms: u64) -> Vec<StuckEpisode> {
    let mut episodes = Vec::new();
    let mut iter = sorted_ts.iter().copied();
    if let Some(first) = iter.next() {
        let mut start = first;
        let mut prev = first;
        for ts in iter {
            if ts.saturating_sub(prev) <= merge_gap_ms {
                prev = ts;
            } else {
                episodes.push(StuckEpisode {
                    start_ms: start,
                    end_ms: prev,
                });
                start = ts;
                prev = ts;
            }
        }
        episodes.push(StuckEpisode {
            start_ms: start,
            end_ms: prev,
        });
    }
    episodes
}

/// 窓 `[lo, hi]` 内の manual_insert_chars 純増 = 文字生成キー press − backspace press
/// (0 下限)。content-free。
fn manual_insert_chars_in(keys: &[KeyPress], lo: u64, hi: u64) -> u64 {
    let mut text = 0u64;
    let mut back = 0u64;
    for k in keys {
        if k.timestamp >= lo && k.timestamp <= hi {
            if is_backspace(k.vk) {
                back += 1;
            } else if is_text_producing(k.vk) {
                text += 1;
            }
        }
    }
    text.saturating_sub(back)
}

/// 窓 `[lo, hi]` 内に paste が 1 件以上あるか。
fn has_paste_in(pastes: &[PasteEvent], lo: u64, hi: u64) -> bool {
    pastes
        .iter()
        .any(|p| p.timestamp >= lo && p.timestamp <= hi)
}

// ─────────────────────────────────────────────────────────────────────────────
// receipt_free_resumption (ADR-024) — 推定器 (p_flow) を一切通らない主要アウトカム
// ─────────────────────────────────────────────────────────────────────────────
//
// ★ 構造 (ADR-024 D1 / D4):
//   トリガー   = Stuck エピソード          … `p_stuck >= 0.60` (★ 実運用で使うと明言された量)
//   イベント   = 離席候補ギャップ明けの初打鍵 … **時計の事実**
//   アウトカム = 受領 (external paste) の有無  … **外部の事実**
//
// ⚠️★★★ `p_flow` は 1 か所も現れない。これが `pflow_exit_without_receipt` との違いである。

/// `receipt_free_resumption` の観測地平 (ms)。ADR-023 D6 が 5 分 / 10 分と定めた。
pub const RECEIPT_HORIZON_5MIN_MS: u64 = 5 * 60 * 1000;
/// 同上 (10 分)。
pub const RECEIPT_HORIZON_10MIN_MS: u64 = 10 * 60 * 1000;

/// 離席候補ギャップ明けの「再開」1 件。
///
/// ⚠️★★★ **系が「離席した」と判定した記録ではない** (**ADR-018 D1**)。
/// 保持するのは「打鍵が `away_gap_ms` 以上途切れ、また打鍵が始まった」という**時計の事実**だけ。
/// ★ 同じ観測形になるが身体的回帰でないもの (席で紙を読む / 電話 / 別デバイス) を
/// 区別する手段は無い。→ ★ **名前は `resumption`(再開) であって `physical_return` ではない。**
///
/// ⚠️★★★ **`Serialize` を意図的に derive しない** (`security-checker` LOW-1・2026-09-07)。
/// ★ 本型は**絶対時刻を 3 本**持つので、derive すると出力側に 1 行足すだけで
/// 「離席候補のタイムライン」が JSON に載りうる。★ **出せないことを型で担保する。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumptionEvent {
    /// ギャップ直前の最後の打鍵時刻 (= ギャップ開始)。
    pub gap_start_ms: u64,
    /// ギャップ明けの最初の打鍵時刻 (= 時間原点 t0)。
    pub resumed_at_ms: u64,
    /// 紐づけ先 Stuck エピソードの終端。
    pub episode_end_ms: u64,
    /// 紐づけ先 Stuck エピソードの添字。
    ///
    /// ⚠️ 寄与エピソード数は**これ**で数える (`code-reviewer`(Codex) High・2026-09-07)。
    /// ★ `episode_end_ms` で dedup すると、終端が同値のエピソードが 2 本あったときに
    /// 過小計上する。★ 現状 `segment_stuck_episodes` は終端の重複を作らないが、
    /// `episodes` は引数で渡ってくるので**呼び出し側の性質に依存しない形**にする。
    pub episode_index: usize,
}

/// 再開後に最初に起きたこと。競合リスクの 3 分類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumptionOutcomeKind {
    /// 主イベント: 受領 (external paste) が起きた。
    Receipt,
    /// 競合イベント: 受領を見ないまま、**次の離席候補ギャップ**が始まった。
    /// ⚠️★★★ 「また離席した」ではない —— ★ 系は離席を判定しない (**ADR-018 D1**)。
    /// ★ 観測しているのは「打鍵が再び `away_gap_ms` 以上途切れた」ことだけである。
    NextAwayCandidate,
    /// 打ち切り: どちらも起きないまま観測が終わった。
    Censored,
}

/// 再開 1 件の観測結果 (時間原点からの経過 ms と分類)。
///
/// ★ `Serialize` する。⚠️★★★ **セッション別 CIF の平均は Aalen-Johansen ではない**ので、
/// 被験者を跨いでプールするには **事象単位の (経過時間, 分類) が要る** (`code-reviewer` M1)。
/// ★ 持つのは **原点からの相対時間**だけで、絶対時刻も文字内容も含まない
/// (`data-privacy` ルール 1 に抵触しない。⚠️ 絶対時刻を持つ [`ResumptionEvent`] は出さない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResumptionOutcome {
    /// 時間原点 (= 再開時刻) からの経過 ms。
    pub elapsed_ms: u64,
    pub kind: ResumptionOutcomeKind,
}

/// 追跡時間ゼロの事象 (= 再開した瞬間に次の離席候補ギャップへ入った、または観測が終わった)。
///
/// ⚠️★★★ **CIF から除外する** (`code-reviewer` H2)。★ 除外しないと、
/// `t=0` の競合事象が受領より先にリスク集合を食い潰し、**`receipt_free` が 1.0 へ寄る** ——
/// 「戻ってきて 1 打鍵してまた消えた」が満点になってしまう。
/// ★ 受領は `t=0` でも情報を持つ (戻って即座に受領した) ので除外しない。
fn is_degenerate(o: &ResumptionOutcome) -> bool {
    o.elapsed_ms == 0 && o.kind != ResumptionOutcomeKind::Receipt
}

/// 地平時点で追跡が残っている事象数。★ **読み手が外挿を棄却するための数値**である。
fn n_at_risk(outcomes: &[ResumptionOutcome], horizon_ms: u64) -> u32 {
    outcomes
        .iter()
        .filter(|o| o.elapsed_ms >= horizon_ms)
        .count() as u32
}

/// ★★★ 主要アウトカム (ADR-024)。⚠️★★★ **`p_flow` に一切依存しない。**
///
/// ★ 報告時は **必ず** `manual_contribution_ratio` (推定に一切依存しない対照) を併記し、
/// 副次の `pflow_exit_without_receipt` との差を示すこと (ADR-023 D7-2 / ADR-024 D2)。
#[derive(Debug, Clone, Serialize)]
pub struct ReceiptFreeResumption {
    /// 事象数 = 離席候補明けの再開のうち Stuck エピソードに紐づいたもの。
    pub event_count: u32,
    /// 事象に寄与した Stuck エピソード数。
    /// ★ 1 エピソードが事象数を支配していないかを読み手が見られるように出す。
    pub source_episode_count: u32,
    /// 主イベント (受領) の件数。
    pub receipt_count: u32,
    /// 競合イベント (次の離席候補ギャップの開始) の件数。⚠️ 「また離席した」ではない。
    pub next_away_candidate_count: u32,
    /// 打ち切り件数。
    pub censored_count: u32,
    /// ⚠️ 追跡時間ゼロで CIF から除外した事象数 (→ [`is_degenerate`])。
    /// ★ 大きいときは「戻ってすぐまた離れた」が多いということであり、**指標より先にこれを見る**。
    pub degenerate_count: u32,
    /// 5 分地平で追跡が残っている事象数。⚠️★★★ **0 のときは外挿である**。
    pub n_at_risk_at_5min: u32,
    /// 10 分地平で追跡が残っている事象数。⚠️★★★ **0 のときは外挿である**。
    pub n_at_risk_at_10min: u32,
    /// `1 - CIF_receipt(5 min)`。事象 0 / 地平まで追跡なし → None (NaN を出さない)。
    pub receipt_free_at_5min: Option<f64>,
    /// `1 - CIF_receipt(10 min)`。同上。
    pub receipt_free_at_10min: Option<f64>,
    /// 事象単位の観測結果 (原点からの相対時間 ＋ 分類)。
    ///
    /// ★ **被験者を跨いで CIF をプールするために出す** (`code-reviewer` M1) ——
    /// ⚠️★★★ セッション別 CIF の単純平均は Aalen-Johansen ではない。
    /// ★ 絶対時刻を含まないので `data-privacy` ルール 1 に抵触しない。
    pub outcomes: Vec<ResumptionOutcome>,
}

/// 打鍵列から「`away_gap_ms` 以上の間隙 → 再開」を抽出し、Stuck エピソードに紐づける。
///
/// **紐づけ規則**: ギャップ開始が `[ep.start_ms, ep.end_ms + after_window_ms]` に入るとき、
/// そのギャップをそのエピソードに紐づける。条件を満たすエピソードが複数あるときは
/// **最も遅いもの**を採る。★ 新しいノブを増やさず、既存の `after_window_ms`
/// (逐語「エピソード後この窓 (ms) 内の paste を stuck 起因とみなす」) の意味を再利用する。
///
/// `keys` / `episodes` はともに timestamp 昇順を仮定。
/// ⚠️ 推定器 (`p_flow` / `p_stuck`) を**この関数の中では**参照しない
/// (★ `episodes` の切り出しには `p_stuck` が使われている = トリガー側)。
pub fn segment_resumptions(
    episodes: &[StuckEpisode],
    keys: &[KeyPress],
    cfg: &OutcomeConfig,
) -> Vec<ResumptionEvent> {
    debug_assert!(
        keys.windows(2).all(|w| w[0].timestamp <= w[1].timestamp),
        "segment_resumptions: keys must be sorted by timestamp"
    );
    debug_assert!(
        episodes.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
        "segment_resumptions: episodes must be sorted by start_ms"
    );
    let mut out = Vec::new();
    for w in keys.windows(2) {
        let gap_start_ms = w[0].timestamp;
        let resumed_at_ms = w[1].timestamp;
        if resumed_at_ms.saturating_sub(gap_start_ms) < cfg.away_gap_ms {
            continue;
        }
        let linked = episodes.iter().enumerate().rev().find(|(_, ep)| {
            gap_start_ms >= ep.start_ms
                && gap_start_ms <= ep.end_ms.saturating_add(cfg.after_window_ms)
        });
        if let Some((episode_index, ep)) = linked {
            out.push(ResumptionEvent {
                gap_start_ms,
                resumed_at_ms,
                episode_end_ms: ep.end_ms,
                episode_index,
            });
        }
    }
    out
}

/// 再開 1 件を競合リスクの 3 分類へ落とす。
///
/// - **主イベント** `Receipt`: `resumed_at_ms` 以降で最初の external paste。
/// - **競合イベント** `NextAwayCandidate`: 受領より先に、次の離席候補ギャップが始まった。
/// - **打ち切り** `Censored`: どちらも起きないまま `observation_end_ms` に達した。
///
/// 同時刻のタイブレークは **受領を優先**する。★ これは保守側である
/// (`receipt_free_resumption` を高く出さない向き)。
/// ⚠️ 推定器を一切参照しない。
///
/// ⚠️★★★ **受領を「紐づけ先エピソードの内側」に絞らないのは意図的である**
/// (`code-reviewer`(Codex) Medium・2026-09-07)。★ 推定するのは
/// 「**再開してから受領するまで**」であって「エピソード内の受領」ではない (ADR-024 D4)。
/// ★ 遠い未来の受領は地平 (5/10 分) の外に落ちるので CIF には寄与せず、
/// その事象を **地平まで at risk として残す**という正しい寄与だけを持つ。
pub fn classify_resumption(
    ev: &ResumptionEvent,
    pastes: &[PasteEvent],
    keys: &[KeyPress],
    observation_end_ms: u64,
    cfg: &OutcomeConfig,
) -> ResumptionOutcome {
    debug_assert!(
        keys.windows(2).all(|w| w[0].timestamp <= w[1].timestamp),
        "classify_resumption: keys must be sorted by timestamp"
    );
    let t0 = ev.resumed_at_ms;

    let receipt_at = pastes
        .iter()
        .filter(|p| p.timestamp >= t0)
        .map(|p| p.timestamp)
        .min();

    // 次の離席候補ギャップの開始 = t0 以降の打鍵で、次の打鍵との間隙が away_gap_ms 以上のもの。
    // `keys` は昇順前提なので二分探索で始点を求める (事象ごとに先頭から舐めない)。
    let from = keys.partition_point(|k| k.timestamp < t0);
    let next_away_candidate_at = keys[from..]
        .windows(2)
        .find(|w| w[1].timestamp.saturating_sub(w[0].timestamp) >= cfg.away_gap_ms)
        .map(|w| w[0].timestamp);

    let (at, kind) = match (receipt_at, next_away_candidate_at) {
        (Some(r), Some(l)) if r <= l => (r, ResumptionOutcomeKind::Receipt),
        (Some(_), Some(l)) => (l, ResumptionOutcomeKind::NextAwayCandidate),
        (Some(r), None) => (r, ResumptionOutcomeKind::Receipt),
        (None, Some(l)) => (l, ResumptionOutcomeKind::NextAwayCandidate),
        (None, None) => (observation_end_ms.max(t0), ResumptionOutcomeKind::Censored),
    };

    ResumptionOutcome {
        elapsed_ms: at.saturating_sub(t0),
        kind,
    }
}

/// Aalen–Johansen 推定量による受領の累積発生関数 (CIF)。
///
/// ```text
/// S(t_0) = 1
/// S(t_j) = S(t_{j-1}) * (1 - (d1_j + d2_j) / n_j)
/// CIF_receipt(t) = Σ_{t_j <= t}  S(t_{j-1}) * d1_j / n_j
/// ```
/// `n_j` = `t_j` 時点のリスク集合 (`elapsed_ms >= t_j`。打ち切りも同時刻なら at risk とする
/// 標準的な扱い) / `d1_j` = 受領 / `d2_j` = 次の離席候補。
///
/// ⚠️★★★ **地平まで追跡が届いていないときは `None` を返す** (`code-reviewer` H1)。
/// ★ 返さないと、**30 秒で観測が終わっただけ**の集団に対して `CIF = 0.0` →
/// `receipt_free = 1.0` = 「5 分間 1 件も受領しなかった」と読める値が出てしまう。
/// ★ 判定は「地平までに事象が 1 件も起きず、かつ地平時点のリスク集合が空」——
/// ★ このとき推定値は**外挿でしかない**。
///
/// 事象 0 のとき `None` (NaN を出さない)。⚠️ 依存 crate を足さない純粋関数。
fn cif_receipt(outcomes: &[ResumptionOutcome], horizon_ms: u64) -> Option<f64> {
    if outcomes.is_empty() {
        return None;
    }
    let mut times: Vec<u64> = outcomes
        .iter()
        .filter(|o| o.kind != ResumptionOutcomeKind::Censored)
        .map(|o| o.elapsed_ms)
        .collect();
    times.sort_unstable();
    times.dedup();

    // H1: 追跡が地平に届いておらず、地平までに事象も無い → 推定できない。
    if !times.iter().any(|&t| t <= horizon_ms) && n_at_risk(outcomes, horizon_ms) == 0 {
        return None;
    }

    let mut surv = 1.0_f64; // S(t_{j-1})
    let mut cif = 0.0_f64;
    for &t in &times {
        if t > horizon_ms {
            break;
        }
        // `t` は非打ち切り outcome の経過時間なので、その outcome 自身が必ず at risk。
        // よって `n_j >= 1` であり 0 除算は起きない。
        let n_j = outcomes.iter().filter(|o| o.elapsed_ms >= t).count() as f64;
        let d1 = outcomes
            .iter()
            .filter(|o| o.elapsed_ms == t && o.kind == ResumptionOutcomeKind::Receipt)
            .count() as f64;
        let d2 = outcomes
            .iter()
            .filter(|o| o.elapsed_ms == t && o.kind == ResumptionOutcomeKind::NextAwayCandidate)
            .count() as f64;
        cif += surv * (d1 / n_j);
        surv *= 1.0 - (d1 + d2) / n_j;
    }
    Some(cif)
}

/// `receipt_free_resumption` を算出する純粋関数。
///
/// `observation_end_ms` は打ち切り時刻 (= セッションの観測終端)。
/// ⚠️★★★ **`feats` を受け取らない。**★ これが「推定器を通らない」ことの構造的な保証である
/// (ADR-024 AC-2)。
pub fn compute_receipt_free_resumption(
    episodes: &[StuckEpisode],
    pastes: &[PasteEvent],
    keys: &[KeyPress],
    observation_end_ms: u64,
    cfg: &OutcomeConfig,
) -> ReceiptFreeResumption {
    let events = segment_resumptions(episodes, keys, cfg);
    let outcomes: Vec<ResumptionOutcome> = events
        .iter()
        .map(|ev| classify_resumption(ev, pastes, keys, observation_end_ms, cfg))
        .collect();

    // ⚠️ 終端の時刻ではなく添字で数える (終端が同値のエピソードでも取りこぼさない)。
    let mut source_episodes: Vec<usize> = events.iter().map(|e| e.episode_index).collect();
    source_episodes.sort_unstable();
    source_episodes.dedup();

    let count_of =
        |k: ResumptionOutcomeKind| outcomes.iter().filter(|o| o.kind == k).count() as u32;

    // H2: 追跡時間ゼロの事象は CIF から外す (件数は残す)。
    let tracked: Vec<ResumptionOutcome> = outcomes
        .iter()
        .copied()
        .filter(|o| !is_degenerate(o))
        .collect();

    ReceiptFreeResumption {
        event_count: outcomes.len() as u32,
        source_episode_count: source_episodes.len() as u32,
        receipt_count: count_of(ResumptionOutcomeKind::Receipt),
        next_away_candidate_count: count_of(ResumptionOutcomeKind::NextAwayCandidate),
        censored_count: count_of(ResumptionOutcomeKind::Censored),
        degenerate_count: outcomes.iter().filter(|o| is_degenerate(o)).count() as u32,
        n_at_risk_at_5min: n_at_risk(&tracked, RECEIPT_HORIZON_5MIN_MS),
        n_at_risk_at_10min: n_at_risk(&tracked, RECEIPT_HORIZON_10MIN_MS),
        receipt_free_at_5min: cif_receipt(&tracked, RECEIPT_HORIZON_5MIN_MS).map(|c| 1.0 - c),
        receipt_free_at_10min: cif_receipt(&tracked, RECEIPT_HORIZON_10MIN_MS).map(|c| 1.0 - c),
        outcomes,
    }
}

/// v0.4 アウトカム指標を算出する純粋関数。
/// `feats` は回復判定 (p_flow>0.5) と session span に使う。`episodes` は
/// `segment_stuck_episodes` 等で得た Stuck 候補区間。すべて timestamp 昇順を仮定。
pub fn compute_outcome_metrics(
    episodes: &[StuckEpisode],
    pastes: &[PasteEvent],
    keys: &[KeyPress],
    feats: &[FeatSnapshot],
    cfg: &OutcomeConfig,
) -> OutcomeMetrics {
    // (b) セッション全体の manual_insert_chars / paste 総和。
    // 破損/巨大値での u64 wrap を防ぐため saturating で畳む (cross-review CLAUDE-1)。
    let manual_insert_chars = manual_insert_chars_in(keys, u64::MIN, u64::MAX);
    let external_paste_chars_total: u64 = pastes
        .iter()
        .fold(0u64, |acc, p| acc.saturating_add(p.chars));
    let denom = manual_insert_chars.saturating_add(external_paste_chars_total);
    let manual_contribution_ratio = if denom > 0 {
        Some(manual_insert_chars as f64 / denom as f64)
    } else {
        None
    };

    // (a) paste 集計
    let external_paste_count = pastes.len() as u32;
    let large_paste_count = pastes
        .iter()
        .filter(|p| p.chars >= cfg.large_threshold)
        .count() as u32;
    let large_paste_rate = ratio(large_paste_count, external_paste_count);

    // エピソード単位の集計
    let episode_count = episodes.len() as u32;
    let mut paste_after_stuck = 0u32;
    let mut pflow_exit_no_receipt = 0u32;
    let mut pflow_exit_count = 0u32;
    let mut pflow_exit_manual_gain = 0u32;

    for ep in episodes {
        // after_stuck 窓内の paste
        if has_paste_in(
            pastes,
            ep.end_ms,
            ep.end_ms.saturating_add(cfg.after_window_ms),
        ) {
            paste_after_stuck += 1;
        }

        // 回復 (p_flow>0.5) を end_ms 以降 recovery_window 内に探す (既存プリミティブ再利用)
        if let Some(rec_ts) = compute_pflow_exit_at(
            ep.end_ms,
            feats,
            cfg.pflow_exit_window_ms,
            DEFAULT_P_FLOW_THRESHOLD,
        ) {
            pflow_exit_count += 1;
            // 回復までの間 [end_ms, rec_ts] に paste が無いか
            let no_paste = !has_paste_in(pastes, ep.end_ms, rec_ts);
            if no_paste {
                pflow_exit_no_receipt += 1;
                // net_diff 正進行 (manual 純増 >= net_diff_min) かつ paste 無し
                if manual_insert_chars_in(keys, ep.end_ms, rec_ts) >= cfg.net_diff_min {
                    pflow_exit_manual_gain += 1;
                }
            }
        }
    }

    let paste_after_stuck_rate = ratio(paste_after_stuck, episode_count);
    let pflow_exit_without_receipt = ratio(pflow_exit_no_receipt, episode_count);
    let pflow_exit_manual_gain_rate = ratio(pflow_exit_manual_gain, pflow_exit_count);

    // ★★★ 主要アウトカム (ADR-024): 推定器を通らない receipt_free_resumption。
    // 打ち切り時刻 = 観測されたイベントの最終時刻。feats / keys / pastes の末尾の最大を採る
    // (⚠️ feats は「いつまで観測していたか」にだけ使い、p_flow の値は読まない)。
    // ⚠️ `last()` は昇順を前提にするので、ここでは列全体の最大を採る (整列前提を持ち込まない)。
    let observation_end_ms = feats
        .iter()
        .map(|f| f.timestamp)
        .chain(keys.iter().map(|k| k.timestamp))
        .chain(pastes.iter().map(|p| p.timestamp))
        .max()
        .unwrap_or(0);
    let receipt_free_resumption =
        compute_receipt_free_resumption(episodes, pastes, keys, observation_end_ms, cfg);

    // 補助: paste_count_per_hour (feat 列の span をセッション時間とみなす)
    let paste_count_per_hour = match (feats.first(), feats.last()) {
        (Some(f0), Some(f1)) if f1.timestamp > f0.timestamp => {
            let hours = (f1.timestamp - f0.timestamp) as f64 / 3_600_000.0;
            if hours > 0.0 {
                Some(external_paste_count as f64 / hours)
            } else {
                None
            }
        }
        _ => None,
    };

    OutcomeMetrics {
        receipt_free_resumption,
        manual_insert_chars,
        external_paste_chars_total,
        manual_contribution_ratio,
        pflow_exit_manual_gain_rate,
        external_paste_count,
        // 収集時にしか分からないため compute では 0。`--metrics` 経路 (replay.rs) が
        // collect_paste_events の戻り値で上書きする。
        excluded_paste_count: 0,
        large_paste_rate,
        paste_after_stuck_rate,
        pflow_exit_without_receipt,
        paste_count_per_hour,
        stuck_episode_count: episode_count,
        pflow_exit_episode_count: pflow_exit_count,
    }
}

/// `num / den` を比率で返す。den == 0 → None (NaN を出さない)。
fn ratio(num: u32, den: u32) -> Option<f64> {
    if den > 0 {
        Some(num as f64 / den as f64)
    } else {
        None
    }
}

/// Option<f64> の比率を pretty 出力用に整形 (None → "n/a")。
fn fmt_opt_ratio(v: Option<f64>) -> String {
    v.map(|x| format!("{:.3}", x))
        .unwrap_or_else(|| "n/a".to_string())
}

/// Stuck ラベル時刻以降、`p_flow > threshold` に達する最初の FeatSnapshot 時刻を返す。
/// `window_ms` 以内に到達しなければ `None`。
///
/// ⚠️★★★ **旧名 `compute_recovered_at`** (ADR-024 D2 で改名)。★ 返すのは
/// 「**3 クラス事後分布が flow 側へ移った時刻**」であって「解決した時刻」ではない。
/// ⚠️ `p_flow > 0.5` は `p_stuck < 0.5` を必然的に含む (事後分布は正規化されている)
/// ため、これは**トリガーと独立した測定ではない** (ADR-023 D7)。
pub fn compute_pflow_exit_at(
    stuck_ts: u64,
    feats: &[FeatSnapshot],
    window_ms: u64,
    p_flow_threshold: f64,
) -> Option<u64> {
    let deadline = stuck_ts.saturating_add(window_ms);
    feats
        .iter()
        .filter(|f| f.timestamp >= stuck_ts && f.timestamp <= deadline)
        .find(|f| f.p_flow > p_flow_threshold)
        .map(|f| f.timestamp)
}

/// 混同行列 / per-class F1 / 中央値を算出する主関数。
///
/// `labels` と `feats` はともに timestamp 昇順を仮定（未ソートでも概ね動くが、
/// 連続 Stuck の扱いなどは順序依存のため呼び出し側で整えること）。
pub fn compute_session_metrics(
    labels: &[UserLabelEvent],
    feats: &[FeatSnapshot],
) -> SessionMetrics {
    let mut warnings: Vec<String> = Vec::new();
    let mut confusion = [[0u32; 3]; 3];
    let mut leads: Vec<i64> = Vec::new();
    let mut recoveries: Vec<u64> = Vec::new();

    if labels.is_empty() {
        warnings.push("no user_label entries found".to_string());
    }
    if feats.is_empty() && !labels.is_empty() {
        warnings.push("no feat snapshots found".to_string());
    }

    for lbl in labels {
        match nearest_feat(lbl.timestamp, feats, LABEL_MATCH_WINDOW_MS) {
            Some(f) => {
                let predicted = argmax_class(f);
                confusion[lbl.label.index()][predicted.index()] += 1;
            }
            None => {
                warnings.push(format!(
                    "label at t={} has no FeatSnapshot within ±{}ms",
                    lbl.timestamp, LABEL_MATCH_WINDOW_MS
                ));
            }
        }

        if let Some(lead_ms) = detection_lead_ms(lbl, feats, LEAD_LAG_WINDOW_MS) {
            leads.push(lead_ms);
        }

        if lbl.label == LabelClass::Stuck {
            if let Some(rec_ts) = compute_pflow_exit_at(
                lbl.timestamp,
                feats,
                DEFAULT_PFLOW_EXIT_WINDOW_MS,
                DEFAULT_P_FLOW_THRESHOLD,
            ) {
                recoveries.push(rec_ts.saturating_sub(lbl.timestamp));
            }
        }
    }

    let per_class = [
        per_class_metrics(LabelClass::Flow, &confusion),
        per_class_metrics(LabelClass::Incubation, &confusion),
        per_class_metrics(LabelClass::Stuck, &confusion),
    ];

    // scikit-learn 互換の macro F1: 固定分母 3 で平均。support=0 のクラスは
    // per_class_metrics() が precision=recall=f1=0.0 を返すので、そのまま加算。
    // こうすることでセッション間比較時に分母が変動しない（D3 の通り）。
    let macro_f1 = per_class.iter().map(|c| c.f1).sum::<f64>() / per_class.len() as f64;

    let micro_f1 = compute_micro_f1(&confusion);

    let total_labels = labels.len() as u32;

    SessionMetrics {
        total_labels,
        confusion: ConfusionMatrix { matrix: confusion },
        per_class,
        macro_f1,
        micro_f1,
        detection_lead_ms_median: median_i64(&mut leads),
        stuck_recovery_ms_median: median_u64(&mut recoveries),
        // outcome は key/paste 行を要するため compute_session_metrics では計算しない。
        // `--metrics` 経路 (replay.rs) が compute_outcome_metrics の結果を後から付与する。
        outcome: None,
        warnings,
    }
}

/// pretty-print 出力を生成（`--metrics` サブコマンドで使う）。
pub fn format_pretty(m: &SessionMetrics) -> String {
    let mut out = String::new();
    let (n_flow, n_inc, n_stuck) = (
        m.per_class[0].support,
        m.per_class[1].support,
        m.per_class[2].support,
    );
    out.push_str("SessionMetrics\n");
    out.push_str(&format!(
        "  Total labels: {} (Flow:{}, Inc:{}, Stuck:{})\n",
        m.total_labels, n_flow, n_inc, n_stuck
    ));
    out.push_str(&format!("  Macro F1: {:.3}\n", m.macro_f1));
    out.push_str(&format!("  Micro F1: {:.3}\n", m.micro_f1));
    out.push_str("  Per-class:\n");
    for c in &m.per_class {
        out.push_str(&format!(
            "    {:10} P={:.3}  R={:.3}  F1={:.3}  support={}\n",
            c.class.as_str(),
            c.precision,
            c.recall,
            c.f1,
            c.support,
        ));
    }
    out.push_str("  Confusion (rows = truth, cols = pred):\n");
    out.push_str("                Flow   Inc    Stuck\n");
    for (i, row) in m.confusion.matrix.iter().enumerate() {
        out.push_str(&format!(
            "    {:10}  {:>4}  {:>4}   {:>4}\n",
            CLASSES[i].as_str(),
            row[0],
            row[1],
            row[2],
        ));
    }
    match m.detection_lead_ms_median {
        Some(v) => out.push_str(&format!(
            "  Detection lead median: {:+} ms  (positive = HMM leads user)\n",
            v
        )),
        None => out.push_str("  Detection lead median: n/a\n"),
    }
    match m.stuck_recovery_ms_median {
        Some(v) => out.push_str(&format!("  Stuck recovery median: {} ms\n", v)),
        None => out.push_str("  Stuck recovery median: n/a\n"),
    }
    if let Some(o) = &m.outcome {
        out.push_str("  Outcome metrics (v0.4):\n");
        out.push_str(&format!(
            "    Stuck episodes: {} (p_flow exit: {})\n",
            o.stuck_episode_count, o.pflow_exit_episode_count
        ));
        // ★★★ 主要アウトカム (ADR-024)。推定器を通らないので先頭に置く。
        let r = &o.receipt_free_resumption;
        out.push_str(&format!(
            "    [PRIMARY] receipt_free_resumption  5min: {}  10min: {}\n",
            fmt_opt_ratio(r.receipt_free_at_5min),
            fmt_opt_ratio(r.receipt_free_at_10min)
        ));
        out.push_str(&format!(
            "              events: {} (from {} episodes)  receipt: {}  next_away_cand: {}  censored: {}\n",
            r.event_count,
            r.source_episode_count,
            r.receipt_count,
            r.next_away_candidate_count,
            r.censored_count
        ));
        out.push_str(&format!(
            "              degenerate: {}  n_at_risk 5min: {}  10min: {}\n",
            r.degenerate_count, r.n_at_risk_at_5min, r.n_at_risk_at_10min
        ));
        out.push_str(&format!(
            "    manual_insert_chars: {}  external_paste_chars: {}  external_paste_count: {} (excluded: {})\n",
            o.manual_insert_chars, o.external_paste_chars_total, o.external_paste_count, o.excluded_paste_count
        ));
        out.push_str(&format!(
            "    manual_contribution_ratio: {}\n",
            fmt_opt_ratio(o.manual_contribution_ratio)
        ));
        // ⚠️ 以下 2 本は p_flow 依存 (ADR-023 D7 / ADR-024)。主要アウトカムにしない。
        out.push_str(&format!(
            "    [pflow-dependent] pflow_exit_manual_gain_rate: {}\n",
            fmt_opt_ratio(o.pflow_exit_manual_gain_rate)
        ));
        out.push_str(&format!(
            "    [pflow-dependent] pflow_exit_without_receipt: {}  paste_after_stuck_rate: {}\n",
            fmt_opt_ratio(o.pflow_exit_without_receipt),
            fmt_opt_ratio(o.paste_after_stuck_rate)
        ));
        out.push_str(&format!(
            "    large_paste_rate: {}  paste_count_per_hour: {}\n",
            fmt_opt_ratio(o.large_paste_rate),
            o.paste_count_per_hour
                .map(|v| format!("{:.2}", v))
                .unwrap_or_else(|| "n/a".to_string())
        ));
    }
    out.push_str(&format!("  Warnings: {}\n", m.warnings.len()));
    for w in &m.warnings {
        out.push_str(&format!("    - {}\n", w));
    }
    out
}

// --- internal helpers -----------------------------------------------------

fn nearest_feat(ts: u64, feats: &[FeatSnapshot], window_ms: u64) -> Option<&FeatSnapshot> {
    feats
        .iter()
        .filter(|f| abs_diff(f.timestamp, ts) <= window_ms)
        .min_by_key(|f| abs_diff(f.timestamp, ts))
}

/// ★ `u64::abs_diff` の薄い別名 (⚠️ 呼び出し側の可読性のために残している)。
fn abs_diff(a: u64, b: u64) -> u64 {
    a.abs_diff(b)
}

/// HMM argmax 付与。タイブレークは Flow > Incubation > Stuck の固定順（稀だが発生しうる）。
fn argmax_class(f: &FeatSnapshot) -> LabelClass {
    if f.p_flow >= f.p_inc && f.p_flow >= f.p_stuck {
        LabelClass::Flow
    } else if f.p_inc >= f.p_stuck {
        LabelClass::Incubation
    } else {
        LabelClass::Stuck
    }
}

/// ラベル周辺 ±window_ms の FeatSnapshot を走査し、
/// ラベルと同じクラスが argmax になった中で **ラベル時刻に最も近い** feat を返す。
/// 戻り値 = label_ts - feat_ts (正 = HMM がラベルより先行 / 負 = 遅延)。
///
/// 初版は `find()` で先頭ヒット (窓内の最小 timestamp) を拾っていたが、
/// それはラベルから遠い偶発的な同クラス出現を拾う問題があった。
/// 実データ (gse_20260415_080214) で lead 中央値が +50s と窓端付近に貼り付き、
/// M1 指摘として顕在化。`min_by_key(abs_diff)` に変更して「ラベル時刻に最も近い
/// 同クラス feat」に再定義する (2026-04-15, decisions.md D9)。
fn detection_lead_ms(lbl: &UserLabelEvent, feats: &[FeatSnapshot], window_ms: u64) -> Option<i64> {
    let lo = lbl.timestamp.saturating_sub(window_ms);
    let hi = lbl.timestamp.saturating_add(window_ms);
    let hit = feats
        .iter()
        .filter(|f| f.timestamp >= lo && f.timestamp <= hi)
        .filter(|f| argmax_class(f) == lbl.label)
        .min_by_key(|f| abs_diff(f.timestamp, lbl.timestamp))?;
    // 正 = HMM がラベルより先行 (feat が早い → label_ts - feat_ts が正)。
    Some(lbl.timestamp as i64 - hit.timestamp as i64)
}

fn per_class_metrics(class: LabelClass, cm: &[[u32; 3]; 3]) -> ClassMetrics {
    let i = class.index();
    let tp = cm[i][i] as f64;
    let fp: f64 = (0..3).filter(|&j| j != i).map(|j| cm[j][i] as f64).sum();
    let fn_: f64 = (0..3).filter(|&j| j != i).map(|j| cm[i][j] as f64).sum();
    let support = (0..3).map(|j| cm[i][j]).sum::<u32>();
    let precision = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
    let recall = if tp + fn_ > 0.0 { tp / (tp + fn_) } else { 0.0 };
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    ClassMetrics {
        class,
        precision,
        recall,
        f1,
        support,
    }
}

/// 多クラスの micro F1 = micro precision = micro recall = accuracy。
///
/// **分母**: 混同行列の合計 = `nearest_feat` でマッチしたラベル数のみ。
/// ラベル近傍に FeatSnapshot が無く warnings に落ちたラベルは含まれない。
/// マッチ率が低いセッションでは `total_labels` より小さな分母になる点に注意
/// （`baseline_metrics.md` では warnings 件数を併記して判断する）。
fn compute_micro_f1(cm: &[[u32; 3]; 3]) -> f64 {
    let tp: u32 = (0..3).map(|i| cm[i][i]).sum();
    let total: u32 = cm.iter().flatten().sum();
    if total == 0 {
        0.0
    } else {
        tp as f64 / total as f64
    }
}

fn median_i64(values: &mut [i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        // i64 加算のオーバーフロー回避のため i128 経由で計算。
        // (両端が i64::MIN と i64::MAX の両立は LEAD_LAG_WINDOW_MS=60_000
        // で現実には起きないが、関数契約として安全側に倒す)
        let a = values[mid - 1] as i128;
        let b = values[mid] as i128;
        Some(((a + b) / 2) as i64)
    } else {
        Some(values[mid])
    }
}

fn median_u64(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        // sort 後なので a <= b、`a + (b - a) / 2` は u64 でも overflow しない。
        let a = values[mid - 1];
        let b = values[mid];
        Some(a + (b - a) / 2)
    } else {
        Some(values[mid])
    }
}

// ── Range-based labeling (Phase 1 GT 改善) ────────────────────────────

/// 範囲ラベル: セグメント [start_ms, end_ms] に対するラベル。
/// 区間内の全 FeatSnapshot と比較することで、1 ラベルから多数の
/// feat-label ペアが生成され、統計検出力が大幅に向上する。
#[derive(Debug, Clone, Copy)]
pub struct RangeLabel {
    pub start_ms: u64,
    pub end_ms: u64,
    pub label: LabelClass,
}

/// 範囲ラベルを用いたメトリクス算出。
///
/// 各 RangeLabel の [start, end] 区間内に含まれる全 feat に対して
/// argmax を取り、confusion matrix を構築する。
/// detection_lead / stuck_recovery は範囲ラベルでは算出しない
/// (区間全体がラベルなので「検出タイミング」の概念がない)。
pub fn compute_range_metrics(
    range_labels: &[RangeLabel],
    feats: &[FeatSnapshot],
) -> SessionMetrics {
    let mut warnings: Vec<String> = Vec::new();
    let mut confusion = [[0u32; 3]; 3];
    let mut total_segments = 0u32;

    if range_labels.is_empty() {
        warnings.push("no range labels found".to_string());
    }
    if feats.is_empty() && !range_labels.is_empty() {
        warnings.push("no feat snapshots found".to_string());
    }

    for rl in range_labels {
        let feats_in_range: Vec<&FeatSnapshot> = feats
            .iter()
            .filter(|f| f.timestamp >= rl.start_ms && f.timestamp <= rl.end_ms)
            .collect();

        if feats_in_range.is_empty() {
            warnings.push(format!(
                "segment [{}, {}] has no FeatSnapshot",
                rl.start_ms, rl.end_ms,
            ));
            continue;
        }

        for f in &feats_in_range {
            let predicted = argmax_class(f);
            confusion[rl.label.index()][predicted.index()] += 1;
        }
        total_segments += 1;
    }

    let per_class = [
        per_class_metrics(LabelClass::Flow, &confusion),
        per_class_metrics(LabelClass::Incubation, &confusion),
        per_class_metrics(LabelClass::Stuck, &confusion),
    ];

    let macro_f1 = per_class.iter().map(|c| c.f1).sum::<f64>() / per_class.len() as f64;
    let micro_f1 = compute_micro_f1(&confusion);

    SessionMetrics {
        total_labels: total_segments,
        confusion: ConfusionMatrix { matrix: confusion },
        per_class,
        macro_f1,
        micro_f1,
        detection_lead_ms_median: None,
        stuck_recovery_ms_median: None,
        outcome: None,
        warnings,
    }
}
