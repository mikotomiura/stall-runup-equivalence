//! `T1d` = **再開**（issue 006・ADR-030 D2 / D3 / D8 (b-2)）。
//!
//! `stall onset` の `t0` の**後**に何が起きたか —— 生成 / 削除 / 移動、再び止まるまでの時間、
//! 安定して生成が続いた時間 —— を tick 行に載せる。
//!
//! ⚠️★★★ **retrospective（転帰側）である。** `restart` は `t0` より後を見る量なので、
//! **detection 側（`activity` / `stall_onset`）の入力に 1 つも渡さない**
//! （ADR-017 D8 / ADR-024 §Migration 逐語「再開後の動態は *アウトカム* であって、決して *入力* にしない」・
//! issue 006 AC7）。★ 配線は `stall_onset` → `restart` の**一方向だけ**である（判断 4）。
//!
//! ⚠️ 禁止用語（`docs/glossary.md`）: 認知リソースの消耗・補充の語彙は使わない。
//! ★ 「安定して生成が続いた時間」は [`Restart::stable_generation_ms`] で表す。
//!
//! # 原点と窓（★ `.steering/20260920-issues-004-006/decisions.md` 判断 2 / 3）
//!
//! ```text
//! 原点 o   = t0 + B          ★ B = StallOnsetConfig::b_ms（⚠️ 本モジュールで値を決めない・AC4）
//! 集計窓   = [o, min(同じ観測区間の次の onset の s, b + 1))      ★ E2（V1 の裁定）
//!            ⚠️ 右端 b は確定扱い（`stall_onset` の `t0 <= b` と対称・V2 の裁定）
//!            ⚠️ 窓が無い（o > b、または次の onset の s <= o）→ post_t0_* / resumption_count は None（V5）
//!
//! 生成区間 = 条件 A が false である極大区間を [o, b] で切ったもの   ★ V6 の裁定
//! r_k      = k 番目の生成区間の開始時刻（★ o より前に始まっていれば o にクリップする）
//! s'_k     = その生成区間が閉じる時刻（条件 A が true へ立ち上がる時刻）
//! ```
//!
//! ★ **V6 が要る理由（⚠️ 実測）**: `B` は `W` と同じ 30 秒系列から選ぶ（ADR-030 D4）ので、
//! **`o` に達する前に打鍵が戻っている**ことが構造的に起きる。この場合「`o` 以後の true → false の辺」は
//! 存在しないため、辺だけで読むと **速やかに再開した人ほど全フィールドが `None`** になり、意味が反転する。
//! → ★ `o` の時点で条件 A が既に `false` なら、**生成区間は `o` で開いている**と読む（左側を `o` で切る）。
//! ⚠️ `o` の時点で条件 A が `true` の場合、この定義は「最初の true → false の辺」と完全に一致する。
//!
//! # 集計窓に依存する量・しない量
//!
//! ★ `restop_ms` / `stable_generation_ms` / `stable_generation_net_presses` は
//! **束ね方（E1 / E2）に依存しない** —— `r_1` と `s'_1` は**観測区間**の中の条件 A の辺だけで決まる
//! （issue 006 Background 逐語）。⚠️ 窓に依存するのは `post_t0_*` と `resumption_count` だけである。
//!
//! ⚠️★★★ **E2 の既知の代償（★ V1 で E1 を入れるまで残る）**: 次の onset が `o` より前にあると、
//! 窓が消えて `post_t0_*` / `resumption_count` が `None` になる一方、
//! `restop_ms` / `stable_generation_ms` は**次の停滞に属する時間帯**を測ることがある。
//! ★ これは [V146] が警告した過剰分割であり、`blockers.md` 起票 20 に上げてある。
//!
//! # 打ち切り（⚠️ `censored` フィールドは持たない・判断 2 の V4）
//!
//! onset のある行で `restop_ms` が `None` になる経路は、**観測区間の右端 `b` に達するまで
//! `s'_1` を観測できなかった右側打ち切りだけ**である。★ したがって打ち切りか否かは
//! `stall_onset.t0_ms != null && restart.restop_ms == null` で完全に復元でき、
//! ⚠️ boolean を 1 つ足しても情報は増えない。★ 生存解析に要る**追跡時間**は別途
//! 研究者判断（`blockers.md` 起票 20）である。

use crate::analysis::anonymize::KeyCategory;
use crate::analysis::metrics::KeyPress;

use super::stall_onset::{ConditionAInterval, StallOnset, StallOnsetConfig};

