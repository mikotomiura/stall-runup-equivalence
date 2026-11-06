//! `Q`（evidence quality）— その tick の証拠がどれだけ信頼できるかを、**計器側の事実だけ**
//! で記述する (issue 007)。
//!
//! ⚠️★★★ **`Q` は状態特徴ではない。** `.idea/survey:194` 逐語「欠測を 0 として入力せず、
//! 推論の抑制・abstention に使う」。`ADR-030 D10` 逐語「`Q` は `none` / `defer` へ退く根拠で
//! あって、認知状態の説明変数ではない」。本モジュールは出力に載せるだけで、他 Tier の計算
//! へは 1 バイトも渡さない (`super::build_feature_rows` の呼び出し順を参照)。
//!
//! # 欠測理由は計器側の事実に限る
//!
//! `missing_reasons` は次の 3 語だけから成る許可リストである。**被験者由来の理由
//! (一時停止・区間削除) は絶対に含めない** — `gse-replay --metrics --json` の出力は
//! `gse-dataset` へ共有されるため、被験者由来の理由が混じると「端末内で完結する」という
//! 削除の約束が研究者側の共有物に漏れる (`docs/o1-deletion-ledger.md`)。
//!
//! - `"capability_undeclared"` — `meta` 行に `capabilities` が無い (全行)
//! - `"hook_dead"` — `observability_mask.hook == false`、または tick 内に `alive=false` の行がある
//!   (⚠️ `observability_mask.hook` は下記の規則で `null` になりうるが、`hook_dead` はそれとは
//!   独立に tick 内の生の行だけからも立つ)
//! - `"dropped_events"` — tick 内に `dropped_delta` を**持ち**、かつ `> 0` の `hook_health` 行がある
//!
//! `feat` 行の穴 (削除・一時停止・計器欠損のいずれか、原因は区別しない・区別してはいけない)
//! は `missing_reasons` に**理由を付けない** —— `observability_mask.feat = false` とだけ書く
//! (ADR-030 D8 (b-2))。観測区間は独自に判定せず `super::stall_onset::observed_intervals`
//! (008) をそのまま使う。
//!
//! # 2026-09-18 改訂 (判断 24・レビュー 2 本の実測を受けて)
//!
//! 1. **`hook` の持ち越しを観測区間で切る** (security-checker MEDIUM): 穴をまたいで古い
//!    `alive` 状態を持ち越すと、削除・一時停止の後に「偽の健全」を返す実測不具合があった。
//!    `feat` 行を 1 本以上持つファイルでは、穴の tick (`mask.feat == false`) は常に
//!    `mask.hook = None`、穴から観測区間へ戻る tick では持ち越しを読み捨ててから
//!    (`None` に戻してから) 読む。`feat` 行を 1 本も持たないファイルでは従来どおり
//!    単純な持ち越しのまま (`instrument_rich.ndjson` の正解表は変わらない)。
//! 2. **`dropped_delta` を `0` で埋めない** (code-reviewer M1): 旧形式のログ行に無いことは
//!    「値が無い」であって「`0` だった」ではない。tick 内の行が全て `dropped_delta` を
//!    持つときだけ飽和加算で合算し、1 本でも欠ければ `None`。
//! 3. **`capabilities` の形と大きさを検査する** (security-checker LOW-3):
//!    JSON オブジェクトでない、または直列化して 4096 バイトを超えるものは
//!    収集側 (`bin/replay.rs::collect_meta_capabilities`) が `Err` を返す。
//!
//! # 2026-09-20 改訂 (判断 3・封筒)
//!
//! ⚠️★★★ **`capabilities` の値は `Quality` から外れ、出力の封筒へ移った**
//! (`bin/replay.rs::FeatureProvenance::capabilities`)。★ セッション不変の事実を
//! 行ごとに複製しないため。⚠️ **「宣言が無い」という事実は tick 単位の欠測理由なので
//! `missing_reasons` に残る** —— 本モジュールは値ではなく
//! `capabilities_declared: bool` を受け取る。

use super::stall_onset::observed_intervals;

