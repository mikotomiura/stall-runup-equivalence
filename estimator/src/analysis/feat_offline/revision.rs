//! `T1c` = **削除と挿入の遷移の形**（issue 004・ADR-030 D10 / D11）。
//!
//! ⚠️★★★ **「修正（revision）の位相」と呼ばない。** ★ 誤打訂正と内容修正は、文字があっても
//! 自然課題ではほぼ分けられず（[V156]）、GSE の削除系の量は大半が表層の訂正を数えている
//! （[V155]: 表層 92%）。★ ここで測るのは **削除と挿入がどう並んだか**という観測だけである
//! （`docs/glossary.md` ／ `.steering/20260918-theory-critical-review/definitions-restop-t1d-t1c.md` §6.2b）。
//! ⚠️ 識別子 `revision` は出力の互換のために残している（★ 呼び名は上記のとおり）。
//!
//! **causal**（`super` のモジュール doc の区分に従う）: 各 tick に載る値は、
//! **その tick までに「閉じ」が確定した単位**だけから決まる（U3 の裁定）。
//! ★ セッション全体を見てから遡って付け直す値は 1 つも無い。
//!
//! # 裁定（★ `.steering/20260920-issues-004-006/decisions.md` 判断 1）
//!
//! | # | 裁定 |
//! |---|---|
//! | U1 | 削除の走りは、**削除系以外の press がすべて切る**（★ `navigation` も切る ——
//!       位置の代理になりうる情報を潰さないため） |
//! | U2 | `category_bigram_repeat_rate` = `1 − 相異なる bigram の種類数 ÷ bigram の総数`
//!       （bigram が 0 個の burst は `None`） |
//! | U3 | 走り・burst を単位とする値は **閉じが確定した時点の tick** に載せる
//!       （走り = 次の削除系以外の press または burst の閉じ ／
//!        burst = 最後の press ＋ `PAUSE_THRESHOLD_MS`） |
//! | U4 | 走りの最小の長さは **1**（★ 2 以上にすると新しい定数 `2` を作ることになる） |
//!
//! ★ U1 ＋ U3 から従う細目:
//! 1. **走りは burst をまたがない**（U3 が閉じる契機に「burst の閉じ」を挙げているため）。
//!    → ★ これで `delete_runs_after_first_in_burst` の「属する burst」が一意に決まる。
//! 2. **件数は数える単位が閉じた tick**、**burst 単位の値は burst が閉じた tick**。
//! 3. ⚠️ **閉じがグリッドの外に落ちた単位は黙って捨てる**
//!    （★ `stall_onset::column_from_onsets` がグリッド外の `t0` を捨てるのと同じ規約。
//!    ⚠️ 実測: `instrument_rich.ndjson` は**末尾 burst の閉じがグリッド外**なので、
//!    その burst 単位の値は出ない —— ★ 閉じを観測していないのだから出さないのが正しい）。
//! 4. **件数は `0`、閉じる単位が無い tick の最大値・率は `None`**（⚠️ 0 で埋めない）。
//!
//! # 本文を参照しない（issue 004 AC3）
//!
//! 純関数の入力は [`CategorizedPress`]（`KeyCategory` と時刻だけ）である。★ `vk` は
//! 境界のアダプタ [`revision_column`] でのみ触り、`KeyCategory::from_vk` に通して落とす。
//! ⚠️ 挿入系 / 削除系の写像は **ADR-030 D3 の SSOT（`super::key_net_delta`）と同値**でなければ
//! ならない —— ★ `tests::category_class_matches_key_net_delta` が**全 VK** で固定する
//! （⚠️ 新しい分類を作っているのではなく、SSOT を `KeyCategory` 側から引き直しているだけである）。
//!
//! # 出さないもの（issue 004 Out）
//!
//! ⚠️★★★ `undo` / `redo` の往復と、`key` 行の修飾キー押下ビット（ADR-032 D9）は**作らない**。
//! ★ ADR-032 D1 / D9 は Proposed であり、§Migration M4（同意 §3 の改訂）が研究者承認待ちである。