/// `T1d` の 1 tick ぶんの値。★ onset が立った tick だけ値が埋まり、
/// それ以外の tick は全フィールド `null`（⚠️ 0 で埋めない・issue 006 AC3）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Restart {
    /// 集計窓の中の挿入系 press の回数（ADR-030 D3 の写像）。
    pub post_t0_inserts: Option<u64>,
    /// 集計窓の中の削除系 press の回数。
    pub post_t0_deletes: Option<u64>,
    /// 集計窓の中の `navigation` カテゴリの press の回数（★ 「どちらでもない」側のうち移動だけ）。
    pub post_t0_navigations: Option<u64>,
    /// `s'_1 − o`（原点から最初の再停止まで）。打ち切りなら `null`。
    pub restop_ms: Option<u64>,
    /// `s'_1 − r_1`（最初の生成区間の長さ）。打ち切りなら `null`。
    pub stable_generation_ms: Option<u64>,
    /// `[r_1, s'_1)` の純産出（挿入系 − 削除系。★ `super::net_production` の共有ヘルパ）。
    /// ⚠️ `stable_generation_ms` は打鍵速度と交絡する（[V164]）ので**必ず対で読む**。
    pub stable_generation_net_presses: Option<i64>,
    /// 集計窓の中で始まった生成区間の本数。窓があって 0 本なら `0`。
    pub resumption_count: Option<u64>,
}

impl Restart {
    /// onset が立っていない tick の行（⚠️ **すべて `null`**・AC3）。
    fn null_row() -> Self {
        Self {
            post_t0_inserts: None,
            post_t0_deletes: None,
            post_t0_navigations: None,
            restop_ms: None,
            stable_generation_ms: None,
            stable_generation_net_presses: None,
            resumption_count: None,
        }
    }
}

/// press 列を種別ごとの累積和にしたもの（★ 区間の件数を `O(log N)` で引くため）。
struct PressPrefix {
    sorted_ts: Vec<u64>,
    inserts: Vec<u64>,
    deletes: Vec<u64>,
    navigations: Vec<u64>,
    net: Vec<i64>,
}

impl PressPrefix {
    fn new(keys: &[KeyPress]) -> Self {
        let mut sorted: Vec<KeyPress> = keys.to_vec();
        sorted.sort_by_key(|k| k.timestamp);

        let n = sorted.len();
        let mut sorted_ts = Vec::with_capacity(n);
        let mut inserts = Vec::with_capacity(n + 1);
        let mut deletes = Vec::with_capacity(n + 1);
        let mut navigations = Vec::with_capacity(n + 1);
        let mut net = Vec::with_capacity(n + 1);
        inserts.push(0);
        deletes.push(0);
        navigations.push(0);
        net.push(0);

        for k in &sorted {
            sorted_ts.push(k.timestamp);
            // ★ 挿入系 / 削除系の写像は ADR-030 D3 の SSOT (`super::key_net_delta`) をそのまま使う。
            let delta = super::key_net_delta(k);
            inserts.push(inserts.last().copied().unwrap_or(0) + u64::from(delta == 1));
            deletes.push(deletes.last().copied().unwrap_or(0) + u64::from(delta == -1));
            let is_navigation = KeyCategory::from_vk(k.vk) == KeyCategory::Navigation;
            navigations.push(navigations.last().copied().unwrap_or(0) + u64::from(is_navigation));
            net.push(net.last().copied().unwrap_or(0) + delta);
        }

        Self {
            sorted_ts,
            inserts,
            deletes,
            navigations,
            net,
        }
    }

    /// `[lo, hi)` に入る press の添字範囲。
    fn range(&self, lo: u64, hi_exclusive: u64) -> (usize, usize) {
        let start = self.sorted_ts.partition_point(|&t| t < lo);
        let end = self.sorted_ts.partition_point(|&t| t < hi_exclusive);
        (start, end.max(start))
    }
}

/// 1 つの観測区間の変化点に対する `O(log C)` の索引（`C` = 変化点の数）。
///
/// ⚠️★★★ **onset 行ごとに変化点を線形に走らないためにある。**
/// ★ 2026-09-20 のレビューで `code-reviewer`（H1）と `security-checker`（M1）が**独立に**
/// 「`restart` が `O(R・C)`（`R` = onset の数）になっている」と指摘した。
/// ⚠️ これは issue 008 が `stall_onset` 側で潰した二乗と**同型**であり、かつ
/// **AC18 / AC18b はどちらも `R = 1`** なので 20 秒の門がこの経路を検出しない
/// （★ 起票 21 に記録した）。
///
/// ★ `true_prefix` は単調非減少なので、「`from` 以上で最初に `value` を取る添字」も
/// 「`[from, to)` に `value` が何個あるか」も**二分探索と引き算**で引ける。
/// ⚠️ **「変化点の値が交互に並ぶ」という前提を使っていない** —— ★ 前提が破れても
/// 値がおかしくなるだけで、走査量は増えないし panic もしない
/// （`code-reviewer` L5: 交替が型で守られていないという指摘への手当て）。
struct ChangeIndex {
    /// `true_prefix[k]` = `changes[0..k]` に含まれる `true` の個数。
    true_prefix: Vec<usize>,
}

impl ChangeIndex {
    fn new(changes: &[(u64, bool)]) -> Self {
        let mut true_prefix = Vec::with_capacity(changes.len() + 1);
        let mut acc = 0usize;
        true_prefix.push(acc);
        for &(_, value) in changes {
            if value {
                acc += 1;
            }
            true_prefix.push(acc);
        }
        Self { true_prefix }
    }

    /// 変化点の数。
    fn len(&self) -> usize {
        self.true_prefix.len() - 1
    }