/// `hook_health` 行 1 件 (issue 007 用の集約入力)。
///
/// `dropped_delta` は `Option<u64>`。2026-08-19 より前のログにはこのフィールドが無いため、
/// 収集側 (`bin/replay.rs::HookHealthDto`) は欠落をそのまま `None` として渡す。
/// ⚠️★★★ 2026-09-18 改訂 (判断 24): 旧版はここを `0` に正規化していたが、「値が無い」と
/// 「`0` だった」は区別できる情報であり、`0` に畳むと情報が失われる (`missing_dropped_delta_is_null_not_zero`
/// が回帰を張る)。
#[derive(Debug, Clone, Copy)]
pub struct HookHealthEvent {
    pub t_ms: u64,
    pub alive: bool,
    pub starved_ms: u64,
    pub rehook: bool,
    pub dropped_delta: Option<u64>,
}

/// tick 内に `hook_health` 行が 1 本以上あるときだけ埋まる集約値。
///
/// `dropped_delta`: tick 内の行が**すべて** `dropped_delta` を持つときだけ飽和加算で合算する。
/// 1 本でも欠けていれば `None` (★ `0` で埋めない・判断 14 と同じ規律)。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct QualityHookHealth {
    pub alive: bool,
    pub starved_ms: u64,
    pub rehook: bool,
    pub dropped_delta: Option<u64>,
}

/// `feat` 行 (観測区間) と `hook_health` (生存監視) それぞれの可観測性。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ObservabilityMask {
    /// tick が 008 の観測区間 (`observed_intervals`) と交わるか。理由は付けない。
    ///
    /// ⚠️★★★ **retrospective**: 区間の右端は、tick の時刻より後にある `feat` 行までの
    /// 間隔が `hole_gap_ms` を超えるかどうかで決まる (`observed_intervals` は全 `feat_ts` を
    /// 一括で見て区間を切る)。★ したがって直近 tick の `feat` は、その tick が閉じた
    /// 時点だけでは確定せず、最大 `hole_gap_ms` 程度先読みしてから確定する
    /// (`activity` 等の causal な Tier とは異なる。`super` モジュール doc の
    /// 「causal / retrospective の区分」を参照)。
    pub feat: bool,
    /// `t < tick_end` を満たす最後の `hook_health` 行の `alive` (状態の持ち越し)。
    /// そういう行が 1 つも無ければ `None` (= 判定不能。ADR-013 D1)。
    ///
    /// ⚠️★★★ **2026-09-18 改訂 (判断 24)**: `feat` 行を持つファイルでは、
    /// `feat == false` の tick (穴) では常に `None` になる (見えていない間の状態を
    /// 主張しない)。穴から観測区間へ戻る tick では、持ち越した状態を読み捨ててから
    /// (`None` に戻してから) その tick 自身の `hook_health` 行を読む
    /// (穴の前の状態を穴の後へ持ち込まない。⚠️ 一時停止中にフックが死ぬと `alive=false` が
    /// NDJSON に出ない経路が実在するため、原因は区別しない)。`feat` 行を 1 本も持たない
    /// ファイルでは従来どおり単純な状態の持ち越しになる。
    pub hook: Option<bool>,
}

/// 1 tick ぶんの `Q`。
///
/// # causal / retrospective の区分
///
/// `hook_health` / `observability_mask.hook` は、その tick までに観測された `hook_health` 行
/// だけで決まる **causal** な値。⚠️ 一方 `observability_mask.feat` は **retrospective**
/// (`ObservabilityMask::feat` の doc 参照) —— 観測区間の右端が確定するまで、その tick の
/// `feat` 行を最大 `hole_gap_ms` 程度先読みして初めて決まる。
///
/// # ⚠️★★★ `capabilities` はここに無い (2026-09-20・判断 3)
///
/// `meta` 行の `capabilities` は **セッション不変の事実**であり、行ごとに複製する意味が無い。
/// `gse-replay --features` の出力では **封筒の `provenance.capabilities`** に 1 回だけ載る
/// (`bin/replay.rs::FeatureProvenance`)。★ 本構造体が持つのは
/// **tick ごとに値が変わりうるもの**だけである。
/// ⚠️ ただし「宣言が無い」という事実は tick 単位の欠測理由として意味を持つので、
/// `missing_reasons` の `"capability_undeclared"` は**そのまま残る**
/// (`quality_column` の `capabilities_declared` 引数から立つ)。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Quality {
    pub hook_health: Option<QualityHookHealth>,
    pub missing_reasons: Vec<String>,
    pub observability_mask: ObservabilityMask,
}

