//! ドメイン推論ヘッド (domain inference heads)
//!
//! ADR-002 (domain-local inference) / ADR-006 (task_kind → β baseline 配線) の実装。
//!
//! # 設計の骨子
//!
//! β baseline は「1 関数内の 2 択 if」ではなく **各 head の所有物**である。
//! どの head を走らせるかは `task_kind` (宣言) が決め、`ime_open` (観測) は
//! **Writing head 内部の within-writing 信号**として使う。両者の役割は直交する。
//!
//! ```text
//!   task_kind ──► head 選択 ──► head が所有する β で潜在二軸を算出
//!   ime_open  ──────────────────► Writing head 内部の β 切替 / BS ペナルティ gate
//! ```
//!
//! # trait の範囲について (ADR-002 からの意図的な逸脱)
//!
//! ADR-002 逐語の `DomainInferenceHead` は
//! `infer(&mut self, &DomainFeatureSnapshot, &SessionContext) -> WorkStateEstimate` だが、
//! `WorkStateEstimate` (`uncertainty` / `evidence_quality` 必須) は Decision Engine 境界
//! (ADR-004) とセットで導入すべきものである。本モジュールは現行 engine の形に合わせ、
//! **潜在二軸の算出だけ**を trait 化する。`infer -> WorkStateEstimate` への移行は Epic 3。
//! 判断の根拠は `.steering/20260818-adr006-head-extraction-wiring/decisions.md` D-8。
//!
//! # 関連
//!
//! - `docs/adr/ADR-002-domain-local-inference.md`
//! - `docs/adr/ADR-006-task-kind-beta-wiring.md`
//! - `docs/architecture/domain-adapter-boundary.md` §Layer 3-4

pub mod coding;
pub mod writing;

pub use coding::CodingHeadV0;
pub use writing::WritingHead;

use std::sync::Arc;

use crate::analysis::features::Features;

/// どの domain head を走らせるかを決めるルーティング信号。
///
/// ⚠️ **`logger::TaskKind` をそのまま持ち込まない。** `analysis/` は `logger` に依存しない
/// （`docs/repository-structure.md` §4 依存方向）。逆向きの
/// `impl From<TaskKind> for TaskDomain` は **`logger.rs` に 1 本だけ**置いてある
/// （`logger` は既に `analysis::engine::HMM_VERSION` を参照しており、この向きの依存は既存）。
/// 本番 (`lib.rs`) もリプレイ (`bin/replay.rs`) も**その 1 本を通す** —— 2 箇所に書くと
/// 片方だけ更新されて本番とリプレイの head 選択がずれ、ゴールデンが守っている保証が嘘になる。
///
/// 「宣言 (`task_kind`) が head を選び、観測 (`ime_open`) は head 内部の信号」という
/// ADR-006 Alternative C の役割分離を型で表している。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskDomain {
    /// `argumentative` / `correspondence` → Writing head。
    #[default]
    Writing,
    /// `creative` → Writing head だが `head_id` で分離できるようにする（ADR-006 #3 からの
    /// 意図的な逸脱・`.steering/` D-6）。**数値挙動は `Writing` と同一。**
    WritingFallback,
    /// `vibe_coding` → Coding head v0。
    Coding,
}

impl TaskDomain {
    /// 全 variant。`schema_dto::HeadIdDto` との対応をテストで網羅するために使う。
    ///
    /// ⚠️ variant を足したらここにも足すこと（足し忘れは
    /// `all_task_domains_map_to_schema_head_ids` が検出する）。
    pub const ALL: [TaskDomain; 3] = [
        TaskDomain::Writing,
        TaskDomain::WritingFallback,
        TaskDomain::Coding,
    ];

    /// この domain で走る head の識別子。engine を作らずに求められる
    /// (`session_meta` を書く時点ではまだ engine を触りたくないため)。
    pub fn head_id(self) -> HeadId {
        match self {
            TaskDomain::Writing => HeadId::Writing,
            TaskDomain::WritingFallback => HeadId::WritingFallback,
            TaskDomain::Coding => HeadId::CodingV0,
        }
    }
}

