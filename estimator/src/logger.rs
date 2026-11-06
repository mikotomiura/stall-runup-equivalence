//! `TaskKind` and its routing to a domain head, excerpted from GSE `src-tauri/src/logger.rs`
//! (commit 24767c7e). The rest of the GSE logger (session recording) is not part of this crate.
//! The lines between the excerpt markers are byte-identical to the original.

use crate::analysis::heads::TaskDomain;

// @excerpt-begin task_kind
/// v0.3 較正スモークの課題種別 (task_kind)。session_meta で宣言する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Argumentative,
    Correspondence,
    Creative,
    VibeCoding,
    /// ★ O1 の「その他・細切れの作業」(ADR-037)。4 つの領域のどれとも決めづらいと本人が宣言した。
    /// ⚠️ 未宣言 (`session_meta` の不在・ADR-016 判断 4b-3) とは別の状態である。
    Other,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Argumentative => "argumentative",
            Self::Correspondence => "correspondence",
            Self::Creative => "creative",
            Self::VibeCoding => "vibe_coding",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "argumentative" => Some(Self::Argumentative),
            "correspondence" => Some(Self::Correspondence),
            "creative" => Some(Self::Creative),
            "vibe_coding" => Some(Self::VibeCoding),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// ADR-006 Alternative C のルーティング: **宣言された `task_kind` が domain head を選ぶ**。
///
/// ⚠️ **変換をここに 1 本だけ置く。** `lib.rs` (本番) も `bin/replay.rs` (リプレイ) も
/// この経路を通す。2 箇所に書くと片方だけ更新されて本番とリプレイの挙動がずれる
/// (= ゴールデンが守れなくなる)。
///
/// ⚠️ 依存方向: `logger` → `analysis` は既存 (`HMM_VERSION` を参照している) なので
/// この向きで置く。逆に `analysis` 側へ `TaskKind` を持ち込むと
/// `analysis` が `logger` に依存してしまい `docs/repository-structure.md` §4 に反する。
///
/// ⚠️ `_ =>` を書かないこと。`TaskKind` に variant を足したら**必ず head を決めさせる**。
impl From<TaskKind> for TaskDomain {
    fn from(kind: TaskKind) -> Self {
        match kind {
            // 論述・文通は Writing head (Phase1 で走っていた挙動そのもの)
            TaskKind::Argumentative | TaskKind::Correspondence => TaskDomain::Writing,
            // Creative は打鍵 β が原理的に不適 (ADR-006 #3) だが代替 head がまだ無い。
            // 数値挙動は Writing と同一のまま、head_id だけ分けて事後に分離可能にする
            // (.steering/20260818-adr006-head-extraction-wiring/decisions.md D-6)
            // Other (ADR-037 D2) も同じ: 適合する head が無いと本人が言った区間。推論は Writing と同一。
            TaskKind::Creative | TaskKind::Other => TaskDomain::WritingFallback,
            // ここが 2b で実際に挙動が変わる唯一の分岐
            TaskKind::VibeCoding => TaskDomain::Coding,
        }
    }
}
// @excerpt-end task_kind