use crate::analysis::anonymize::KeyCategory;
use crate::analysis::features::PAUSE_THRESHOLD_MS;
use crate::analysis::metrics::KeyPress;

/// press 1 件を **種別と時刻だけ**に落としたもの（issue 004 AC3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CategorizedPress {
    pub t_ms: u64,
    pub category: KeyCategory,
}

/// ADR-030 D3 の写像（挿入系 / 削除系 / どちらでもない）を `KeyCategory` 側から引いたもの。
/// ⚠️ `super::key_net_delta`（SSOT）と全 VK で一致することを単体テストが固定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressClass {
    Insert,
    Delete,
    Neither,
}

/// `KeyCategory` → [`PressClass`]。⚠️ **新しい分類ではない** —— ADR-030 D3 の
/// `metrics::is_text_producing` / `is_backspace` を `KeyCategory` の語彙へ引き写したものである。
fn class_of(category: KeyCategory) -> PressClass {
    match category {
        KeyCategory::Backspace | KeyCategory::Delete => PressClass::Delete,
        // `is_text_producing` が true な VK は、この 7 category に 1 つ残らず入る
        // (`anonymize.rs` の `Punctuation` / `SentencePunctuation` / `Tab` の doc 参照)。
        KeyCategory::Alphabetic
        | KeyCategory::Numeric
        | KeyCategory::Punctuation
        | KeyCategory::SentencePunctuation
        | KeyCategory::Tab
        | KeyCategory::Space
        | KeyCategory::Enter => PressClass::Insert,
        KeyCategory::Navigation
        | KeyCategory::Modifier
        | KeyCategory::Function
        | KeyCategory::Escape
        | KeyCategory::Other => PressClass::Neither,
    }
}

/// `T1c` の 1 tick ぶんの値。★ 呼び名は「削除と挿入の遷移の形」（モジュール doc 冒頭）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Revision {
    /// その tick で閉じた削除の走りの**最大長**。閉じた走りが無ければ `null`。
    pub delete_run_max: Option<u64>,
    /// その tick で閉じた削除の走りの**本数**。
    pub delete_runs: u64,
    /// 削除の走りの**直後の press が挿入系**だった回数（★ 旧名 `rewrite_count`）。
    pub del_to_ins_transitions: u64,
    /// burst の中で **2 本目以降**に現れた削除の走りの本数（★ 旧名 `redelete_count`）。
    pub delete_runs_after_first_in_burst: u64,
    /// その tick で閉じた burst の `{D, I}` 交替回数の最大（★ 旧名 `loop_depth_max`）。
    /// 閉じた burst が無ければ `null`。
    pub di_alternations_max: Option<u64>,
    /// その tick で閉じた burst の `KeyCategory` bigram の重複率（★ 旧名 `local_recurrence`）。
    /// bigram が 0 個（press 1 件の burst）なら `null`、閉じた burst が無ければ `null`。
    pub category_bigram_repeat_rate: Option<f64>,
}

impl Revision {
    /// 単位が 1 つも閉じなかった tick の行（★ 件数は 0・最大値と率は `null`）。
    fn empty_row() -> Self {
        Self {
            delete_run_max: None,
            delete_runs: 0,
            del_to_ins_transitions: 0,
            delete_runs_after_first_in_burst: 0,
            di_alternations_max: None,
            category_bigram_repeat_rate: None,
        }
    }
}

/// 閉じが確定した削除の走り 1 本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeleteRun {
    /// 閉じが確定した時刻（U3: 次の削除系以外の press ／ 無ければ burst の閉じ）。
    closed_at_ms: u64,
    len: u64,
    /// その burst の中で 2 本目以降か。
    after_first_in_burst: bool,
    /// 走りを閉じた press が挿入系だったか（`del_to_ins_transitions` の材料）。
    closed_by_insert_at_ms: Option<u64>,
}