    /// `[from, to)` に含まれる値 `value` の変化点の個数（`O(1)`）。
    fn count(&self, from: usize, to: usize, value: bool) -> usize {
        let from = from.min(self.len());
        let to = to.clamp(from, self.len());
        let trues = self.true_prefix[to] - self.true_prefix[from];
        if value {
            trues
        } else {
            (to - from) - trues
        }
    }

    /// `from` 以上で最初に値 `value` を取る変化点の添字（`O(log C)`）。
    fn first_with(&self, from: usize, value: bool) -> Option<usize> {
        let len = self.len();
        if from >= len {
            return None;
        }
        // `count(from, k + 1, value) >= 1` は k について単調なので、最小の k を二分探索する。
        let mut lo = from;
        let mut hi = len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.count(from, mid + 1, value) >= 1 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        (lo < len && self.count(from, lo + 1, value) >= 1).then_some(lo)
    }
}

/// 1 つの onset 行ぶんの `restart` を計算する。
///
/// ⚠️ 引数に `s` を取らない —— ★ `s` は**呼び出し側が観測区間と次の onset を引く**のに使う。
/// ここへ渡すと「原点に `s` も効く」という誤読を招く（`code-reviewer` L6 / `security-checker` L3）。
fn restart_for_onset(
    t0_ms: u64,
    interval: &ConditionAInterval,
    index: &ChangeIndex,
    next_onset_s_ms: Option<u64>,
    config: &StallOnsetConfig,
    presses: &PressPrefix,
) -> Restart {
    let mut row = Restart::null_row();

    // 原点 o = t0 + B（⚠️ 値は決めない・AC4）。加算が溢れる入力では何も測れない。
    let Some(o) = t0_ms.checked_add(config.b_ms) else {
        return row;
    };
    if o > interval.b {
        // ⚠️ 原点が観測区間の右端を越えた = 窓も生成区間も無い（V5）。
        return row;
    }

    // ---- 集計窓（E2・V1） ----
    // ★ 右端 b は確定扱いなので、窓の排他的な右端は b + 1 とする（V2 と対称）。
    // ⚠️ 次の onset がある場合、窓は **`[o, 次の s)` の半開**である
    //   (2026-09-21・`/cross-review` 1 巡目・Codex 指摘 3 —— 仕様どおりだが明示する)。
    //   ★ `次の s` ちょうどの press は**この窓に入らない**。それは次の onset の
    //     *検出窓* `[s, t0)` に属する時刻であり、転帰として数えるべきものではない
    //     （ADR-030 D2「検出窓と転帰窓を重ねない」）。
    let window_end_exclusive = next_onset_s_ms.unwrap_or_else(|| interval.b.saturating_add(1));
    let window = (o < window_end_exclusive).then_some((o, window_end_exclusive));

    let changes = &interval.changes;
    let after_o = changes.partition_point(|&(t, _)| t <= o);
    // ⚠️ `o` 以前に評価時刻が 1 つも無い（＝ 条件 A の値が決まらない）。
    // ★ `o = t0 + B >= s >= a + W`（区間の最初の評価時刻）なので構造的には起きないが、
    //   境界入力で落ちないよう「情報が無い」側に倒す。
    let state_known = after_o > 0;
    // ★ V6: `o` の時点で条件 A が false なら、生成区間は既に `o` で開いている。
    let generating_at_o = state_known && !changes[after_o - 1].1;

    // ---- 最初の生成区間（V6） ----
    if state_known {
        let (r_1, search_from) = if generating_at_o {
            (Some(o), after_o)
        } else {
            match index.first_with(after_o, false) {
                Some(i) if changes[i].0 <= interval.b => (Some(changes[i].0), i + 1),
                _ => (None, after_o),
            }
        };
        if let Some(r_1) = r_1 {
            let s_prime_1 = index
                .first_with(search_from, true)
                .map(|i| changes[i].0)
                .filter(|&t| t <= interval.b);
            if let Some(s_prime_1) = s_prime_1 {
                row.restop_ms = Some(s_prime_1.saturating_sub(o));
                row.stable_generation_ms = Some(s_prime_1.saturating_sub(r_1));
                let (lo, hi) = presses.range(r_1, s_prime_1);
                row.stable_generation_net_presses = Some(presses.net[hi] - presses.net[lo]);
            }
            // ⚠️ `s'_1` が無い（右端まで閉じなかった）行は打ち切りなので `null` のまま（AC9）。
        }
    }

    // ---- 窓に依存する量（V5: 窓が無ければ null） ----
    if let Some((lo, hi_exclusive)) = window {
        let (start, end) = presses.range(lo, hi_exclusive);
        row.post_t0_inserts = Some(presses.inserts[end] - presses.inserts[start]);
        row.post_t0_deletes = Some(presses.deletes[end] - presses.deletes[start]);
        row.post_t0_navigations = Some(presses.navigations[end] - presses.navigations[start]);
        // ★ 窓の中で始まった生成区間の本数。
        //   ⚠️★★★ **V6 でクリップされた区間は `o` で始まったものとして 1 に数える**
        //   （`code-reviewer` M4）—— ★ したがって「`o` より前に再開済みで窓の中では 1 度も
        //   再開していない」行と「窓の中でちょうど 1 回再開した」行は**同じ 1 になる**。
        //   ★ 両者は `stable_generation_ms == restop_ms`（前者）で区別できる。
        let started_in_window = if state_known {
            let hi_idx = changes.partition_point(|&(t, _)| t < hi_exclusive);
            index.count(after_o, hi_idx, false) + usize::from(generating_at_o)
        } else {
            0
        };
        row.resumption_count = Some(started_in_window as u64);
    }

    row
}