/// tick が観測区間のどれかと交わるかどうかの列 (ADR-030 D8 (b-2) の観測区間をそのまま使う)。
/// tick `k` は `[t_k, t_k + tick_ms)`、区間は `[a, b]` (閉区間・`feat` 行の実測値)。
/// 交わる条件は `t_k <= b && t_k + tick_ms > a`。
fn feat_mask_column(
    feat_ts: &[u64],
    hole_gap_ms: u64,
    tick_count: usize,
    grid_origin_ms: u64,
    tick_ms: u64,
) -> Vec<bool> {
    let intervals = observed_intervals(feat_ts, hole_gap_ms);
    let mut mask = vec![false; tick_count];
    let mut iv_idx = 0usize;
    for (k, slot) in mask.iter_mut().enumerate() {
        let t_k = grid_origin_ms + k as u64 * tick_ms;
        let tick_end = t_k + tick_ms;
        // 過去 (b < t_k) になった区間は以後の tick とも交わらないので読み飛ばす。
        while iv_idx < intervals.len() && intervals[iv_idx].1 < t_k {
            iv_idx += 1;
        }
        if let Some(&(a, b)) = intervals.get(iv_idx) {
            *slot = t_k <= b && tick_end > a;
        }
    }
    mask
}

/// 1 Hz tick 単位の `Quality` 列を作る。`super::build_feature_rows` から呼ばれる。
///
/// - `capabilities_declared`: `meta` 行に `capabilities` の宣言があったか。
///   ⚠️★★★ **値そのものは受け取らない** (2026-09-20・判断 3) —— 値は封筒
///   (`bin/replay.rs::FeatureProvenance::capabilities`) が 1 回だけ持つ。ここで要るのは
///   「宣言があったか」という真偽だけで、それが `"capability_undeclared"` を立てる。
/// - `hook_health`: `hook_health` 行の列 (時刻昇順でなくてよい)。
/// - `feat_ts` / `hole_gap_ms`: 008 の観測区間の入力と同じもの (別に読み直さない)。
pub fn quality_column(
    tick_count: usize,
    grid_origin_ms: u64,
    tick_ms: u64,
    capabilities_declared: bool,
    hook_health: &[HookHealthEvent],
    feat_ts: &[u64],
    hole_gap_ms: u64,
) -> Vec<Quality> {
    let mut sorted_hh: Vec<HookHealthEvent> = hook_health.to_vec();
    sorted_hh.sort_by_key(|e| e.t_ms);

    let feat_mask = feat_mask_column(feat_ts, hole_gap_ms, tick_count, grid_origin_ms, tick_ms);
    // ⚠️★★★ 判断 24: 持ち越しを区間で切るのは「feat 行を 1 本以上持つファイル」だけ
    // (原因は区別しない・feat 行の存在だけに依存する・プライバシー中立)。
    let has_feat_rows = !feat_ts.is_empty();
    let capability_undeclared = !capabilities_declared;

    let mut hh_idx = 0usize;
    let mut carried_alive: Option<bool> = None;
    let mut prev_feat = false;
    let mut rows = Vec::with_capacity(tick_count);
    for (k, &feat) in feat_mask.iter().enumerate() {
        let t_k = grid_origin_ms + k as u64 * tick_ms;
        let tick_end = t_k + tick_ms;

        // 判断 24: 穴 (feat=false) から観測区間 (feat=true) へ戻る tick では、
        // その tick 自身の行を読む前に持ち越しを捨てる (穴の前の状態を穴の後へ持ち込まない)。
        if has_feat_rows && k > 0 && feat && !prev_feat {
            carried_alive = None;
        }

        let mut tick_events: Vec<HookHealthEvent> = Vec::new();
        while hh_idx < sorted_hh.len() && sorted_hh[hh_idx].t_ms < tick_end {
            let ev = sorted_hh[hh_idx];
            carried_alive = Some(ev.alive);
            if ev.t_ms >= t_k {
                tick_events.push(ev);
            }
            hh_idx += 1;
        }

        let hook_health_field = if tick_events.is_empty() {
            None
        } else {
            // 判断 24: dropped_delta は tick 内の行が全て持つときだけ飽和加算で合算する。
            let dropped_delta = if tick_events.iter().all(|e| e.dropped_delta.is_some()) {
                Some(tick_events.iter().fold(0u64, |acc, e| {
                    acc.saturating_add(e.dropped_delta.unwrap_or(0))
                }))
            } else {
                None
            };
            Some(QualityHookHealth {
                alive: tick_events.iter().all(|e| e.alive),
                starved_ms: tick_events.iter().map(|e| e.starved_ms).max().unwrap_or(0),
                rehook: tick_events.iter().any(|e| e.rehook),
                dropped_delta,
            })
        };

        // 判断 24: 穴の tick では「見えていない間の状態を主張しない」= null にする。
        let hook_mask = if has_feat_rows && !feat {
            None
        } else {
            carried_alive
        };

        let mask = ObservabilityMask {
            feat,
            hook: hook_mask,
        };

        let mut reasons: Vec<String> = Vec::new();
        if capability_undeclared {
            reasons.push("capability_undeclared".to_string());
        }
        // ⚠️ hook_dead は mask.hook (null になりうる) とは独立に、tick 内の生の行からも立つ。
        let hook_dead = mask.hook == Some(false) || tick_events.iter().any(|e| !e.alive);
        if hook_dead {
            reasons.push("hook_dead".to_string());
        }
        if tick_events
            .iter()
            .any(|e| e.dropped_delta.map(|d| d > 0).unwrap_or(false))
        {
            reasons.push("dropped_events".to_string());
        }
        reasons.sort();

        rows.push(Quality {
            hook_health: hook_health_field,
            missing_reasons: reasons,
            observability_mask: mask,
        });

        prev_feat = feat;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // ⚠️ 2026-09-20 (判断 3): `caps()` ヘルパは消した。`quality_column` は
    // `capabilities` の**値**を受け取らなくなり、テストが渡すのは真偽値だけになったため。
    // ★ 値が封筒へ正しく載ることは `bin/replay.rs` の
    // `envelope_carries_meta_facts_and_sharing_marker` が固定する。

    fn hh(t_ms: u64, alive: bool, dropped_delta: Option<u64>) -> HookHealthEvent {
        HookHealthEvent {
            t_ms,
            alive,
            starved_ms: 0,
            rehook: false,
            dropped_delta,
        }
    }

    /// AC2: `missing_reasons` はどんな入力でも 3 語の許可リストの外に出ない。
    #[test]
    fn missing_reasons_are_instrument_side_only() {
        const ALLOWED: [&str; 3] = ["capability_undeclared", "dropped_events", "hook_dead"];
        let scenarios: Vec<Vec<Quality>> = vec![
            quality_column(5, 0, 1000, false, &[], &[], 2000),
            quality_column(5, 0, 1000, true, &[hh(500, false, Some(3))], &[], 2000),
            quality_column(
                10,
                0,
                1000,
                true,
                &[hh(2500, true, Some(0)), hh(7200, false, Some(9))],
                &[0, 1000, 9000],
                2000,
            ),
        ];
        for rows in scenarios {
            for row in rows {
                for reason in &row.missing_reasons {
                    assert!(
                        ALLOWED.contains(&reason.as_str()),
                        "許可リスト外の理由が出た: {reason}"
                    );
                }
            }
        }
    }

    /// AC3: 被験者由来語 (一時停止 / 区間削除) がどんな入力でも出力に現れない。
    /// ⚠️ 2026-09-20 (判断 3) 以後、`capabilities` は行に無い (封筒が持つ) ので、
    /// ここで見るのは `missing_reasons` だけでよい。★ 封筒側の `capabilities` は
    /// 「加工せずそのまま載せる」フィールドなので**この検査の対象外**であり、
    /// `007-acceptance.sh` も封筒の `capabilities` を除外して走査している。
    #[test]
    fn participant_originated_reasons_are_never_emitted() {
        let forbidden = ["recording_pause", "redaction", "redact", "participant"];
        let rows = quality_column(
            5,
            0,
            1000,
            true,
            &[hh(500, false, Some(1))],
            &[0, 4500],
            2000,
        );
        for row in rows {
            for reason in &row.missing_reasons {
                for f in forbidden {
                    assert!(
                        !reason.contains(f),
                        "missing_reasons に被験者由来語が出た: {reason}"
                    );
                }
            }
        }
    }

    /// AC4: 欠測は `null`。`0` や空配列・空オブジェクトへフォールバックしない。
    #[test]
    fn missing_is_null_not_zero() {
        let rows = quality_column(3, 0, 1000, false, &[], &[], 2000);
        for row in &rows {
            assert_eq!(row.hook_health, None, "hook_health 行が無ければ null");
            assert_eq!(
                row.observability_mask.hook, None,
                "hook_health 未観測なら null"
            );
        }
    }

    /// AC7: 宣言が無いことは `missing_reasons` の `capability_undeclared` で表す。
    ///
    /// ⚠️★★★ 2026-09-20 改訂 (判断 3): 旧版はここで `row.capabilities == None` も見ていたが、
    /// `capabilities` は**封筒へ移した**ので行には無い。「宣言が無いことを `null` で表し、
    /// 空配列・空オブジェクトで埋めない」という規律自体は封筒側へ移り、
    /// `envelope-acceptance.sh` が `provenance.capabilities == null` を検査する。
    #[test]
    fn absent_capabilities_is_reported_as_missing_reason() {
        let rows = quality_column(2, 0, 1000, false, &[], &[], 2000);
        for row in &rows {
            assert_eq!(
                row.missing_reasons,
                vec!["capability_undeclared".to_string()]
            );
        }
    }

    /// ★ 判断 3 の回帰: 宣言の**有無だけ**が行に効き、値そのものは行に現れない。
    #[test]
    fn declared_capabilities_do_not_appear_in_rows() {
        let declared = quality_column(2, 0, 1000, true, &[], &[], 2000);
        for row in &declared {
            assert!(row.missing_reasons.is_empty());
        }
        // ⚠️ 行を直列化しても `capabilities` というキーは 1 つも出ない (封筒が持つ)。
        let json = serde_json::to_string(&declared).expect("serialize");
        assert!(
            !json.contains("capabilities"),
            "行に capabilities が漏れている: {json}"
        );
    }

    /// AC9: `feat` の穴には理由を付けない。`observability_mask.feat = false` とだけ書く。
    #[test]
    fn feat_hole_is_masked_without_a_reason() {
        // 観測区間: [0, 1000] と [5000, 6000] (間の 2000..5000 が穴)。hole_gap_ms=1000。
        let feat_ts = [0, 1000, 5000, 6000];
        let rows = quality_column(7, 0, 1000, true, &[], &feat_ts, 1000);
        // tick 2,3,4 (t_k = 2000,3000,4000) は穴 (直前の区間終端 1000 から 4000 は超えず、
        // 区間 [5000,6000] とも交わらない)。
        for idx in [2usize, 3, 4] {
            assert!(!rows[idx].observability_mask.feat, "tick {idx} は穴のはず");
            assert!(
                rows[idx].missing_reasons.is_empty(),
                "穴 tick {idx} に理由が付いた: {:?}",
                rows[idx].missing_reasons
            );
        }
        assert!(rows[0].observability_mask.feat);
        assert!(rows[5].observability_mask.feat);
    }

    /// AC10: `mask.hook` は状態を持ち越す。alive=false の後は alive=true の行が来るまで
    /// `false` のまま (`hook_dead` も立ち続ける)。
    #[test]
    fn hook_state_carries_forward_until_next_row() {
        let rows = quality_column(
            6,
            0,
            1000,
            true,
            &[hh(500, false, Some(0)), hh(4500, true, Some(0))],
            &[],
            2000,
        );
        assert_eq!(rows[0].observability_mask.hook, Some(false));
        assert_eq!(rows[1].observability_mask.hook, Some(false));
        assert_eq!(rows[2].observability_mask.hook, Some(false));
        assert_eq!(rows[3].observability_mask.hook, Some(false));
        assert!(rows[3].missing_reasons.contains(&"hook_dead".to_string()));
        assert_eq!(rows[4].observability_mask.hook, Some(true));
        assert!(!rows[4].missing_reasons.contains(&"hook_dead".to_string()));
        assert_eq!(rows[5].observability_mask.hook, Some(true));
    }

    /// 境界: `hook_health` 行が 1 本も無い入力で panic しない。
    #[test]
    fn empty_hook_health_produces_null_mask_for_every_row() {
        let rows = quality_column(4, 0, 1000, false, &[], &[], 2000);
        assert_eq!(rows.len(), 4);
        for row in rows {
            assert_eq!(row.observability_mask.hook, None);
            assert_eq!(row.hook_health, None);
        }
    }

    /// AC13 (判断 24・security-checker MEDIUM): 観測区間の穴をまたいで `mask.hook` の
    /// 古い状態を持ち越さない。穴の tick は `mask.hook = null`、穴から観測区間へ戻る tick
    /// では持ち越しを捨てる (穴の前の `alive=true` の残り香が、穴の後に「健全」と読めてはならない)。
    #[test]
    fn hook_carry_is_cut_at_observed_interval_boundaries() {
        // 観測区間: [0,2000] と [5000,7000] (間の 3000..4000 の幅が穴)。hole_gap_ms=1000。
        let feat_ts = [0u64, 1000, 2000, 5000, 6000, 7000];
        let hh_events = [
            hh(500, true, Some(0)),   // 穴の前: alive=true (tick 0)
            hh(6500, false, Some(0)), // 穴の後: alive=false (tick 6)
        ];
        let rows = quality_column(8, 0, 1000, true, &hh_events, &feat_ts, 1000);
        assert_eq!(rows.len(), 8);

        let feat_flags: Vec<bool> = rows.iter().map(|r| r.observability_mask.feat).collect();
        assert_eq!(
            feat_flags,
            vec![true, true, true, false, false, true, true, true],
            "観測区間のマスクが前提と違う: {feat_flags:?}"
        );

        let hook_flags: Vec<Option<bool>> =
            rows.iter().map(|r| r.observability_mask.hook).collect();
        assert_eq!(
            hook_flags,
            vec![
                Some(true),
                Some(true),
                Some(true),
                None,
                None,
                None, // ★ 穴の直後 (tick 5): 穴の前の Some(true) を持ち越さない
                Some(false),
                Some(false),
            ],
            "穴をまたいだ持ち越しが切れていない: {hook_flags:?}"
        );

        for idx in [3usize, 4, 5] {
            assert!(
                !rows[idx].missing_reasons.contains(&"hook_dead".to_string()),
                "tick {idx} は hook_dead を主張してはならない (見えていない間の状態は主張しない・持ち越しは捨てた)"
            );
        }
        for idx in [6usize, 7] {
            assert!(
                rows[idx].missing_reasons.contains(&"hook_dead".to_string()),
                "tick {idx} は hook_dead が立つはず"
            );
        }
    }

    /// AC14 (判断 24・code-reviewer M1): tick 内の `hook_health` 行に `dropped_delta` が
    /// 1 本でも欠けていれば集約は `null`。`0` で埋めない。
    #[test]
    fn missing_dropped_delta_is_null_not_zero() {
        // 1 行だけ、かつ欠けている → 集約は null、dropped_events も立たない。
        let rows_missing = quality_column(1, 0, 1000, true, &[hh(100, true, None)], &[], 2000);
        assert_eq!(
            rows_missing[0].hook_health,
            Some(QualityHookHealth {
                alive: true,
                starved_ms: 0,
                rehook: false,
                dropped_delta: None,
            })
        );
        assert!(
            !rows_missing[0]
                .missing_reasons
                .contains(&"dropped_events".to_string()),
            "dropped_delta が無ければ dropped_events は立たないはず"
        );

        // 2 行のうち 1 行が欠けていれば、揃っている方の値があっても集約は null。
        let rows_partial = quality_column(
            1,
            0,
            1000,
            true,
            &[hh(100, true, Some(5)), hh(200, true, None)],
            &[],
            2000,
        );
        assert_eq!(rows_partial[0].hook_health.unwrap().dropped_delta, None);

        // 対照: 全行が dropped_delta を持てば飽和加算で合算される。
        let rows_full = quality_column(
            1,
            0,
            1000,
            true,
            &[hh(100, true, Some(5)), hh(200, true, Some(3))],
            &[],
            2000,
        );
        assert_eq!(rows_full[0].hook_health.unwrap().dropped_delta, Some(8));
        assert!(rows_full[0]
            .missing_reasons
            .contains(&"dropped_events".to_string()));
    }

    /// AC16 (判断 24・code-reviewer mutation 指摘): 1 tick に複数行があるときの集約
    /// (AND / max / OR / 飽和加算) と理由の昇順を完全一致で固定する。
    #[test]
    fn aggregation_of_multiple_rows_in_one_tick() {
        let rows = quality_column(
            1,
            0,
            1000,
            false,
            &[
                HookHealthEvent {
                    t_ms: 100,
                    alive: true,
                    starved_ms: 100,
                    rehook: true,
                    dropped_delta: Some(1),
                },
                HookHealthEvent {
                    t_ms: 900,
                    alive: false,
                    starved_ms: 40000,
                    rehook: false,
                    dropped_delta: Some(2),
                },
            ],
            &[],
            2000,
        );
        assert_eq!(
            rows[0].hook_health,
            Some(QualityHookHealth {
                alive: false,
                starved_ms: 40000,
                rehook: true,
                dropped_delta: Some(3),
            })
        );
        assert_eq!(rows[0].observability_mask.hook, Some(false));
        assert_eq!(
            rows[0].missing_reasons,
            vec![
                "capability_undeclared".to_string(),
                "dropped_events".to_string(),
                "hook_dead".to_string(),
            ]
        );
    }

    /// AC16 続き: 同じ tick で `false → true` の順に行が並んでいても (=状態は alive で
    /// 終わっても)、tick 内に alive=false の行があった事実で `hook_dead` は立つ。
    #[test]
    fn in_tick_dead_row_sets_hook_dead_even_if_state_ends_alive() {
        let rows = quality_column(
            1,
            0,
            1000,
            true,
            &[
                HookHealthEvent {
                    t_ms: 100,
                    alive: false,
                    starved_ms: 5000,
                    rehook: false,
                    dropped_delta: Some(0),
                },
                HookHealthEvent {
                    t_ms: 900,
                    alive: true,
                    starved_ms: 0,
                    rehook: true,
                    dropped_delta: Some(0),
                },
            ],
            &[],
            2000,
        );
        assert_eq!(
            rows[0].observability_mask.hook,
            Some(true),
            "最後の行 (t=900) の alive が状態になる"
        );
        assert!(
            rows[0].missing_reasons.contains(&"hook_dead".to_string()),
            "tick 内に alive=false の行があれば hook_dead は立ち続ける: {:?}",
            rows[0].missing_reasons
        );
    }

    /// AC17 / M2: tick 境界ちょうどの行・原点より前の行・観測区間端 `t_k == b` の tick の
    /// 帰属を固定する。
    #[test]
    fn tick_boundary_rows_are_attributed_correctly() {
        // 観測区間 [1000, 2000] (hole_gap_ms=2000 で一つづきの区間)。
        let feat_ts = [1000u64, 1500, 2000];
        let hh_events = [
            HookHealthEvent {
                t_ms: 500, // 原点 (grid_origin_ms=1000) より前
                alive: false,
                starved_ms: 0,
                rehook: false,
                dropped_delta: Some(0),
            },
            HookHealthEvent {
                t_ms: 2000, // tick1 (t_k=2000) ちょうど
                alive: true,
                starved_ms: 0,
                rehook: false,
                dropped_delta: Some(0),
            },
            HookHealthEvent {
                t_ms: 3000, // tick1 の tick_end (= tick2 の t_k) ちょうど
                alive: false,
                starved_ms: 0,
                rehook: false,
                dropped_delta: Some(0),
            },
        ];
        let rows = quality_column(4, 1000, 1000, true, &hh_events, &feat_ts, 2000);
        assert_eq!(rows.len(), 4);

        // 原点より前の行: tick0 の集約 (hook_health) には含まれないが、持ち越しには使う。
        assert_eq!(
            rows[0].hook_health, None,
            "原点より前の行は tick0 の集約に含めない"
        );
        assert_eq!(
            rows[0].observability_mask.hook,
            Some(false),
            "原点より前の行も持ち越しには使う"
        );
        assert!(rows[0].missing_reasons.contains(&"hook_dead".to_string()));

        // tick1 の t_k ちょうどの行は tick1 に帰属する。
        assert!(
            rows[1].hook_health.is_some(),
            "t_k ちょうどの行は自分の tick に入る"
        );
        assert!(
            rows[1].hook_health.unwrap().alive,
            "tick1 の集約は alive=true のはず"
        );
        assert_eq!(rows[1].observability_mask.hook, Some(true));

        // tick1 の tick_end (= tick2 の t_k) ちょうどの行は tick2 に帰属し、tick1 には入らない。
        assert!(
            rows[2].hook_health.is_some(),
            "tick_end ちょうどの行は次の tick に入る"
        );
        assert!(
            !rows[2].hook_health.unwrap().alive,
            "tick_end ちょうどの行が tick1 に漏れ込んでいる可能性がある"
        );

        // 観測区間端 t_k == b (= 2000) の tick (tick1) は mask.feat == true。
        assert!(
            rows[1].observability_mask.feat,
            "区間端ちょうどの tick も観測区間に含める"
        );
    }
}