/// 閉じが確定した burst 1 つ。
#[derive(Debug, Clone, Copy, PartialEq)]
struct BurstSummary {
    /// 閉じが確定した時刻（U3: 最後の press ＋ `PAUSE_THRESHOLD_MS`）。
    closed_at_ms: u64,
    di_alternations: u64,
    bigram_repeat_rate: Option<f64>,
}

/// 走りと burst を取り出した結果（tick へ載せる前の中間表現）。
#[derive(Debug, Default)]
struct RevisionEvents {
    runs: Vec<DeleteRun>,
    bursts: Vec<BurstSummary>,
}

/// press 列（時刻昇順）を burst 単位の添字範囲へ切る。
///
/// ⚠️★★★ **独自の pause 定義を作らない**（issue 004 AC7 / Codex TASK-PRE 指摘 5）——
/// `super::extract_pause_resumptions`（= `features::PAUSE_THRESHOLD_MS` の SSOT）が返す
/// 「pause を終える press の時刻」の列を、**同じ昇順で突き合わせながら**歩いて切る。
///
/// ★ 値の一致で突き合わせて安全な理由: `super::press_intervals` は隣接対を昇順に 1 つずつ
/// 作り、pause の `resuming_ts` は**その対の 2 打目の時刻**である。連続する 2 対が同じ
/// `resuming_ts` を持つことは `t_i == t_{i+1}` のときだけで、そのとき間隔は 0ms なので
/// pause にならない。→ ★ 同じ順に歩けば取り違えは起きない（同時刻打鍵があっても）。
fn burst_ranges(sorted: &[CategorizedPress]) -> Vec<(usize, usize)> {
    if sorted.is_empty() {
        return Vec::new();
    }
    let press_ts: Vec<u64> = sorted.iter().map(|p| p.t_ms).collect();
    let resumptions = super::extract_pause_resumptions(&press_ts);

    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut next_resumption = 0usize;
    for (i, press) in sorted.iter().enumerate().skip(1) {
        if next_resumption < resumptions.len() && resumptions[next_resumption] == press.t_ms {
            next_resumption += 1;
            ranges.push((start, i));
            start = i;
        }
    }
    ranges.push((start, sorted.len()));
    ranges
}

/// burst 1 つぶんの `{D, I}` 交替回数（U1 の裁定と対称: どちらでもない press は列から外す）。
/// ★ 隣接する同じ記号を潰した列の長さ − 1。D/I の press が 1 つも無ければ `0`。
fn di_alternations(burst: &[CategorizedPress]) -> u64 {
    let mut alternations = 0u64;
    let mut prev: Option<PressClass> = None;
    for p in burst {
        let class = class_of(p.category);
        if class == PressClass::Neither {
            continue;
        }
        if let Some(previous) = prev {
            if previous != class {
                alternations += 1;
            }
        }
        prev = Some(class);
    }
    alternations
}

/// burst 1 つぶんの `KeyCategory` bigram の重複率（U2 の裁定: `1 − 種類数 ÷ 総数`）。
/// bigram が 0 個（press が 1 件以下）なら `None`。
///
/// ⚠️★★★ **出力は率という実数 1 つだが、`distinct`（相異なる bigram の *種類数*）は
/// 逆算できる** —— 分母 `total` は同じ出力の `activity.keys` を burst 区間で足せば得られる
/// （2026-09-20・`security-checker` L1）。★ ただし**どの型が出たか・順序は 1 ビットも出ない**ので、
/// 本文の部分復元は構造的に不可能である。⚠️ 「率だから安全」という前提を置かないこと。
fn category_bigram_repeat_rate(burst: &[CategorizedPress]) -> Option<f64> {
    if burst.len() < 2 {
        return None;
    }
    // ★ 種類数を数えるだけなので、文字列ではなく**判別子**で並べ替える
    //   （2026-09-20・`code-reviewer` M3: `as_str()` の文字列比較 ＋ clone は
    //    15 万 press が 1 burst になる入力で効く）。⚠️ 並び順に意味は無い（distinct だけを使う）。
    let mut bigrams: Vec<(u8, u8)> = burst
        .windows(2)
        .map(|w| (w[0].category as u8, w[1].category as u8))
        .collect();
    let total = bigrams.len() as f64;
    bigrams.sort_unstable();
    bigrams.dedup();
    // ★ `1.0 - distinct/total` ではなく差で書く（⚠️ 1/3 が 0.33333333333333337 にならない）。
    Some((total - bigrams.len() as f64) / total)
}