/// どの head がセッションを採点したかの識別子。
///
/// ⚠️ **`&str` ではなく enum にしている理由**: この値は `logger.rs` の**手書き NDJSON ライタ**
/// (`format!` ベースでエスケープしない) に渡る。`&str` のままだと、将来ここに動的文字列が
/// 流れ込んだときに `"` や改行で行が壊れる・偽の NDJSON 行を注入できる余地が残る。
/// 固定カテゴリを型で閉じておけば、その経路自体が存在しなくなる
/// (security-checker 指摘・2026-08-18)。`schema_dto::HeadIdDto` と対で維持すること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadId {
    /// `argumentative` / `correspondence` を Writing head で採点した。
    Writing,
    /// **Creative / Other を Writing head で採点した。**
    ///
    /// ★ Other (ADR-037 D2) は「4 つの領域のどれとも決めづらい」と本人が宣言した区間で、
    /// 適合する head が無い点で Creative と同じ。⚠️ 両者は `session_meta` の `task_kind` で分ける。
    ///
    /// ⚠️ ADR-006 下位決定 #3 は「Creative には打鍵 β を持たせない」と決めているが、
    /// 代替 head がまだ存在しないため Writing head へ流している (#3 からの意図的な逸脱・
    /// `.steering/20260818-adr006-head-extraction-wiring/decisions.md` D-6)。
    /// この識別子を `session_meta` に刻んでおくことで「**適合していない head で採点された
    /// セッション**」を事後に分離でき、Epic 6 で Creative の feasibility を評価するときの
    /// 一次データになる。**数値挙動は `Writing` と完全に同一。**
    WritingFallback,
    /// `vibe_coding` を Coding head v0 で採点した。
    CodingV0,
}

impl HeadId {
    pub fn as_str(self) -> &'static str {
        match self {
            HeadId::Writing => "writing",
            HeadId::WritingFallback => "writing_fallback",
            HeadId::CodingV0 => "coding_v0",
        }
    }
}

/// ルーティング表。ADR-006 Alternative C の「task_kind が head を選ぶ」の実体。
///
/// ⚠️ ここに `_ => ` を書かないこと。variant を足したら**必ず head を決めさせる**ため、
/// 網羅性検査をコンパイラに効かせる。
pub fn head_for(domain: TaskDomain) -> Arc<dyn DomainInferenceHead> {
    match domain {
        TaskDomain::Writing => Arc::new(WritingHead::new()),
        TaskDomain::WritingFallback => Arc::new(WritingHead::fallback()),
        TaskDomain::Coding => Arc::new(CodingHeadV0::new()),
    }
}

/// ドメイン推論ヘッド。
///
/// 各 head は自分の β baseline を所有し、特徴量から潜在二軸
/// (X = Cognitive Friction, Y = Engagement) を算出する。
pub trait DomainInferenceHead: Send + Sync {
    /// head の識別子。どの head がそのセッションを採点したかを記録するために使う。
    fn head_id(&self) -> HeadId;

    /// 潜在二軸 (X = Friction, Y = Engagement) を算出する。
    ///
    /// `ime_open` は「今この瞬間、日本語 IME が日本語入力モード (あ/カ) か」を表す実行時フラグ。
    /// **これをどう使うか・使わないかは head が決める** (Writing head は β 切替に使い、
    /// Coding head は使わない)。
    fn latent_axes(&self, features: &Features, ime_open: bool) -> (f64, f64);

    /// backspace ペナルティビン (obs=25) を適用してよいか。
    ///
    /// ⚠️ **デフォルト実装を置かない。** 各 head に明示実装を強制する。
    /// 既定 (`!ime_open`) を置くと、新しい head を足した人が「BS ペナルティを IME で
    /// gate するか」を**判断せずに黙って継承**してしまう。β を head の所有物にしながら
    /// gate だけ暗黙の既定に落とすのは非対称であり、ADR-006 の「β は head の所有物」
    /// という決定と整合しない (code-reviewer 指摘・2026-08-18)。
    fn applies_backspace_penalty(&self, ime_open: bool) -> bool;
}
