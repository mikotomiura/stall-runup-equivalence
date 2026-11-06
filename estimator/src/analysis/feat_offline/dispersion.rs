//! `T1a` (分布・個人相対) — pause 長の分布統計 (p50/p90/max) と IKI の CV、
//! およびセッション内 baseline からの逸脱 (surprise) (ADR-030 D10)。
//!
//! ⚠️★★★ **pause の長さを「短い/中/長い」の区分に分けて意味を割り当てることはしない** —
//! [P121] は事前宣言した 4 区分すべてが成績と負相関を示し、弁別は長さではなく
//! 直前イベントにあると結論している (それは `T1b` = `boundary` の担当)。
//! ここで作るのは **分布の要約値そのもの**であって、区分ごとの解釈ではない。
//!
//! pause の判定そのものは `super::extract_pause_resumptions` (共有ヘルパ、
//! `features.rs::PAUSE_THRESHOLD_MS` が SSOT) にそのまま委ねる。本モジュールは
//! 対になる直前 press との差分から長さ (`gap_ms`) を添えるだけで、独自の pause
//! 判定や新しい数値定数は 1 つも持たない (Codex TASK-PRE 指摘 5)。
//!
//! # surprise の baseline は「セッション内」である (★ 研究者採択 2026-09-14)
//!
//! `decisions.md` 判断 3 のとおり、baseline は**このセッション内の IKI 分布**から作る。
//! ⚠️ `ADR-030 D4` の `θ_person` (個人 × `task_kind` の分位点) とは別物 — D4 は
//! `t0` 検出器の閾値の話であって、`T1a` の baseline の話ではない。混同しないこと。
//!
//! # `pause_*` の集約窓 (issue 003・判断 4)
//!
//! `pause_p50_ms` / `pause_p90_ms` / `pause_max_ms` は tick ではなく **`t1a_window_ms` の因果窓**で
//! 集約する ([`pause_ms_by_window`])。⚠️ **`iki_cv` / `surprise` には窓を掛けない** (前者は tick
//! ローカル・後者はセッション相対のまま)。★ 窓は**観測区間 (`feat` 行の穴) をまたがない**。
//! ⚠️ **`feat` 行が 1 本も無い入力では `pause_*` は全行 `null`** である (観測区間が確定しないので値で埋めない)。

/// 1 tick ぶんの `T1a` (分布・個人相対) 値。
///
/// # `pause_*` は **`t1a_window_ms` の因果窓**で集約した値である (issue 003・判断 4)
///
/// tick `k` の `pause_p50_ms` / `pause_p90_ms` / `pause_max_ms` は、[`pause_ms_by_window`] が作る窓
/// `[tick_end − t1a_window_ms, tick_end)` に**再開する** pause の長さの分位点である。
/// ⚠️ 窓の右端は tick の**終端**。**値そのものは因果的**（未来を見ない）だが、**穴による切断は
/// retrospective** である —— `observed_intervals` が決める区間の右端は、その tick より後にある
/// `feat` 行を最大 `hole_gap_ms` 程度先読みしてから確定する (`super` モジュール doc の
/// 「causal / retrospective の区分」と同じ)。★ 窓長が `ENGINE_TICK_MS` なら旧来の
/// 「その tick に帰属する pause だけ」と一致する（穴が無い限り）。
///
/// ⚠️★★★ **窓が 1 tick だと 3 値は常に同値になる** (2026-09-15・`code-reviewer` HIGH-1・
/// **構造的な帰結であってバグではない**。旧実装はこの状態で出荷されていた):
///
/// - `features::PAUSE_THRESHOLD_MS` (= 2000ms) > `loop_step::ENGINE_TICK_MS` (= 1000ms)。
/// - pause は**再開側の press が属する tick** に帰属する (`super` のモジュール doc)。
/// - 同じ tick に終わる pause が 2 本あるとすると、再開時刻 `t1 < t2` は同一 tick なので
///   `t2 - t1 < 1000`。ところが `t2` で終わる pause の開始は `t1` 以降の press なので
///   `gap(t2) <= t2 - t1 < 1000 < 2000` となり、**`gap(t2) >= PAUSE_THRESHOLD_MS` に矛盾する**。
/// - → ★★★ **1 tick に pause は高々 1 本。要素数が 1 なので 3 分位はすべて同じ値になる。**
///
/// ★ 実測 (2026-09-15・`tests/fixtures/` の **14 本すべて**): `activity.pauses > 1` の tick は
/// **0 件**、`p50 != max` の行も **0 件**だった。
///
/// → ★ **窓を広げて解いた**。窓長は事前宣言 (pre-registration) 事項なので値は
/// `src-tauri/prereg/p1-episode.json` の `t1a_window_ms` が SSOT である
/// (★ 判断 4・起票 7 の選択肢 (a))。⚠️ **`--set t1a_window_ms=<ENGINE_TICK_MS>` で窓を 1 tick に
/// 戻すとこの退化が再現する** —— これが `003-acceptance.sh` AC4 の陽性対照である。
///
/// ⚠️★★★ **窓は観測区間 (`feat` 行の穴) をまたがない** (★ 起票 6): 削除区間・一時停止・計器欠損の
/// 前後にある pause は同じ窓に入らず、**穴をまたぐ 1 本の巨大 pause も窓に入らない**。
/// ⚠️ 原因ラベルは作らない (ADR-030 却下リスト 9) —— 使うのは `feat` 行の**存在**だけである。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Dispersion {
    /// 窓に再開する pause 長 (ms) の中央値相当 (nearest-rank 法)。窓に pause が 0 本なら `null`。
    /// ⚠️ `feat` 行が 1 本も無い入力では**全行 `null`** (観測区間が確定しない・[`pause_ms_by_window`])。
    pub pause_p50_ms: Option<f64>,
    /// 同じく 90 パーセンタイル相当。
    pub pause_p90_ms: Option<f64>,
    /// 同じく最大値。⚠️ 窓が 1 tick だと 3 値は必ず同値になる (上の struct doc)。
    pub pause_max_ms: Option<f64>,
    /// この tick に帰属する IKI (打鍵間隔) の変動係数 (std / mean)。要素 2 個未満、
    /// または mean が 0 (同時刻打鍵のみ) なら `null`。
    pub iki_cv: Option<f64>,
    /// セッション全体の IKI baseline からのこの tick の逸脱 (z 相当: `(tick_mean - baseline_mean) / baseline_std`)。
    /// baseline が定義できない (セッション全体で IKI が 2 個未満、または分散 0) か、
    /// この tick に IKI が 1 件も無ければ `null`。
    pub surprise: Option<f64>,
}