/// press 列（時刻昇順）から、閉じが確定した走りと burst を取り出す純関数。
///
/// ⚠️ 引数は `KeyCategory` と時刻だけである（issue 004 AC3）。
fn extract_revision_events(sorted: &[CategorizedPress]) -> RevisionEvents {
    let mut events = RevisionEvents::default();

    for (lo, hi) in burst_ranges(sorted) {
        let burst = &sorted[lo..hi];
        // U3: burst の閉じは「最後の press ＋ PAUSE_THRESHOLD_MS」。
        // ⚠️ 加算が溢れる入力ではこの burst の単位を捨てる (境界入力で panic しない)。
        let Some(burst_closed_at_ms) = burst
            .last()
            .and_then(|p| p.t_ms.checked_add(PAUSE_THRESHOLD_MS))
        else {
            continue;
        };

        let mut runs_in_burst = 0u64;
        let mut current_run_len = 0u64;
        for p in burst {
            match class_of(p.category) {
                PressClass::Delete => current_run_len += 1,
                // U1: 削除系以外の press はすべて走りを切る。
                other => {
                    if current_run_len > 0 {
                        events.runs.push(DeleteRun {
                            closed_at_ms: p.t_ms,
                            len: current_run_len,
                            after_first_in_burst: runs_in_burst > 0,
                            closed_by_insert_at_ms: (other == PressClass::Insert).then_some(p.t_ms),
                        });
                        runs_in_burst += 1;
                        current_run_len = 0;
                    }
                }
            }
        }
        // burst の末尾まで削除系が続いた走りは、burst の閉じで確定する (U3)。
        if current_run_len > 0 {
            events.runs.push(DeleteRun {
                closed_at_ms: burst_closed_at_ms,
                len: current_run_len,
                after_first_in_burst: runs_in_burst > 0,
                closed_by_insert_at_ms: None,
            });
        }

        events.bursts.push(BurstSummary {
            closed_at_ms: burst_closed_at_ms,
            di_alternations: di_alternations(burst),
            bigram_repeat_rate: category_bigram_repeat_rate(burst),
        });
    }

    events
}