/// 1 Hz tick 単位の `restart` 列を作る。`super::build_feature_rows` から呼ばれる。
///
/// ⚠️★★★ **入力は `stall_onset` の *列* である**（判断 4）—— `stall_onset_column` は
/// グリッド外の `t0` と同じ tick に落ちた 2 つ目の onset を捨てるので、生の onset 列から
/// 独立に作ると**行が対応しなくなる**（★ AC3「`t0` が立っていない行は全部 `null`」が壊れる）。
/// ★ 一方、集計窓の右端に使う「次の onset の `s`」は **`timeline` 側の（捨てられた分も含む）
/// onset 列**から取る —— 窓の右端は「次に onset が立った事実」で決まるからである。
pub(super) fn restart_column(
    stall_onset_col: &[StallOnset],
    timeline: &[ConditionAInterval],
    keys: &[KeyPress],
    config: &StallOnsetConfig,
) -> Vec<Restart> {
    if stall_onset_col.iter().all(|row| !row.is_onset) {
        return stall_onset_col
            .iter()
            .map(|_| Restart::null_row())
            .collect();
    }
    let presses = PressPrefix::new(keys);
    // ★ 変化点の索引は**観測区間ごとに 1 回だけ**作る（⚠️ onset 行ごとに作らない）。
    let indexes: Vec<ChangeIndex> = timeline
        .iter()
        .map(|interval| ChangeIndex::new(&interval.changes))
        .collect();

    stall_onset_col
        .iter()
        .map(|row| {
            let (Some(s_ms), Some(t0_ms)) = (row.s_ms, row.t0_ms) else {
                return Restart::null_row();
            };
            // `s` を含み `t0` が右端に収まる観測区間を引く（★ 区間は時刻順・互いに素）。
            let idx = timeline.partition_point(|interval| interval.a <= s_ms);
            let Some(i) = idx.checked_sub(1) else {
                return Restart::null_row();
            };
            let interval = &timeline[i];
            // ⚠️ `s` が区間の**穴**に落ちている行を、手前の区間に付けない
            //   (2026-09-21・`/cross-review` 1 巡目・Codex 指摘 2)。
            //   ★ `t0 = s + θ` かつ `θ >= 0` なので `s > b` なら `t0 > b` でもあり、
            //     下の検査だけでも現状は捕まる。⚠️ **その含意に依存させない**ために明示する。
            if s_ms > interval.b || t0_ms > interval.b {
                return Restart::null_row();
            }
            // ★ onset は区間内で `s` 昇順なので二分探索で引く（⚠️ 線形 find だと `O(R²)`）。
            let next_onset_s_ms = interval
                .onsets
                .get(interval.onsets.partition_point(|onset| onset.s_ms <= s_ms))
                .map(|onset| onset.s_ms);
            restart_for_onset(
                t0_ms,
                interval,
                &indexes[i],
                next_onset_s_ms,
                config,
                &presses,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::loop_step::ENGINE_TICK_MS;

    fn key(t_ms: u64, vk: u32) -> KeyPress {
        KeyPress {
            timestamp: t_ms,
            vk,
        }
    }

    /// 文字キー（挿入系）。
    fn letter(t_ms: u64) -> KeyPress {
        key(t_ms, 0x41)
    }

    fn onset_row(s_ms: u64, t0_ms: u64) -> StallOnset {
        StallOnset {
            is_onset: true,
            s_ms: Some(s_ms),
            t0_ms: Some(t0_ms),
            theta_person_ms: Some(t0_ms - s_ms),
            theta_sample_n: Some(1),
            task_kind: None,
            phase: "unspecified".to_string(),
            ime_overlap: Some(false),
        }
    }

    fn plain_row() -> StallOnset {
        StallOnset {
            is_onset: false,
            s_ms: None,
            t0_ms: None,
            theta_person_ms: None,
            theta_sample_n: None,
            task_kind: None,
            phase: "unspecified".to_string(),
            ime_overlap: None,
        }
    }

    fn interval(a: u64, b: u64, changes: Vec<(u64, bool)>) -> ConditionAInterval {
        ConditionAInterval {
            a,
            b,
            changes,
            onsets: Vec::new(),
        }
    }

    /// AC3: onset が立っていない行は**全フィールド `null`**（⚠️ 0 で埋めない）。
    #[test]
    fn no_onset_yields_null_not_zero() {
        let column = vec![plain_row(), plain_row()];
        let timeline = vec![interval(0, 100_000, vec![(0, false)])];
        let config = StallOnsetConfig::default();
        let rows = restart_column(&column, &timeline, &[letter(10)], &config);

        assert_eq!(rows.len(), 2);
        for row in &rows {
            let json = serde_json::to_value(row).expect("serialize");
            for (name, value) in json.as_object().expect("object") {
                assert!(value.is_null(), "{name} は null のはず: {value}");
            }
        }
    }

    /// AC2: 原点が **`t0`**（＋ buffer `B`）であって、単純 pause ではないこと。
    ///
    /// ★ 同じ条件 A の辺の列に対し、`t0` を後ろへずらすと `restop_ms` が**その差だけ縮む**。
    /// ⚠️ 単純 pause（＝ `s` の側）に原点を置いていたら、`t0` を動かしても値は変わらない。
    #[test]
    fn restart_is_anchored_at_t0_not_pause() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        // 条件 A: 0 で true（停滞）→ 50_000 で false（生成）→ 80_000 で true（再停止）
        let changes = vec![(0, true), (50_000, false), (80_000, true)];
        let timeline = vec![interval(0, 200_000, changes)];

        let early = restart_column(&[onset_row(5_000, 10_000)], &timeline, &[], &config);
        let late = restart_column(&[onset_row(5_000, 20_000)], &timeline, &[], &config);

        assert_eq!(early[0].restop_ms, Some(80_000 - 10_000));
        assert_eq!(late[0].restop_ms, Some(80_000 - 20_000));
        assert_ne!(
            early[0].restop_ms, late[0].restop_ms,
            "`s` が同じでも `t0` が違えば値が変わる（★ 原点は t0）"
        );
        // `r_1` は辺そのもの（50_000）なので、安定生成の長さは t0 に依らない。
        assert_eq!(early[0].stable_generation_ms, Some(30_000));
        assert_eq!(late[0].stable_generation_ms, Some(30_000));
    }

    /// AC4: buffer `B` を `StallOnsetConfig` から引いていること（★ 値を決めていない）。
    #[test]
    fn buffer_b_comes_from_the_stall_onset_config() {
        let changes = vec![(0, true), (50_000, false), (80_000, true)];
        let timeline = vec![interval(0, 200_000, changes)];
        let column = vec![onset_row(5_000, 10_000)];

        let without_buffer = restart_column(
            &column,
            &timeline,
            &[],
            &StallOnsetConfig {
                b_ms: 0,
                ..StallOnsetConfig::default()
            },
        );
        let with_buffer = restart_column(
            &column,
            &timeline,
            &[],
            &StallOnsetConfig {
                b_ms: 5_000,
                ..StallOnsetConfig::default()
            },
        );
        assert_eq!(without_buffer[0].restop_ms, Some(70_000));
        assert_eq!(
            with_buffer[0].restop_ms,
            Some(65_000),
            "B を足すと原点が後ろへ動く"
        );
    }

    /// ★ V6 の裁定: 原点の時点で条件 A が既に `false`（もう生成している）なら、
    /// 生成区間は `o` で開いていると読む（⚠️ `null` にしない）。
    #[test]
    fn generation_already_open_at_origin_clips_r1_to_origin() {
        let config = StallOnsetConfig {
            b_ms: 30_000,
            ..StallOnsetConfig::default()
        };
        // 条件 A: 0 で true → 20_000 で false（★ 原点 40_000 より前に再開している）→ 90_000 で true
        let changes = vec![(0, true), (20_000, false), (90_000, true)];
        let timeline = vec![interval(0, 200_000, changes)];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &timeline, &[], &config);

        let o = 10_000 + 30_000;
        assert_eq!(rows[0].restop_ms, Some(90_000 - o));
        assert_eq!(
            rows[0].stable_generation_ms,
            Some(90_000 - o),
            "r_1 は o にクリップされる（★ buffer の内側へ遡らない）"
        );
        assert_eq!(rows[0].resumption_count, Some(1));
    }

    /// AC9: onset はあるが `s'_1` が観測区間の中に無い（打ち切り）行では `null`（⚠️ 0 で埋めない）。
    #[test]
    fn censored_restop_yields_null_not_zero() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        // 50_000 で生成が始まるが、観測区間の右端 (60_000) までに立ち上がらない。
        let changes = vec![(0, true), (50_000, false)];
        let timeline = vec![interval(0, 60_000, changes)];
        let rows = restart_column(
            &[onset_row(5_000, 10_000)],
            &timeline,
            &[letter(55_000)],
            &config,
        );

        assert_eq!(rows[0].restop_ms, None, "⚠️ 0 で埋めない");
        assert_eq!(rows[0].stable_generation_ms, None);
        assert_eq!(rows[0].stable_generation_net_presses, None);
        // ★ 窓はあるので、窓に依存する量は値を持つ（打ち切りとは別の話）。
        assert_eq!(rows[0].resumption_count, Some(1));
        assert_eq!(rows[0].post_t0_inserts, Some(1));

        // ★ 再開が 1 度も無いまま右端に達した行も同じく null（★ ただし resumption_count は 0）。
        let never = vec![(0, true)];
        let rows = restart_column(
            &[onset_row(5_000, 10_000)],
            &[interval(0, 60_000, never)],
            &[],
            &config,
        );
        assert_eq!(rows[0].restop_ms, None);
        assert_eq!(
            rows[0].resumption_count,
            Some(0),
            "窓はあるので 0（null ではない）"
        );
    }

    /// V5 の裁定: 集計窓そのものが無い行では `post_t0_*` / `resumption_count` が `null`。
    #[test]
    fn missing_window_yields_null_counts() {
        let config = StallOnsetConfig {
            b_ms: 30_000,
            ..StallOnsetConfig::default()
        };
        // 原点 o = 10_000 + 30_000 = 40_000 が観測区間の右端 (30_000) を越える。
        let timeline = vec![interval(0, 30_000, vec![(0, true)])];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &timeline, &[], &config);
        assert_eq!(rows[0].post_t0_inserts, None, "⚠️ 0 で埋めない");
        assert_eq!(rows[0].resumption_count, None);

        // E2 で次の onset の `s` が原点以前にある場合も窓が無い。
        let mut crowded = interval(0, 200_000, vec![(0, true), (20_000, false), (35_000, true)]);
        crowded.onsets = vec![super::super::stall_onset::OnsetEvent {
            s_ms: 35_000,
            t0_ms: 38_000,
            theta_person_ms: 3_000,
            theta_sample_n: 1,
        }];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &[crowded], &[], &config);
        assert_eq!(rows[0].post_t0_inserts, None);
        assert_eq!(rows[0].resumption_count, None);
    }

    /// 窓があって press が 0 件なら `0`（⚠️ `null` ではない）。
    #[test]
    fn empty_window_counts_zero_not_null() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        let timeline = vec![interval(0, 60_000, vec![(0, true)])];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &timeline, &[], &config);
        assert_eq!(rows[0].post_t0_inserts, Some(0));
        assert_eq!(rows[0].post_t0_deletes, Some(0));
        assert_eq!(rows[0].post_t0_navigations, Some(0));
    }

    /// `post_t0_*` が ADR-030 D3 の写像で数えられていること（`navigation` は移動だけ）。
    #[test]
    fn post_t0_counts_use_the_adr_030_d3_mapping() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        let timeline = vec![interval(0, 60_000, vec![(0, true)])];
        let keys = vec![
            letter(11_000),
            key(12_000, 0x08), // backspace → 削除系
            key(13_000, 0x25), // ← 矢印 → navigation
            key(14_000, 0xA0), // 左 Shift → modifier（★ どれにも数えない）
            letter(9_000),     // ⚠️ 原点より前 → 窓の外
        ];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &timeline, &keys, &config);
        assert_eq!(rows[0].post_t0_inserts, Some(1));
        assert_eq!(rows[0].post_t0_deletes, Some(1));
        assert_eq!(rows[0].post_t0_navigations, Some(1));
    }

    /// `stable_generation_net_presses` が `[r_1, s'_1)` の純産出であること。
    #[test]
    fn stable_generation_net_presses_counts_the_generation_span() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        let changes = vec![(0, true), (20_000, false), (30_000, true)];
        let timeline = vec![interval(0, 60_000, changes)];
        let keys = vec![
            letter(19_000), // ⚠️ r_1 より前
            letter(21_000),
            letter(22_000),
            key(23_000, 0x08), // 削除系 → −1
            letter(31_000),    // ⚠️ s'_1 以降
        ];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &timeline, &keys, &config);
        assert_eq!(rows[0].stable_generation_net_presses, Some(1));
    }

    /// AC7: `restart` が **detection 側へ渡っていない**こと。
    ///
    /// ★ 配線の向きを実測で固定する —— `build_feature_rows` が返す `stall_onset` 列は、
    /// `restart` を一切知らない `stall_onset_column` の出力と**完全に一致する**。
    #[test]
    fn restart_is_not_fed_into_detection() {
        let epoch = 1_700_000_000_000_u64;
        let keys: Vec<KeyPress> = (0..5).map(|i| letter(epoch + i * 100)).collect();
        let feat_ts: Vec<u64> = (0..90).map(|i| epoch + i * ENGINE_TICK_MS).collect();
        let config = StallOnsetConfig::default();

        let rows = super::super::build_feature_rows(
            &keys,
            &[],
            &[],
            10_000,
            &super::super::StallOnsetInputs {
                feat_ts: &feat_ts,
                task_kind_declarations: &[],
                config,
            },
            &super::super::QualityInputs {
                capabilities_declared: false,
                hook_health: &[],
            },
        )
        .expect("within tick limit");

        let detection_only = super::super::stall_onset::stall_onset_column(
            &keys,
            &feat_ts,
            &[],
            rows.len(),
            rows[0].t_ms,
            ENGINE_TICK_MS,
            &config,
        );
        assert_eq!(detection_only.len(), rows.len());
        for (row, expected) in rows.iter().zip(detection_only.iter()) {
            assert_eq!(row.stall_onset.is_onset, expected.is_onset);
            assert_eq!(row.stall_onset.s_ms, expected.s_ms);
            assert_eq!(row.stall_onset.t0_ms, expected.t0_ms);
            assert_eq!(row.stall_onset.theta_person_ms, expected.theta_person_ms);
        }
        assert!(
            rows.iter().any(|row| row.stall_onset.is_onset),
            "この入力では onset が立つはず（★ 空振りの比較にしない）"
        );
    }

    /// ★ onset が多数立つ入力でも、各行の値が**素朴な走査と一致する**こと。
    ///
    /// ⚠️★★★ 2026-09-20 のレビュー（`code-reviewer` H1 / `security-checker` M1）で
    /// `O(R・C)` を潰して二分探索に置き換えたので、**置き換えの正しさをここで固定する**。
    /// ★ `AC18` / `AC18b` は `R = 1` なのでこの形を 1 度も通っていない（起票 21）。
    #[test]
    fn many_onsets_match_a_naive_scan() {
        let config = StallOnsetConfig {
            b_ms: 1_000,
            ..StallOnsetConfig::default()
        };
        // 条件 A が 10_000ms 周期で true / false を繰り返し、onset が 60 本立つ観測区間。
        let period = 10_000u64;
        let count = 60u64;
        let mut changes = Vec::new();
        let mut onsets = Vec::new();
        for k in 0..count {
            let rise = k * period;
            changes.push((rise, true));
            changes.push((rise + period / 2, false));
            onsets.push(crate::analysis::feat_offline::stall_onset::OnsetEvent {
                s_ms: rise,
                t0_ms: rise + 100,
                theta_person_ms: 100,
                theta_sample_n: 1,
            });
        }
        let b = count * period;
        let mut interval = interval(0, b, changes.clone());
        interval.onsets = onsets.clone();
        let column: Vec<StallOnset> = onsets
            .iter()
            .map(|onset| onset_row(onset.s_ms, onset.t0_ms))
            .collect();
        let rows = restart_column(&column, &[interval], &[], &config);

        assert_eq!(rows.len(), count as usize);
        for (k, row) in rows.iter().enumerate() {
            let k = k as u64;
            let o = k * period + 100 + config.b_ms;
            // 素朴な走査: `o` 以後で最初に false へ落ち、その後で最初に true へ戻る時刻。
            let naive_r = changes
                .iter()
                .find(|&&(t, v)| t > o && !v)
                .map(|&(t, _)| t)
                .filter(|&t| t <= b);
            let naive_s_prime = naive_r.and_then(|r| {
                changes
                    .iter()
                    .find(|&&(t, v)| t > r && v)
                    .map(|&(t, _)| t)
                    .filter(|&t| t <= b)
            });
            // `o` の時点では条件 A は true（rise の直後）なので、V6 のクリップは起きない。
            assert_eq!(
                row.restop_ms,
                naive_s_prime.map(|s| s - o),
                "k = {k} の restop_ms が素朴な走査と食い違う"
            );
            assert_eq!(
                row.stable_generation_ms,
                naive_s_prime.zip(naive_r).map(|(s, r)| s - r),
                "k = {k} の stable_generation_ms が素朴な走査と食い違う"
            );
        }
        assert!(
            rows.iter().any(|row| row.restop_ms.is_some()),
            "空振りの比較にしない（★ 少なくとも 1 行は値を持つ）"
        );
    }

    /// ⚠️★★★ **`restop_ms` の `null` は、onset のある行では*必ず*右側打ち切りを意味する。**
    ///
    /// ★ 判断 2 の **V4**（`censored` フラグを足さない）は、この一意性に**依存している** ——
    /// 「打ち切りか否か」は `t0_ms != null && restop_ms == null` で復元できる、という復元規則が
    /// `mod.rs` の `FeatureRow.restart` doc に書いてある。
    /// ⚠️ 将来 `null` に別の意味（「再開が無い」「窓が無い」等の*区別*）が混ざると、**復元規則が壊れる**
    /// （2026-09-21・`/cross-review` 2 巡目・Codex 指摘 3）。
    /// → ★ `null` になる 3 通りの経路をここで**列挙して固定する**。⚠️ 4 つ目が生えたらこのテストを直すこと。
    #[test]
    fn restop_null_always_means_right_censoring() {
        let config = StallOnsetConfig {
            b_ms: 1_000,
            ..StallOnsetConfig::default()
        };
        let onset = [onset_row(5_000, 10_000)];

        // (1) 窓が無い（原点が観測区間の右端を越える）
        let no_window = restart_column(
            &onset,
            &[interval(0, 10_500, vec![(0, true)])],
            &[],
            &config,
        );
        assert_eq!(no_window[0].restop_ms, None);
        assert_eq!(
            no_window[0].resumption_count, None,
            "窓が無いので null（V5）"
        );

        // (2) 窓はあるが `r_1` が観測区間の中に無い（1 度も再開しないまま右端に達した）
        let never_resumed = restart_column(
            &onset,
            &[interval(0, 60_000, vec![(0, true)])],
            &[],
            &config,
        );
        assert_eq!(never_resumed[0].restop_ms, None);
        assert_eq!(never_resumed[0].resumption_count, Some(0), "窓はあるので 0");

        // (3) `r_1` はあるが `s'_1` が観測区間の中に無い（生成が続いたまま右端に達した）
        let censored = restart_column(
            &onset,
            &[interval(0, 60_000, vec![(0, true), (20_000, false)])],
            &[],
            &config,
        );
        assert_eq!(censored[0].restop_ms, None);
        assert_eq!(censored[0].resumption_count, Some(1));

        // ★ 3 通りとも「観測区間の右端まで再停止を観測できなかった」＝ 右側打ち切りである。
        //   ⚠️ onset のある行で `restop_ms` が `null` になる経路は、この 3 つしかない。
        for row in [&no_window[0], &never_resumed[0], &censored[0]] {
            assert_eq!(row.stable_generation_ms, None);
            assert_eq!(row.stable_generation_net_presses, None);
        }
    }

    /// ★ 観測区間の同定と窓の右端の境界（2026-09-21・`/cross-review` 1 巡目・Codex 指摘 2 / 3 / 4）。
    ///
    /// ⚠️ 観測区間は**穴で切れている**ので、`s` が穴に落ちた onset を手前の区間に付けてはならない。
    /// ★ 窓の右端は `次の onset の s` の**半開**であり、同時刻の press はこの窓に入らない。
    #[test]
    fn interval_lookup_and_window_edges_are_exact() {
        let config = StallOnsetConfig {
            b_ms: 0,
            ..StallOnsetConfig::default()
        };
        // 区間は 2 つ: [0, 10_000] と [50_000, 90_000]（★ 間の 10_000〜50_000 は穴）
        let first = interval(0, 10_000, vec![(0, true), (5_000, false), (8_000, true)]);
        let second = interval(
            50_000,
            90_000,
            vec![(50_000, true), (60_000, false), (70_000, true)],
        );
        let timeline = vec![first, second];

        // (a) `s` が穴の中 → 手前の区間に付けず null
        let in_hole = restart_column(&[onset_row(20_000, 20_500)], &timeline, &[], &config);
        assert_eq!(in_hole[0].restop_ms, None);
        assert_eq!(in_hole[0].resumption_count, None, "窓も無い（V5）");

        // (b) `s` が区間の左端ちょうど → その区間で評価される
        let at_a = restart_column(&[onset_row(50_000, 50_000)], &timeline, &[], &config);
        assert_eq!(at_a[0].restop_ms, Some(70_000 - 50_000));

        // (c) `s` が区間の右端ちょうど → 区間内として扱う（★ `t0 <= b` と対称）
        let at_b = restart_column(&[onset_row(10_000, 10_000)], &timeline, &[], &config);
        assert_eq!(
            at_b[0].restop_ms, None,
            "右端では再停止が観測できない（打ち切り）"
        );
        assert_eq!(at_b[0].resumption_count, Some(0), "窓はある");

        // (d) 窓の右端は次の onset の `s` の半開。同時刻の press は入らない。
        let mut crowded = interval(
            0,
            90_000,
            vec![(0, true), (5_000, false), (8_000, true), (40_000, false)],
        );
        crowded.onsets = vec![
            crate::analysis::feat_offline::stall_onset::OnsetEvent {
                s_ms: 1_000,
                t0_ms: 2_000,
                theta_person_ms: 1_000,
                theta_sample_n: 1,
            },
            crate::analysis::feat_offline::stall_onset::OnsetEvent {
                s_ms: 30_000,
                t0_ms: 31_000,
                theta_person_ms: 1_000,
                theta_sample_n: 1,
            },
        ];
        let keys = vec![letter(29_999), letter(30_000)];
        let rows = restart_column(&[onset_row(1_000, 2_000)], &[crowded], &keys, &config);
        assert_eq!(
            rows[0].post_t0_inserts,
            Some(1),
            "次の onset の s (30_000) ちょうどの press は入らない"
        );
    }

    /// 境界入力で panic しないこと（`t0 + B` の溢れ / 観測区間が引けない / 空入力）。
    #[test]
    fn restart_column_handles_degenerate_inputs() {
        let config = StallOnsetConfig::default();
        assert!(restart_column(&[], &[], &[], &config).is_empty());

        // `t0 + B` が u64 を溢れる
        let timeline = vec![interval(0, u64::MAX, vec![(0, true)])];
        let rows = restart_column(&[onset_row(1, u64::MAX)], &timeline, &[], &config);
        assert_eq!(rows[0].restop_ms, None);

        // onset の `s` を含む観測区間が timeline に無い
        let rows = restart_column(&[onset_row(5_000, 10_000)], &[], &[], &config);
        assert_eq!(rows[0].resumption_count, None);

        // 変化点が `o` 以前に 1 つも無い（構造的には起きないが落ちないこと）
        let late = vec![interval(0, 200_000, vec![(150_000, true)])];
        let rows = restart_column(&[onset_row(5_000, 10_000)], &late, &[], &config);
        assert_eq!(rows[0].restop_ms, None);
    }
}