/// セッション全体の IKI 分布から取る baseline (mean / std)。1 セッションにつき 1 回だけ
/// 計算し、各 tick の [`compute_dispersion`] に引数として渡す (AC5: シグネチャが
/// セッション全体の統計を引数に取る = tick ローカルには絶対に作り直さない)。
///
/// ⚠️★★★ **`Serialize` を持たないが、「出力に出ていない」とは読まないこと**
/// (2026-09-15・`security-checker` L1・実測で確認):
/// `surprise = (tick_mean - mean_ms) / std_ms` の `tick_mean` は**同じ行の
/// `activity.mean_iki_ms` として出ている**ので、**2 行あれば `mean_ms` / `std_ms` が連立で解ける**。
/// ★ 実測 (`instrument_rich.ndjson`): 復元値が生 NDJSON からの直接計算と**小数 10 桁まで一致**した。
///
/// ⚠️ **新しい露出ではない** —— ★ 提出される匿名化 NDJSON の `key` 行は
/// `(category, 相対 ms)` を**ミリ秒分解能のまま**持つ (`anonymize.rs` は `t` を相対化するだけで
/// 分解能を落とさない) ので、同じ値はそこからも厳密に再計算できる。
/// → ★ ただし **「baseline は出していない」を前提に判断すると誤る**ので、ここに記録しておく。
#[derive(Debug, Clone, Copy)]
pub struct SessionIkiBaseline {
    mean_ms: f64,
    std_ms: f64,
}

impl SessionIkiBaseline {
    /// セッション全体 (全 tick 分をまとめた) の IKI 値から baseline を作る。
    /// 2 個未満、または分散が 0 (全値が同一) だと z 相当の分母が定義できないため `None`。
    pub fn from_session_iki(all_iki_ms: &[u64]) -> Option<Self> {
        if all_iki_ms.len() < 2 {
            return None;
        }
        let mean_ms = mean(all_iki_ms)?;
        let variance = all_iki_ms
            .iter()
            .map(|&v| {
                let d = v as f64 - mean_ms;
                d * d
            })
            .sum::<f64>()
            / all_iki_ms.len() as f64;
        let std_ms = variance.sqrt();
        if std_ms == 0.0 {
            return None;
        }
        Some(Self { mean_ms, std_ms })
    }
}

fn mean(values: &[u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().map(|&v| v as f64).sum::<f64>() / values.len() as f64)
}

/// nearest-rank 法 (線形補間なし) で分位点を取る。`sorted_ascending` は昇順ソート済みを
/// 仮定する。要素 0 個なら `None` (**呼び出し側は 0 で埋めない**)。
/// `rank = ceil(q * n)` (1-indexed, `[1, n]` にクランプ) — 標準的な nearest-rank 定義。
///
/// ⚠️ **線形補間の median とは一致しない** (2026-09-15・`code-reviewer` LOW)。
/// ★ 偶数個では**下側**を返す (`[10, 20]` → `10.0`。一般的な median は `15.0`)。
/// → ⚠️ 外部ツール・論文値と突き合わせるときは定義差を承知すること。
fn quantile(sorted_ascending: &[u64], q: f64) -> Option<f64> {
    let n = sorted_ascending.len();
    if n == 0 {
        return None;
    }
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    Some(sorted_ascending[idx] as f64)
}