/// 1 Hz tick 単位の `revision` 列を作る。`super::build_feature_rows` から呼ばれる。
///
/// ★ ここが `vk` を触る唯一の場所である（`KeyCategory::from_vk` に通して落とす）。
/// `tick_count` / `grid_origin_ms` / `tick_ms` は呼び出し側の tick グリッドと揃える。
/// ⚠️ グリッドの外に閉じが落ちた単位は黙って捨てる（モジュール doc の細目 3）。
///
/// ⚠️ `tick_ms` は 0 でないこと（★ 呼び出し側は `ENGINE_TICK_MS` を渡す）。
/// ★ 可視性は `restart_column` と揃えて `pub(super)` にしてある
/// （2026-09-20・`security-checker` L2: `pub` のままだとモジュール外から 0 を渡せた）。
pub(super) fn revision_column(
    keys: &[KeyPress],
    tick_count: usize,
    grid_origin_ms: u64,
    tick_ms: u64,
) -> Vec<Revision> {
    let mut column: Vec<Revision> = (0..tick_count).map(|_| Revision::empty_row()).collect();

    let mut sorted: Vec<CategorizedPress> = keys
        .iter()
        .map(|k| CategorizedPress {
            t_ms: k.timestamp,
            category: KeyCategory::from_vk(k.vk),
        })
        .collect();
    sorted.sort_by_key(|p| p.t_ms);

    let events = extract_revision_events(&sorted);
    // ⚠️ 0 除算は release でも panic する。★ 割る前に**理由の分かる形で**止める
    //   (2026-09-21・`/cross-review` 1 巡目・Codex 指摘 1)。
    assert!(tick_ms > 0, "revision_column: tick_ms は 0 にできない");
    let tick_of = |t: u64| -> Option<usize> {
        if t < grid_origin_ms {
            return None;
        }
        let idx = ((t - grid_origin_ms) / tick_ms) as usize;
        (idx < tick_count).then_some(idx)
    };

    for run in &events.runs {
        if let Some(idx) = tick_of(run.closed_at_ms) {
            let row = &mut column[idx];
            row.delete_runs += 1;
            row.delete_run_max = Some(row.delete_run_max.map_or(run.len, |m| m.max(run.len)));
            if run.after_first_in_burst {
                row.delete_runs_after_first_in_burst += 1;
            }
        }
        // ★ del→ins の遷移は「遷移を確定させた挿入系 press」の tick に載せる (U3 の細目 2)。
        if let Some(at_ms) = run.closed_by_insert_at_ms {
            if let Some(idx) = tick_of(at_ms) {
                column[idx].del_to_ins_transitions += 1;
            }
        }
    }

    for burst in &events.bursts {
        let Some(idx) = tick_of(burst.closed_at_ms) else {
            continue;
        };
        let row = &mut column[idx];
        row.di_alternations_max = Some(
            row.di_alternations_max
                .map_or(burst.di_alternations, |m| m.max(burst.di_alternations)),
        );
        // ★ burst の閉じは互いに PAUSE_THRESHOLD_MS 以上離れるので、1 tick に閉じる burst は
        //   高々 1 つである (判断 1 の細目 3)。`max` と同じ向き (大きい方を残す) にしておく。
        row.category_bigram_repeat_rate =
            match (row.category_bigram_repeat_rate, burst.bigram_repeat_rate) {
                (Some(existing), Some(new_value)) => Some(existing.max(new_value)),
                (existing, None) => existing,
                (None, new_value) => new_value,
            };
    }

    column
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト専用の閾値超えギャップ (★ `PAUSE_THRESHOLD_MS` から作り、リテラルを置かない)。
    const TEST_PAUSE_GAP_MS: u64 = PAUSE_THRESHOLD_MS + 1;

    fn press(t_ms: u64, category: KeyCategory) -> CategorizedPress {
        CategorizedPress { t_ms, category }
    }

    /// AC2: 連続する削除の走りの長さと本数が手計算と一致すること。
    #[test]
    fn delete_runs_match_hand_computed() {
        // 1 つの burst: A BS BS BS A BS A
        //   → 走り 1 = 長さ 3 (100ms 刻み・閉じは 4 つ目の press = t=400)
        //   → 走り 2 = 長さ 1 (閉じは t=600)
        let presses = vec![
            press(0, KeyCategory::Alphabetic),
            press(100, KeyCategory::Backspace),
            press(200, KeyCategory::Backspace),
            press(300, KeyCategory::Backspace),
            press(400, KeyCategory::Alphabetic),
            press(500, KeyCategory::Backspace),
            press(600, KeyCategory::Alphabetic),
        ];
        let events = extract_revision_events(&presses);
        assert_eq!(events.runs.len(), 2, "走りは 2 本");
        assert_eq!(events.runs[0].len, 3);
        assert_eq!(events.runs[0].closed_at_ms, 400);
        assert!(!events.runs[0].after_first_in_burst);
        assert_eq!(events.runs[0].closed_by_insert_at_ms, Some(400));
        assert_eq!(events.runs[1].len, 1);
        assert_eq!(events.runs[1].closed_at_ms, 600);
        assert!(
            events.runs[1].after_first_in_burst,
            "2 本目なので after_first_in_burst"
        );
        assert_eq!(events.bursts.len(), 1);
    }

    /// U1 の裁定: 削除系以外の press は**すべて**走りを切る (navigation も切る)。
    #[test]
    fn navigation_cuts_the_delete_run() {
        let presses = vec![
            press(0, KeyCategory::Backspace),
            press(100, KeyCategory::Navigation),
            press(200, KeyCategory::Backspace),
            press(300, KeyCategory::Alphabetic),
        ];
        let events = extract_revision_events(&presses);
        assert_eq!(events.runs.len(), 2, "navigation を挟むと 2 本になる");
        assert_eq!(events.runs[0].len, 1);
        assert_eq!(events.runs[0].closed_at_ms, 100);
        assert_eq!(
            events.runs[0].closed_by_insert_at_ms, None,
            "navigation は挿入系ではないので del→ins ではない"
        );
        assert_eq!(events.runs[1].closed_by_insert_at_ms, Some(300));
    }

    /// U4 の裁定: 走りは長さ 1 から数える。
    #[test]
    fn a_single_delete_press_is_a_run() {
        let presses = vec![
            press(0, KeyCategory::Alphabetic),
            press(100, KeyCategory::Delete),
            press(200, KeyCategory::Alphabetic),
        ];
        let events = extract_revision_events(&presses);
        assert_eq!(events.runs.len(), 1);
        assert_eq!(events.runs[0].len, 1);
    }

    /// U3 の裁定: 走りは burst をまたがない。pause を挟んだ削除系は別の走りになり、
    /// 前の走りは **burst の閉じ**で確定する。
    #[test]
    fn a_run_does_not_span_a_pause() {
        let presses = vec![
            press(0, KeyCategory::Backspace),
            press(TEST_PAUSE_GAP_MS, KeyCategory::Backspace),
        ];
        let events = extract_revision_events(&presses);
        assert_eq!(events.bursts.len(), 2, "pause で burst が 2 つに切れる");
        assert_eq!(events.runs.len(), 2, "走りも 2 本になる");
        assert_eq!(events.runs[0].len, 1);
        assert_eq!(
            events.runs[0].closed_at_ms, PAUSE_THRESHOLD_MS,
            "1 本目は burst の閉じ (最後の press + PAUSE_THRESHOLD_MS) で確定する"
        );
        assert!(!events.runs[1].after_first_in_burst, "別 burst の 1 本目");
    }

    /// ⚠️★★★ `burst_ranges` は `extract_pause_resumptions` の出力と**値の一致で同じ順に歩く**。
    /// ★ その論証が load-bearing なので、**pause 境界のちょうどそこに同時刻打鍵がある**入力で固定する
    /// (2026-09-20・`code-reviewer` M2)。⚠️ 将来 `extract_pause_resumptions` が dedup や
    /// 添字返しに変わると、ここが赤くなって気づける。
    #[test]
    fn burst_ranges_survive_same_timestamp_presses_at_a_pause_boundary() {
        let gap = TEST_PAUSE_GAP_MS;
        // pause を終える press が同時刻に 2 つある
        let at_boundary = vec![
            press(0, KeyCategory::Alphabetic),
            press(gap, KeyCategory::Alphabetic),
            press(gap, KeyCategory::Alphabetic),
        ];
        assert_eq!(
            burst_ranges(&at_boundary),
            vec![(0, 1), (1, 3)],
            "2 つ目の同時刻 press は同じ burst に入る (⚠️ 間隔 0ms は pause ではない)"
        );

        // pause の両側に同時刻打鍵がある
        let both_sides = vec![
            press(0, KeyCategory::Alphabetic),
            press(0, KeyCategory::Alphabetic),
            press(gap, KeyCategory::Alphabetic),
            press(gap, KeyCategory::Alphabetic),
            press(2 * gap, KeyCategory::Alphabetic),
        ];
        assert_eq!(burst_ranges(&both_sides), vec![(0, 2), (2, 4), (4, 5)]);

        // 閾値ちょうどは pause (★ extract_pause_resumptions が `>=` である SSOT と揃う)
        let exact = vec![
            press(0, KeyCategory::Alphabetic),
            press(PAUSE_THRESHOLD_MS, KeyCategory::Alphabetic),
        ];
        assert_eq!(burst_ranges(&exact), vec![(0, 1), (1, 2)]);
    }

    /// AC6: `category_bigram_repeat_rate` が定義 (U2) と一致すること。
    #[test]
    fn category_bigram_repeat_rate_matches_definition() {
        // A A A B → bigram は (A,A) (A,A) (A,B) の 3 個・種類は 2 → 1 − 2/3 = 1/3
        let presses = vec![
            press(0, KeyCategory::Alphabetic),
            press(100, KeyCategory::Alphabetic),
            press(200, KeyCategory::Alphabetic),
            press(300, KeyCategory::Numeric),
        ];
        let rate = category_bigram_repeat_rate(&presses).expect("bigram がある");
        assert!((rate - 1.0 / 3.0).abs() < 1e-12, "rate = {rate}");

        // 重複が 1 つも無い列は 0
        let distinct = vec![
            press(0, KeyCategory::Alphabetic),
            press(100, KeyCategory::Numeric),
            press(200, KeyCategory::Space),
        ];
        assert_eq!(category_bigram_repeat_rate(&distinct), Some(0.0));

        // press 1 件の burst は bigram が 0 個 → None (⚠️ 0.0 で埋めない)
        assert_eq!(
            category_bigram_repeat_rate(&[press(0, KeyCategory::Alphabetic)]),
            None
        );
    }

    /// `di_alternations` は `{D, I}` の交替だけを数える (どちらでもない press は列から外す)。
    #[test]
    fn di_alternations_counts_only_d_i_changes() {
        // D D I I D → 圧縮すると D I D → 交替 2 回
        let presses = vec![
            press(0, KeyCategory::Backspace),
            press(100, KeyCategory::Backspace),
            press(200, KeyCategory::Alphabetic),
            press(300, KeyCategory::Alphabetic),
            press(400, KeyCategory::Backspace),
        ];
        assert_eq!(di_alternations(&presses), 2);

        // navigation は列から外れるので D nav D は交替 0 回
        let with_navigation = vec![
            press(0, KeyCategory::Backspace),
            press(100, KeyCategory::Navigation),
            press(200, KeyCategory::Backspace),
        ];
        assert_eq!(di_alternations(&with_navigation), 0);

        // D/I の press が 1 つも無い burst は 0 (★ 交替が無かった、という観測)
        assert_eq!(di_alternations(&[press(0, KeyCategory::Navigation)]), 0);
    }

    /// AC3: 純関数が **本文を参照していない**こと —— 引数は `KeyCategory` と時刻だけであり、
    /// 同じ category に写る別の VK へ差し替えても結果が変わらない。
    #[test]
    fn revision_uses_no_text_content() {
        let a_press = KeyPress {
            timestamp: 0,
            vk: 0x41, // 'A'
        };
        let z_press = KeyPress {
            timestamp: 0,
            vk: 0x5A, // 'Z' — 同じ Alphabetic
        };
        assert_eq!(
            KeyCategory::from_vk(a_press.vk),
            KeyCategory::from_vk(z_press.vk)
        );

        let with_a = vec![
            a_press,
            KeyPress {
                timestamp: 100,
                vk: 0x08,
            },
            KeyPress {
                timestamp: 200,
                vk: 0x41,
            },
        ];
        let with_z: Vec<KeyPress> = with_a
            .iter()
            .map(|k| KeyPress {
                timestamp: k.timestamp,
                vk: if k.vk == 0x41 { 0x5A } else { k.vk },
            })
            .collect();

        let column_a = revision_column(&with_a, 4, 0, super::super::ENGINE_TICK_MS);
        let column_z = revision_column(&with_z, 4, 0, super::super::ENGINE_TICK_MS);
        let json_a = serde_json::to_string(&column_a).expect("serialize");
        let json_z = serde_json::to_string(&column_z).expect("serialize");
        assert_eq!(json_a, json_z, "どの文字かは結果に影響しない");
    }

    /// ⚠️★★★ 挿入系 / 削除系の写像が **ADR-030 D3 の SSOT (`super::key_net_delta`) と
    /// 全 VK で一致する**こと (判断 1 の「却下した細部」)。
    /// ★ これが赤くなったら、`KeyCategory` 側ではなく **SSOT が動いた**ことを疑う。
    #[test]
    fn category_class_matches_key_net_delta() {
        for vk in 0u32..=0xFF {
            let key = KeyPress { timestamp: 0, vk };
            let expected = match super::super::key_net_delta(&key) {
                1 => PressClass::Insert,
                -1 => PressClass::Delete,
                _ => PressClass::Neither,
            };
            let actual = class_of(KeyCategory::from_vk(vk));
            assert_eq!(actual, expected, "vk = {vk:#04x} の分類が SSOT と食い違う");
        }
    }

    /// tick への帰属 (U3) と、`null` / `0` の使い分け。
    #[test]
    fn values_land_on_the_tick_where_the_unit_closes() {
        let tick = super::super::ENGINE_TICK_MS;
        // tick 0 に BS×2、tick 1 の頭 (t = tick) に挿入系 → 走りは tick 1 で閉じる。
        let keys = vec![
            KeyPress {
                timestamp: 0,
                vk: 0x08,
            },
            KeyPress {
                timestamp: 100,
                vk: 0x08,
            },
            KeyPress {
                timestamp: tick,
                vk: 0x41,
            },
        ];
        let column = revision_column(&keys, 5, 0, tick);

        assert_eq!(column[0].delete_runs, 0, "tick 0 ではまだ閉じていない");
        assert_eq!(column[0].delete_run_max, None, "⚠️ 0 で埋めない");
        assert_eq!(column[1].delete_runs, 1);
        assert_eq!(column[1].delete_run_max, Some(2));
        assert_eq!(column[1].del_to_ins_transitions, 1);
        assert_eq!(column[1].delete_runs_after_first_in_burst, 0);

        // burst の閉じは最後の press (t = tick) + PAUSE_THRESHOLD_MS。
        let burst_tick = ((tick + PAUSE_THRESHOLD_MS) / tick) as usize;
        assert_eq!(
            column[burst_tick].di_alternations_max,
            Some(1),
            "D → I の 1 回"
        );
        assert!(column[burst_tick].category_bigram_repeat_rate.is_some());
        assert_eq!(
            column[0].category_bigram_repeat_rate, None,
            "burst が閉じていない tick は null"
        );
    }

    /// 境界入力で panic しないこと（空入力 / 同時刻打鍵 / 時刻逆行 / グリッド外の閉じ）。
    #[test]
    fn revision_column_handles_degenerate_inputs() {
        let tick = super::super::ENGINE_TICK_MS;
        assert!(revision_column(&[], 0, 0, tick).is_empty());

        let same_ts = vec![
            KeyPress {
                timestamp: 500,
                vk: 0x08,
            },
            KeyPress {
                timestamp: 500,
                vk: 0x08,
            },
            KeyPress {
                timestamp: 500,
                vk: 0x41,
            },
        ];
        let column = revision_column(&same_ts, 3, 0, tick);
        assert_eq!(column[0].delete_runs, 1);
        assert_eq!(column[0].delete_run_max, Some(2));

        // 時刻逆行の入力でも昇順として扱われる
        let reversed = vec![
            KeyPress {
                timestamp: 200,
                vk: 0x41,
            },
            KeyPress {
                timestamp: 100,
                vk: 0x08,
            },
            KeyPress {
                timestamp: 0,
                vk: 0x41,
            },
        ];
        let column = revision_column(&reversed, 3, 0, tick);
        assert_eq!(column[0].delete_runs, 1);

        // グリッドが短いと burst の閉じが外に落ちる → 黙って捨てる (細目 3)
        let column = revision_column(&reversed, 1, 0, tick);
        assert_eq!(column.len(), 1);
        assert_eq!(column[0].di_alternations_max, None);

        // 時刻が u64 の上限付近でも panic しない (burst の閉じの加算が溢れる)
        let overflow = vec![KeyPress {
            timestamp: u64::MAX,
            vk: 0x08,
        }];
        let column = revision_column(&overflow, 1, 0, tick);
        assert_eq!(column[0].delete_runs, 0);
    }
}