/// IKI 値の変動係数 (std / mean)。2 個未満、または mean が 0 (同時刻打鍵のみ) なら `None`。
///
/// ⚠️ **標準偏差は母集団版 (÷ n) である** (2026-09-15・`code-reviewer` MEDIUM で明文化)。
/// ★ 標本版 (÷ (n-1)) ではない。⚠️ tick 内の n は 2〜3 が支配的なので**差は小さくない**。
/// ★ `SessionIkiBaseline::from_session_iki` も同じ ÷ n を使う (流儀を揃えてある)。
fn coefficient_of_variation(values: &[u64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let m = mean(values)?;
    if m == 0.0 {
        return None;
    }
    let variance = values
        .iter()
        .map(|&v| {
            let d = v as f64 - m;
            d * d
        })
        .sum::<f64>()
        / values.len() as f64;
    Some(variance.sqrt() / m)
}

/// pause 1 件ぶんの長さ。`resuming_ts` は帰属先 tick の判定に使う
/// (`super::build_feature_rows` の「pause は再開側の tick に帰属する」規則と対称)。
#[derive(Debug, Clone, Copy)]
pub struct PauseDuration {
    pub resuming_ts: u64,
    pub gap_ms: u64,
}

impl PauseDuration {
    /// この pause の開始 (= 再開する press の直前の press) の時刻。
    fn start_ts(&self) -> u64 {
        self.resuming_ts.saturating_sub(self.gap_ms)
    }
}

/// press 時刻列から pause (長さ付き) の列を作る。pause の判定は `super::extract_pause_resumptions`
/// (共有ヘルパ) にそのまま委ね、本関数は対になる直前 press との差分から長さを添えるだけ
/// (`boundary::pause_boundary_pairs` と対称の手法。判定を再実装しない)。
pub fn pause_durations(press_ts: &[u64]) -> Vec<PauseDuration> {
    let mut sorted: Vec<u64> = press_ts.to_vec();
    sorted.sort_unstable();
    let resumptions = super::extract_pause_resumptions(&sorted);

    let mut pending = resumptions.into_iter();
    let mut next_expected = pending.next();
    let mut out = Vec::new();
    for i in 1..sorted.len() {
        let Some(expected) = next_expected else {
            break;
        };
        if sorted[i] == expected {
            out.push(PauseDuration {
                resuming_ts: sorted[i],
                gap_ms: sorted[i].saturating_sub(sorted[i - 1]),
            });
            next_expected = pending.next();
        }
    }
    out
}

/// tick ごとの `T1a` の pause 長の集合を、**因果的なローリング窓**で作る (issue 003・判断 4)。
///
/// tick `k` (開始 `t_k = grid_origin_ms + k * tick_ms`・終端 `tick_end = t_k + tick_ms`) の窓は
/// `[tick_end − window_ms, tick_end)` で、再開時刻 (`resuming_ts`) がここに入る pause を集める。
/// ★ 右端は tick の終端で**未来を見ない**。★ tick の帰属規則 (左閉右開・再開側の tick) と同じ向きなので、
/// `window_ms == tick_ms` なら旧来の「その tick に帰属する pause だけ」と一致する（穴が無い限り）。
/// ⚠️ `window_ms` が `tick_ms` 未満なら tick の後半だけを見る (`0` は空の窓 = 全 tick が空)。
///
/// ⚠️★★★ **窓は観測区間 (`feat` 行の穴) をまたがない** (★ 起票 6 の `T1a` 側):
/// `observed` は `stall_onset::observed_intervals` の返り値 (昇順・互いに素) で、
/// 隣り合う区間の間 `(end_j, start_{j+1})` が**穴**である。
///
/// # tick が区間と交わるかの判定は `quality::feat_mask_column` と揃える (2026-09-25・MEDIUM-1)
///
/// ⚠️ **tick の終端 (`tick_end`) を基準に「もう穴に落ちたか」を判定してはならない。**
/// `quality::feat_mask_column` は tick の**開始** `t_k` を基準に交わりを判定する
/// (`t_k <= b && tick_end > a`)。この基準がずれると、grid の起点 (`grid_origin_ms`) が
/// `feat` 行自身の格子とずれたとき (例: `grid_origin_ms = 200`)、区間の右端 `b` を内部に含む tick
/// (例: tick `[19200, 20200)`・`b = 20000`) が「もう次の区間へ進んだ」と誤判定され、
/// **区間の内側で再開した pause が窓から丸ごと消えていた** —— `feat_mask_column` はこの tick を
/// 「観測済み」(`feat = true`) と判定するのに、本関数だけ空の窓を返す食い違いがあった
/// (レビュー指摘・回帰は
/// [`tests::window_survives_offset_grid_origin_when_interval_end_falls_inside_a_tick`])。
/// → 本関数も `t_k` を基準に「次の区間へ進んだか」を判定し、tick が区間と交わるなら
/// **窓の右端もその区間の右端 `b` で切る** (`min(tick_end, b + 1)`。`+1` は区間が閉区間 `[a, b]` で
/// `b` 自身の瞬間を含むため)。
///
/// ⚠️ **穴と重なる pause (開始が穴より前・再開が穴より後) も入れない**
/// —— そうしないと削除後の NDJSON で穴の前後が 1 本の巨大 pause として接続され、
/// その長さが `pause_max_ms` に出る。★ tick が**どの区間とも交わらない**
/// (開始・終端ともに隣り合う区間の間の穴に収まる) tick は**空の窓**になる (0 で埋めず `null` に落ちる)。
/// ⚠️ 穴として扱うのは**隣り合う区間の間だけ**である —— 最初の区間より前・最後の区間より後は
/// 「区間が無い」だけで穴ではない (この 2 つは窓を切らない)。
/// ⚠️★★★ **`observed` が空 (`feat` 行が 1 本も無い) なら、全 tick の窓を空にする** (→ `null`・
/// 2026-09-26 に**「窓を切らずに値を返す」から改めた**)。★ 穴を検出する手段が無い入力では、
/// pause が観測区間の内側かどうかを確かめられない。観測できないものを値で埋めない —— 補訂 1 項目 2 の
/// onset (`feat` 行が無ければ**立てない側**に倒す) と同じ向きに揃えた
/// (研究者裁定・`.steering/20260926-p1-main-prep/decisions.md` 判断 9。ADR-030 補訂 3 項目 1 に記録した)。
/// ⚠️ 境界: 観測区間が **1 本でもあれば**、上のとおり最初の区間より前・最後の区間より後は窓を切らない。
/// → 0 本と 1 本の間で挙動は不連続になる (孤立した `feat` 行 1 本で、その外側の pause が復活しうる)。本規則は変えていない。
/// ⚠️ 帰結: **`feat` 行を持たない入力 (古いログ・一部の合成 fixture) は `pause_*` を全行失う**。
/// ★ `iki_cv` / `surprise` は窓を掛けない列なので、この規則の影響を受けない。
/// ⚠️ 原因ラベルは作らない (ADR-030 却下リスト 9): 使うのは `feat` 行の**存在**だけ。
///
/// 戻り値は tick ごとの pause 長 (ms・**順不同**) で、`compute_dispersion` へそのまま渡す。
/// 計算量は `O(tick_count + pauses + Σ 窓内の pause 数)` (2 本のポインタが単調に進む)。
pub fn pause_ms_by_window(
    pauses: &[PauseDuration],
    tick_count: usize,
    grid_origin_ms: u64,
    tick_ms: u64,
    window_ms: u64,
    observed: &[(u64, u64)],
) -> Vec<Vec<u64>> {
    // ★ `observed` が空 (`feat` 行が 1 本も無い) なら、どの pause も観測区間の内側だと確かめられない。
    //   観測できないものは値で埋めず、全 tick の窓を空にする (→ `compute_dispersion` で `null`)。
    if observed.is_empty() {
        return vec![Vec::new(); tick_count];
    }

    let mut sorted: Vec<PauseDuration> = pauses.to_vec();
    sorted.sort_unstable_by_key(|p| p.resuming_ts);

    let mut out: Vec<Vec<u64>> = Vec::with_capacity(tick_count);
    // left: 窓に入る最初の pause / right: 窓の右端以降の最初の pause (= 窓の外の未来)。
    let (mut left, mut right) = (0_usize, 0_usize);
    // `quality::feat_mask_column` と同じ advance 規則: tick の**開始** `t_k` より前に
    // 閉じている区間だけを読み飛ばす (⚠️ 旧実装は tick の終端を基準にしていたため、区間の
    // 右端 `b` を内部に含む tick を「もう次の区間へ進んだ」と誤判定していた・MEDIUM-1)。
    let mut iv_idx = 0_usize;
    for k in 0..tick_count {
        // security LOW-2: 飽和演算で桁あふれを避ける (`grid_origin_ms` / `tick_ms` は
        // CLI の `--set` 由来なので、極端な値でも panic / wrap しない)。
        let t_k = grid_origin_ms.saturating_add((k as u64).saturating_mul(tick_ms));
        let tick_end = t_k.saturating_add(tick_ms);

        while iv_idx + 1 < observed.len() && observed[iv_idx].1 < t_k {
            iv_idx += 1;
        }
        // `iv_idx == 0` は「最初の区間より前・またはその中」= 左側に穴が無いので下限は 0
        // (最初の区間より前は穴として扱わない・上の doc)。
        let hole_floor = if iv_idx == 0 { 0 } else { observed[iv_idx].0 };

        // tick が現在の区間と交わるか (`t_k <= b` 側だけ確認すればよい —— `iv_idx` の advance
        // 条件が既に `observed[iv_idx].1 >= t_k` を保証している)。交わるなら窓の右端を
        // その区間の右端 `b` で切る。交わらない (= 区間が既に閉じていて、次の区間が無いため
        // これ以上進めない = 最後の区間を過ぎた) 場合は「区間が無いだけで穴ではない」ので
        // `tick_end` をそのまま使う (上の doc)。
        let right_edge = match observed.get(iv_idx) {
            Some(&(_, b)) if t_k <= b => tick_end.min(b.saturating_add(1)),
            _ => tick_end,
        };
        let window_start = right_edge.saturating_sub(window_ms).max(hole_floor);

        while right < sorted.len() && sorted[right].resuming_ts < right_edge {
            right += 1;
        }
        while left < right && sorted[left].resuming_ts < window_start {
            left += 1;
        }
        out.push(
            sorted[left..right]
                .iter()
                .filter(|p| p.start_ts() >= hole_floor)
                .map(|p| p.gap_ms)
                .collect(),
        );
    }
    out
}

/// tick ごとの `Dispersion` を作る純関数。`pause_ms_in_window` は [`pause_ms_by_window`] が作った
/// **その tick の窓**の pause 長、`iki_ms_in_tick` は呼び出し側 (`super::build_feature_rows`) が既に
/// tick へ帰属済みの値を渡す (`activity::compute_t0_activity` / `boundary::compute_boundary` と対称)。
/// ⚠️ `iki_ms_in_tick` に窓は掛からない (`iki_cv` は tick ローカル)。
/// `baseline` はセッション全体から一度だけ作ったものを渡す (tick ごとに作り直さない)。
///
/// ⚠️★★★ **非有限値 (NaN / Inf) を出してはならない。**
/// ★ `serde_json` は NaN / Inf を**エラーにせず黙って `null` に書き換える**
/// (2026-09-15・`security-checker` L2・`serde_json` の `ser.rs` を実読して確認)。
/// → ⚠️ **`null` が「要素が 0 個だった」(AC4) と「計算が壊れた」の両方を意味してしまい、
/// 区別できなくなる。** ★ 現在は 0 除算を事前ガードしているので到達しないが
/// (`SessionIkiBaseline::from_session_iki` の `std_ms == 0.0` /
/// [`coefficient_of_variation`] の `m == 0.0`)、**将来の改修で壊れても無音である**。
/// → ★ 回帰は [`tests::emitted_values_are_always_finite`] が張る。
pub fn compute_dispersion(
    pause_ms_in_window: &[u64],
    iki_ms_in_tick: &[u64],
    baseline: Option<&SessionIkiBaseline>,
) -> Dispersion {
    let mut sorted_pause: Vec<u64> = pause_ms_in_window.to_vec();
    sorted_pause.sort_unstable();

    let surprise = match (mean(iki_ms_in_tick), baseline) {
        (Some(tick_mean), Some(b)) => Some((tick_mean - b.mean_ms) / b.std_ms),
        _ => None,
    };

    Dispersion {
        pause_p50_ms: quantile(&sorted_pause, 0.5),
        pause_p90_ms: quantile(&sorted_pause, 0.9),
        pause_max_ms: quantile(&sorted_pause, 1.0),
        iki_cv: coefficient_of_variation(iki_ms_in_tick),
        surprise,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC2: 分位点が境界値 (要素 0 / 1 / 2 / 偶数 / 奇数個) で正しい。
    #[test]
    fn quantiles_handle_boundary_sizes() {
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&[], 1.0), None);

        assert_eq!(quantile(&[42], 0.5), Some(42.0));
        assert_eq!(quantile(&[42], 0.9), Some(42.0));
        assert_eq!(quantile(&[42], 1.0), Some(42.0));

        let two = [10, 20];
        assert_eq!(quantile(&two, 0.5), Some(10.0));
        assert_eq!(quantile(&two, 0.9), Some(20.0));
        assert_eq!(quantile(&two, 1.0), Some(20.0));

        let even = [10, 20, 30, 40];
        assert_eq!(quantile(&even, 0.5), Some(20.0));
        assert_eq!(quantile(&even, 1.0), Some(40.0));

        let odd = [10, 20, 30, 40, 50];
        assert_eq!(quantile(&odd, 0.5), Some(30.0));
        assert_eq!(quantile(&odd, 0.9), Some(50.0));
        assert_eq!(quantile(&odd, 1.0), Some(50.0));
    }

    /// AC4: 要素が 0 個のとき `dispersion` の全フィールドが `null` (0 で埋めない)。
    #[test]
    fn empty_window_is_null_not_zero() {
        let d = compute_dispersion(&[], &[], None);
        assert_eq!(d.pause_p50_ms, None);
        assert_eq!(d.pause_p90_ms, None);
        assert_eq!(d.pause_max_ms, None);
        assert_eq!(d.iki_cv, None);
        assert_eq!(d.surprise, None);
    }

    /// AC5: surprise はセッション全体の baseline (関数の引数) からの逸脱として計算される
    /// — tick ローカルの値だけからは作り直さない。
    #[test]
    fn surprise_is_session_relative() {
        // mean=100, std=20 になるよう選んだセッション全体の IKI。
        let session_iki = vec![80, 120, 80, 120];
        let baseline = SessionIkiBaseline::from_session_iki(&session_iki).expect("variance > 0");

        let spike = compute_dispersion(&[], &[140], Some(&baseline));
        assert_eq!(spike.surprise, Some(2.0)); // (140 - 100) / 20

        let calm = compute_dispersion(&[], &[100], Some(&baseline));
        assert_eq!(calm.surprise, Some(0.0));

        // baseline が無い (呼び出し側がセッション全体で分散 0 等と判断した) tick は null。
        let no_baseline = compute_dispersion(&[], &[140], None);
        assert_eq!(no_baseline.surprise, None);
    }

    /// ⚠️★★★ **2026-09-15 に検出力を足した** (`code-reviewer` HIGH-2)。
    ///
    /// ★ 旧版の入力は `[0, PAUSE_THRESHOLD_MS, PAUSE_THRESHOLD_MS+100]` ＝
    /// **pause 1 本・時刻重複なし**だったので `i-1` が常に 0 になり、
    /// ⚠️ **`gap` を `sorted[i] - sorted[0]` と書き間違えても、`next_expected` を進め忘れても
    /// 同じ結果を返した**（★ reviewer が mutation harness で 3 実装の出力一致を実測）。
    /// → ★ **pause 2 本以上 ＋ 同時刻重複**を入れて、対の突き合わせを実際に固定する。
    #[test]
    fn pause_durations_pairs_gap_with_resuming_ts() {
        use crate::analysis::features::PAUSE_THRESHOLD_MS;
        let press_ts = vec![0, PAUSE_THRESHOLD_MS, PAUSE_THRESHOLD_MS + 100];
        let durations = pause_durations(&press_ts);
        // 0 -> PAUSE_THRESHOLD_MS は閾値ちょうどで pause、続く +100 は pause でない。
        assert_eq!(durations.len(), 1);
        assert_eq!(durations[0].resuming_ts, PAUSE_THRESHOLD_MS);
        assert_eq!(durations[0].gap_ms, PAUSE_THRESHOLD_MS);

        // ★ pause 2 本。⚠️ 2 本目の gap は「直前の press との差」であり、
        //   **セッション先頭からの差ではない**（★ `sorted[i] - sorted[0]` と書くと 6000 になる）。
        let two = vec![0_u64, 3000, 6000];
        let d2 = pause_durations(&two);
        assert_eq!(d2.len(), 2);
        assert_eq!((d2[0].resuming_ts, d2[0].gap_ms), (3000, 3000));
        assert_eq!(
            (d2[1].resuming_ts, d2[1].gap_ms),
            (6000, 3000),
            "2 本目の gap は直前 press との差 (先頭との差ではない)"
        );

        // ★ 同時刻重複を挟む。⚠️ 重複で対がずれると gap が変わる。
        //   0 / 0 / 2500 / 2500 / 5000 → pause は 2500(gap 2500) と 5000(gap 2500)。
        let dup = vec![0_u64, 0, 2500, 2500, 5000];
        let d3 = pause_durations(&dup);
        assert_eq!(d3.len(), 2, "同時刻重複があっても pause は 2 本: {d3:?}");
        assert_eq!((d3[0].resuming_ts, d3[0].gap_ms), (2500, 2500));
        assert_eq!(
            (d3[1].resuming_ts, d3[1].gap_ms),
            (5000, 2500),
            "重複時刻を挟んでも直前 press との差になる"
        );

        // ★ 重複が「pause の開始側」に来る場合。
        //   0 / 2500 / 2500 / 2500 / 7000 → pause は 2500(gap 2500) と 7000(gap 4500)。
        let dup2 = vec![0_u64, 2500, 2500, 2500, 7000];
        let d4 = pause_durations(&dup2);
        assert_eq!(d4.len(), 2, "{d4:?}");
        assert_eq!((d4[0].resuming_ts, d4[0].gap_ms), (2500, 2500));
        assert_eq!((d4[1].resuming_ts, d4[1].gap_ms), (7000, 4500));
    }

    #[test]
    fn iki_cv_needs_at_least_two_points() {
        assert_eq!(coefficient_of_variation(&[]), None);
        assert_eq!(coefficient_of_variation(&[100]), None);
        // mean == 0 (同時刻打鍵のみ) は除算不能なので null。
        assert_eq!(coefficient_of_variation(&[0, 0]), None);
        // std と mean が等しくなる (gap 0 と 2*mean の2点) ため cv はちょうど 1.0。
        assert_eq!(coefficient_of_variation(&[0, 2000]), Some(1.0));
    }

    /// ⚠️★★★ **出力に非有限値 (NaN / Inf) が 1 つも出ないこと**
    /// (2026-09-15・`security-checker` L2)。
    ///
    /// ★ `serde_json` は NaN / Inf を**エラーにせず黙って `null` に書き換える**ので、
    /// ⚠️ 壊れた計算が **AC4 の「要素 0 個 = `null`」と見分けのつかない `null`** に化ける。
    /// → ★ 「壊れたら赤くなる」状態をここで作る (⚠️ 無音で通らせない)。
    #[test]
    fn emitted_values_are_always_finite() {
        use crate::analysis::features::PAUSE_THRESHOLD_MS;

        let check = |d: &Dispersion, label: &str| {
            for (name, v) in [
                ("pause_p50_ms", d.pause_p50_ms),
                ("pause_p90_ms", d.pause_p90_ms),
                ("pause_max_ms", d.pause_max_ms),
                ("iki_cv", d.iki_cv),
                ("surprise", d.surprise),
            ] {
                if let Some(x) = v {
                    assert!(
                        x.is_finite(),
                        "{label}: {name} が非有限 ({x}) —— serde_json は黙って null にする"
                    );
                }
            }
        };

        // ★ 分母が 0 になりうる入力を並べる (⚠️ 想像で並べていない: 0 除算の 2 経路に対応)。
        // (a) 全 IKI が同値 → baseline の分散 0
        let same = vec![100_u64, 100, 100];
        check(
            &compute_dispersion(
                &[],
                &same,
                SessionIkiBaseline::from_session_iki(&same).as_ref(),
            ),
            "全 IKI 同値",
        );
        // (b) 全 IKI が 0ms (同時刻打鍵のみ) → cv の分母 0
        let zeros = vec![0_u64, 0, 0];
        check(
            &compute_dispersion(
                &[],
                &zeros,
                SessionIkiBaseline::from_session_iki(&zeros).as_ref(),
            ),
            "全 IKI が 0ms",
        );
        // (c) 空 tick
        check(&compute_dispersion(&[], &[], None), "空 tick");
        // (d) 通常入力 + 極端に大きい pause
        let wide = vec![1_u64, u32::MAX as u64];
        check(
            &compute_dispersion(
                &[PAUSE_THRESHOLD_MS, u32::MAX as u64],
                &wide,
                SessionIkiBaseline::from_session_iki(&wide).as_ref(),
            ),
            "極端に広い分布",
        );

        // ★ 実際に serde_json を通して `null` が混ざらないことも見る (⚠️ 型だけでは足りない)。
        let d = compute_dispersion(
            &[PAUSE_THRESHOLD_MS],
            &[100, 300],
            SessionIkiBaseline::from_session_iki(&[100, 300]).as_ref(),
        );
        let json = serde_json::to_string(&d).expect("serialize");
        assert!(
            !json.contains("null"),
            "値が揃っている入力で null が出た (非有限値が null に化けた疑い): {json}"
        );
    }

    fn pd(resuming_ts: u64, gap_ms: u64) -> PauseDuration {
        PauseDuration {
            resuming_ts,
            gap_ms,
        }
    }

    /// AC5 (issue 003): 窓は**因果的**である —— tick の値は `[tick_end − window, tick_end)` に
    /// 再開する pause だけから作り、未来を見ない。
    #[test]
    fn window_is_causal_and_never_looks_ahead() {
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        let t = |n: u64| n * ENGINE_TICK_MS;
        // 再開が tick 3 / 6 / 9 / 12 の先頭。gap は全部別の値 (★ どの pause が窓に入ったか見分ける)。
        let all = vec![
            pd(t(3), 2500),
            pd(t(6), 2600),
            pd(t(9), 2700),
            pd(t(12), 2800),
        ];
        let ticks = 15;
        let window = 3 * ENGINE_TICK_MS;
        // ★ 全体を覆う 1 本の観測区間 (穴なし)。⚠️ `&[]` (= `feat` 行が 1 本も無い) だと全 tick が空になる。
        let whole = [(0, t(ticks as u64))];
        let run =
            |ps: &[PauseDuration]| pause_ms_by_window(ps, ticks, 0, ENGINE_TICK_MS, window, &whole);
        let full = run(&all);

        // tick 2 = [t(2), t(3)): 再開 t(3) は tick 3 に属する。★ **未来なのでまだ見えない**。
        assert!(full[2].is_empty(), "{:?}", full[2]);
        // tick 3: 自 tick に再開した pause が見える。
        assert_eq!(full[3], vec![2500]);
        // tick 5 (終端 t(6)・窓 [t(3), t(6))): 下限 t(3) は含み、上限 t(6) は含まない (tick 6 の pause)。
        assert_eq!(full[5], vec![2500]);
        // tick 6 (終端 t(7)・窓 [t(4), t(7))): t(3) は窓の外へ出て、t(6) が入る。
        assert_eq!(full[6], vec![2600]);

        // ★ 未来の pause を 1 本足しても、その pause の tick より前の行は 1 つも変わらない。
        for cut in 0..all.len() {
            let partial = run(&all[..cut]);
            let cut_tick = (all[cut].resuming_ts / ENGINE_TICK_MS) as usize;
            for k in 0..cut_tick {
                assert_eq!(
                    full[k], partial[k],
                    "tick {k} が未来の pause (cut={cut}) に依存している"
                );
            }
            // ★ 陽性対照: pause 自身の tick では差が出る (= 窓が pause を実際に見ている)。
            assert_ne!(
                full[cut_tick], partial[cut_tick],
                "cut={cut} の pause が窓に入っていない"
            );
        }
    }

    /// 窓長 `0` は空の窓であり、panic せず全 tick が空になる (★ CLI は `t1a_window_ms >= 1` に限る)。
    #[test]
    fn zero_window_is_empty_and_does_not_panic() {
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        let all = vec![pd(3 * ENGINE_TICK_MS, 2500)];
        // ⚠️ 観測区間を渡す (`&[]` だと窓長に関係なく全 tick が空で、この検査が空振りする)。
        let whole = [(0, 6 * ENGINE_TICK_MS)];
        let out = pause_ms_by_window(&all, 6, 0, ENGINE_TICK_MS, 0, &whole);
        assert_eq!(out.len(), 6);
        assert!(out.iter().all(|v| v.is_empty()), "{out:?}");
        // ★ 陽性対照: 窓長だけを 1 tick に戻すと同じ入力で pause が見える (= 空の原因は窓長 0)。
        let one_tick = pause_ms_by_window(&all, 6, 0, ENGINE_TICK_MS, ENGINE_TICK_MS, &whole);
        assert_eq!(one_tick[3], vec![2500], "{one_tick:?}");
    }

    /// AC1 (issue 008・判断 9): **`feat` 行が 1 本も無い (`observed` が空) 入力では、pause が在っても
    /// 全 tick の窓が空になる** —— 観測できないものを値で埋めない (onset と同じ向き)。
    /// ★ 陽性対照: 同じ pause でも観測区間が 1 本あれば窓に入る (= 空の原因は `observed` が空なこと)。
    #[test]
    fn no_observed_interval_yields_empty_windows() {
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        let t = |n: u64| n * ENGINE_TICK_MS;
        let all = [pd(t(3), 2500), pd(t(6), 2600), pd(t(9), 2700)];
        let ticks = 12;
        // 窓は全体を覆う広さ (窓長が原因で空になっていないことを保つ)。
        let window = t(ticks as u64);

        let none = pause_ms_by_window(&all, ticks, 0, ENGINE_TICK_MS, window, &[]);
        assert_eq!(none.len(), ticks, "tick 数は保たれる (1 行 1 tick)");
        assert!(none.iter().all(|w| w.is_empty()), "{none:?}");

        let whole = [(0, t(ticks as u64))];
        let observed = pause_ms_by_window(&all, ticks, 0, ENGINE_TICK_MS, window, &whole);
        assert_eq!(observed[11], vec![2500, 2600, 2700], "{observed:?}");

        // ★ 空の窓は `compute_dispersion` で `null` になる (0 で埋めない)。
        let d = compute_dispersion(&none[11], &[], None);
        assert_eq!(
            (d.pause_p50_ms, d.pause_p90_ms, d.pause_max_ms),
            (None, None, None)
        );
        // 境界: tick が 0 本でも panic しない。
        assert!(pause_ms_by_window(&all, 0, 0, ENGINE_TICK_MS, window, &[]).is_empty());
    }

    /// AC6 (issue 003・起票 6): 窓は**観測区間 (`feat` 行の穴) をまたがない**。
    /// ⚠️ 穴を挟んだ pause は同じ窓に入らず、**穴と重なる 1 本の巨大 pause も入らない**。
    #[test]
    fn window_does_not_span_a_feat_hole() {
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        let t = |n: u64| n * ENGINE_TICK_MS;
        // 観測区間は [t(1), t(20)] と [t(40), t(56)]。間の (t(20), t(40)) が穴。
        let observed = [(t(1), t(20)), (t(40), t(56))];
        let before_hole = pd(t(10), 3000); // 区間 0 の中
        let straddler = pd(t(44), t(25)); // 開始 t(19) は区間 0・再開 t(44) は区間 1 → 穴と重なる
        let after_hole = pd(t(52), 3300); // 区間 1 の中 (開始 t(52) − 3300 は t(40) 以降)
        let all = [before_hole, straddler, after_hole];
        let ticks = 60;
        // ★ 全体を覆う広い窓 (窓長だけでは 3 本とも入る)。
        let window = t(60);
        let with_holes = pause_ms_by_window(&all, ticks, 0, ENGINE_TICK_MS, window, &observed);
        // ★ 穴の無い版: 同じ範囲を 1 本の観測区間で覆う。⚠️ `&[]` (`feat` 行 0 本) は「穴なし」ではなく
        //   「観測区間が確定しない」なので全 tick が空になる (`no_observed_interval_yields_empty_windows`)。
        let no_info = pause_ms_by_window(&all, ticks, 0, ENGINE_TICK_MS, window, &[(0, t(60))]);

        // ★ 陽性対照: 穴が無ければ 3 本とも同じ窓に入り、巨大 pause が最大値になる。
        assert_eq!(no_info[59], vec![3000, t(25), 3300]);
        // 穴の後の窓には**穴より後で閉じた pause だけ**が入る (前の区間の pause も巨大 pause も入らない)。
        assert_eq!(with_holes[59], vec![3300]);
        // どの tick にも巨大 pause は現れない。
        assert!(
            !with_holes.iter().flatten().any(|&g| g == t(25)),
            "穴をまたぐ pause が窓に入った"
        );

        // 穴より前の tick は切られない (区間 0 の中の pause はそのまま見える)。
        assert_eq!(with_holes[19], vec![3000]);
        // ★ 終端が穴の中にある tick は空の窓 (0 で埋めず null に落ちる)。
        assert!(with_holes[29].is_empty(), "{:?}", with_holes[29]);
        assert!(with_holes[41].is_empty(), "{:?}", with_holes[41]);
        assert_eq!(no_info[29], vec![3000], "陽性対照: 穴が無ければ見える");
    }

    /// MEDIUM-1 (issue 003 レビュー・2026-09-25): grid の起点 (`grid_origin_ms`) が `feat` 行
    /// 自身の格子とずれていても、観測区間の右端 `b` を内部に含む tick で「区間の内側で再開した
    /// pause」が窓から消えない。
    ///
    /// ⚠️ 旧実装は「もう次の区間へ進んだか」を tick の**終端** (`tick_end - 1`) を基準に判定して
    /// いたため、tick `[19200, 20200)` (`b = 20000` を内部に含む) で `20000 < 20199` が成立し、
    /// 区間の内側で再開した pause がまだ存在するにもかかわらず窓が丸ごと空になっていた。
    /// ★ この修正 (`t_k` 基準に変更 + 右端クリップ) を戻すと本テストは赤になる
    /// (2026-09-25・変異テストで確認済み)。
    #[test]
    fn window_survives_offset_grid_origin_when_interval_end_falls_inside_a_tick() {
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        // grid_origin_ms を格子からずらす (レビュー指摘の実例と同じ 200ms オフセット)。
        let grid_origin_ms = 200_u64;
        // 観測区間: [1000, 20000] と [40000, 60000] (間の (20000, 40000) が穴)。
        let observed = [(1000_u64, 20000_u64), (40000_u64, 60000_u64)];
        // tick 19 = [19200, 20200): 区間の右端 b=20000 をこの tick の内部に含む
        // (`quality::feat_mask_column` はこの tick を `feat = true` と判定する)。
        let inside_before_b = pd(19_700, 500); // 区間 1 の内側 (19200..20000) で再開
                                               // 穴をまたぐ pause: 開始 (19900) は区間 1 の内側、再開 (40500) は区間 2 の内側。
                                               // どの tick の窓にも入ってはならない。
        let straddler = pd(40_500, 20_600);
        let all = [inside_before_b, straddler];
        let ticks = 45;
        let window = 60 * ENGINE_TICK_MS; // 広い窓 (窓長だけでは両方とも入りうる)
        let out = pause_ms_by_window(
            &all,
            ticks,
            grid_origin_ms,
            ENGINE_TICK_MS,
            window,
            &observed,
        );

        assert_eq!(
            out[19],
            vec![500],
            "区間の内側で再開した pause が、b を内部に含む tick の窓から消えている: {:?}",
            out[19]
        );
        assert!(
            !out.iter().flatten().any(|&g| g == 20_600),
            "穴をまたぐ pause が窓に入った"
        );
    }

    /// AC7 (issue 003): 窓は **`iki_cv` / `surprise` を変えない** (⚠️ pause 分位点だけに掛かる)。
    /// ★ 窓を 1 tick と広い窓で `build_feature_rows` を通し、他の列が 1 つも動かないことを固定する。
    #[test]
    fn window_does_not_change_iki_cv_or_surprise() {
        use crate::analysis::feat_offline::{
            build_feature_rows, QualityInputs, StallOnsetConfig, StallOnsetInputs,
        };
        use crate::analysis::loop_step::ENGINE_TICK_MS;
        use crate::analysis::metrics::KeyPress;

        // 間隔がまちまちで pause を何本も含む打鍵列 (0x41 = 'A' は文字生成キー)。
        let ts = [
            0_u64, 150, 300, 3000, 3200, 3400, 6500, 6600, 6900, 10_000, 10_100, 13_500,
        ];
        let keys: Vec<KeyPress> = ts
            .iter()
            .map(|&timestamp| KeyPress {
                timestamp,
                vk: 0x41,
            })
            .collect();
        // ★ 1Hz の `feat` 行で全体を覆う (穴なし)。⚠️ `feat` 行が 0 本だと `pause_*` は全行 `null` になり、
        //   下の陽性対照 (窓を広げると pause 列が動く) が空振りする。
        let feat_ts: Vec<u64> = (0..=14).map(|i| i * 1000).collect();
        let run = |window_ms: u64| {
            let inputs = StallOnsetInputs {
                feat_ts: &feat_ts,
                task_kind_declarations: &[],
                config: StallOnsetConfig {
                    t1a_window_ms: window_ms,
                    ..StallOnsetConfig::default()
                },
            };
            let q = QualityInputs {
                capabilities_declared: false,
                hook_health: &[],
            };
            build_feature_rows(&keys, &[], &[], 10_000, &inputs, &q).expect("within tick limit")
        };
        let narrow = run(ENGINE_TICK_MS);
        let wide = run(StallOnsetConfig::default().t1a_window_ms);
        assert_eq!(narrow.len(), wide.len());

        let mut pause_columns_moved = false;
        for (i, (n, w)) in narrow.iter().zip(&wide).enumerate() {
            assert_eq!(
                n.dispersion.iki_cv, w.dispersion.iki_cv,
                "tick {i}: iki_cv が窓で動いた"
            );
            assert_eq!(
                n.dispersion.surprise, w.dispersion.surprise,
                "tick {i}: surprise が窓で動いた"
            );
            // dispersion 以外の列も 1 つも動かない。
            let mut nv = serde_json::to_value(n).expect("serialize");
            let mut wv = serde_json::to_value(w).expect("serialize");
            nv.as_object_mut().expect("object").remove("dispersion");
            wv.as_object_mut().expect("object").remove("dispersion");
            assert_eq!(nv, wv, "tick {i}: dispersion 以外の列が窓で動いた");
            if n.dispersion.pause_p50_ms != w.dispersion.pause_p50_ms
                || n.dispersion.pause_max_ms != w.dispersion.pause_max_ms
            {
                pause_columns_moved = true;
            }
        }
        // ★ 陽性対照: 窓を広げると pause 列は動く (= この入力で窓が効いている)。
        assert!(
            pause_columns_moved,
            "窓を広げても pause 列が動かない (入力が窓を試せていない)"
        );
    }
}
