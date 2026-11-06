//! 長い穴とファイルの最後の直前にある、入力の無い `feat` 行を落とす前処理
//! (P1 事前宣言 v1.2・ADR-030 補訂 4・issue #330)。
//!
//! # なぜ要るか
//!
//! Windows のモダンスタンバイでは、電源につないだまま寝かせると、寝た後もしばらく
//! `feat` 行が 1Hz で書かれ続ける (2026-10-08 の実測で 16 分)。観測区間は `feat` 行の
//! **存在**だけで切るので ([`super::stall_onset::observed_intervals`])、その間が
//! 「観測していて、手が止まっている」区間になり、寝ている間に stall onset が立ちうる。
//! v0.1.40 の NDJSON には寝始めた時刻を示す行が無いので、構造から推して削る。
//!
//! # 規則 (研究者裁定 2026-10-08・`.steering/20261008-p1-sleep-trim/decisions.md` 判断 1)
//!
//! 1. 終わりの点 `h`: 次の `feat` 行との差が `min_gap_ms` **以上**ある `feat` 行 (長い穴の直前) と、
//!    時刻が最大の `feat` 行 (ファイルの最後。⚠️ 時刻で並べ替えた後の最後であり、ファイルの行順ではない)。
//! 2. `L` = `h` 以下で最後の入力の時刻。入力は呼び出し側が渡す (`gse-replay` は `key` 行と
//!    `pointer_bin` 行の時刻だけを渡す。`ime_*` / `focus_transition` はシステムが出しうるので入れない)。
//! 3. `R` = `L` 以上で最初の `feat` 行 (最後の入力を含む秒の行。`h` 自身が候補なので必ずある)。
//!    `cut = max(R, h − max_trim_ms)`。`L` が無く `h − max_trim_ms` も負なら下限なし。
//!    ★ `R` まで残すので、最後の入力は観測区間の内側に入る (右端を「最後の入力まで」戻す・判断 6)。
//!    ⚠️ ただし最後の入力が `feat` 行の穴 (`hole_gap_ms` を超える間隔) の中にあるときは、`R` は穴の後ろの行になり、
//!    入力そのものはどの観測区間にも入らない (穴の中の入力は前処理が無くても観測区間の外である)。
//! 4. `cut < t ≤ h` の `feat` 行を落とす。`max_trim_ms == 0` なら `cut = h` で何も落ちない。
//!
//! 落とした行は「入力ファイルに無かった」のと同じに扱う (呼び出し側は以後この列だけを使う)。
//! 原因のラベルは作らない (ADR-030 D8 (b-2))。規則は穴の長さと入力の有無だけで書く。
//!
//! # 先読み
//!
//! この前処理は retrospective である: `t` を落とすかは、`t` より後にある `feat` 行の穴と、
//! `t` から `h` までの入力の有無で決まる (最大 `max_trim_ms + min_gap_ms` 先まで見る)。
//! 検出器 ([`super::stall_onset`]) の因果性 (`key` 行の先読みゼロ) は、削った後の列を入力とする
//! 範囲でそのまま成り立つ。
//!
//! # 前提
//!
//! `min_gap_ms` は観測区間を切る `hole_gap_ms` より大きいこと (呼び出し側の `validate` が保証する)。
//! それ以下だと、穴でない間隔の前でも削ってしまう。

/// 前処理で落とした量 (封筒の `provenance.idle_trim` に載せる)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdleTrimSummary {
    /// 落とした `feat` 行の数。
    pub dropped_feat_rows: u64,
    /// 落とした行のつながり (1 本 = 1 つの終わりの点で落とした行の、最初と最後の時刻)。
    /// 短い穴 (`min_gap_ms` 未満) をまたいで落ちた行は 1 本につながり、長い穴をまたげば別の本になる
    /// (本数 = 行を落とした終わりの点の数)。
    pub spans: Vec<(u64, u64)>,
}

/// [`idle_trim`] の本体。`feat_ts` / `input_ts` は順不同・重複ありでよい。
/// 戻り値の `feat` 列は昇順・重複除去済み。
pub fn idle_trim(
    feat_ts: &[u64],
    input_ts: &[u64],
    min_gap_ms: u64,
    max_trim_ms: u64,
) -> (Vec<u64>, IdleTrimSummary) {
    let mut feat: Vec<u64> = feat_ts.to_vec();
    feat.sort_unstable();
    feat.dedup();
    if feat.is_empty() {
        return (feat, IdleTrimSummary::default());
    }
    let mut input: Vec<u64> = input_ts.to_vec();
    input.sort_unstable();

    // 終わりの点 `h` と、そこで残す行の下限 `cut` (`cut < t ≤ h` を落とす。`None` は下限なし)。
    // 最後の `feat` 行は必ず終わりの点なので、`ends` は空にならない。
    let mut ends: Vec<(u64, Option<u64>)> = Vec::new();
    for (i, &h) in feat.iter().enumerate() {
        let is_end = match feat.get(i + 1) {
            Some(&next) => next - h >= min_gap_ms,
            None => true,
        };
        if !is_end {
            continue;
        }
        let last_input = input
            .partition_point(|&t| t <= h)
            .checked_sub(1)
            .map(|j| input[j]);
        // ★ 最後の入力を含む秒の `feat` 行 (入力以上で最初の `feat` 行・`h` 自身が候補なので必ずある) は残す。
        //   → 最後の入力が観測区間の内側に入る (右端を「最後の入力まで」戻す)。
        let last_input_row = last_input.map(|l| feat[feat.partition_point(|&t| t < l)]);
        let floor = h.checked_sub(max_trim_ms);
        let cut = match (last_input_row, floor) {
            (Some(l), Some(f)) => Some(l.max(f)),
            (Some(l), None) => Some(l),
            (None, f) => f,
        };
        ends.push((h, cut));
    }

    // `cut` は `h` について単調非減少 (`L` も `h − max_trim_ms` も単調・`None` を最小とみなす)。
    // → 各行は「その行以上で最初の終わりの点」の `cut` だけを見ればよい
    //   (それより後の点の `cut` は同じか大きく、それより前の点は行より前で終わる)。
    // ★ `spans` は終わりの点ごとに分ける (1 本 = 1 つの終わりの点で落とした行)。→ 本数 = 削った回数。
    //   短い穴をまたいでも同じ終わりの点なら 1 本、長い穴をまたげば別の本になる。
    let mut kept: Vec<u64> = Vec::with_capacity(feat.len());
    let mut summary = IdleTrimSummary::default();
    // (終わりの点の添字, 最初の時刻, 最後の時刻)
    let mut open_span: Option<(usize, u64, u64)> = None;
    let mut k = 0;
    for &t in &feat {
        while ends[k].0 < t {
            k += 1;
        }
        let dropped = ends[k].1.is_none_or(|cut| t > cut);
        if dropped {
            summary.dropped_feat_rows += 1;
            open_span = match open_span {
                Some((end, a, _)) if end == k => Some((end, a, t)),
                other => {
                    if let Some((_, a, b)) = other {
                        summary.spans.push((a, b));
                    }
                    Some((k, t, t))
                }
            };
        } else {
            if let Some((_, a, b)) = open_span.take() {
                summary.spans.push((a, b));
            }
            kept.push(t);
        }
    }
    if let Some((_, a, b)) = open_span {
        summary.spans.push((a, b));
    }
    (kept, summary)
}

/// 長い無入力の前処理 (P1 事前宣言 v1.3 §7・ADR-030 補訂 5・`.steering/20261011-o1-awake-idle-label/`)。
///
/// # なぜ要るか
///
/// O1 のアプリは宣言したタスクを自分では外さないので、PC が起きていれば、席を外していても
/// `feat` 行が 1Hz で続く。[`idle_trim`] は `feat` 行の穴の直前しか見ないので、行が途切れない
/// 「起きたままの無入力」は残る (2026-10-09〜11 の研究者の実走では、3 日間 `feat` 行の穴が 0 個で、
/// 30 分以上の無入力が `feat` 行のある時間の約 7 割を占めた)。
///
/// # 規則 (研究者裁定 2026-10-11・判断 3〜6)
///
/// 1. 入力の時刻を並べ、最初の入力が `file_first` より後なら頭に `file_first` を、最後の入力が
///    `file_last` より前なら末尾に `file_last` を置く (入力が無ければ 2 点だけ)。
/// 2. 隣り合う 2 点 `a < b` の差が `min_input_gap_ms` **以上**なら、その間の行を落とす:
///    - `a` が入力なら、`a` 以上で最初の `feat` 行 (`a` を含む秒の行) を残し、それより後を落とす。
///      ⚠️ `a` が `feat` 行の穴の中やファイルの最初より前にあるときは、残る行は `a` を含まず、
///      穴の後ろ (ファイルの最初) の行になる ([`idle_trim`] の `R` と同じ規約)。
///      `a` が `file_first` なら、`file_first` 以上を落とす。
///    - `b` が入力なら、`b` より前を落とす (`b` を含む秒の行は残る)。`b` が `file_last` なら、`file_last` 以下を落とす。
/// 3. `min_input_gap_ms == 0` なら何も落とさない。
///
/// 長さは壁時計で測る (区間の中の `feat` 行の穴を差し引かない)。原因のラベルは作らない。
///
/// # idle trim との関係
///
/// 呼び出し側は [`idle_trim`] の**後の**列を `feat_ts` に渡し、`file_first` / `file_last` には
/// **前の**列の最小・最大を渡す。規則は入力と境だけで決まり、[`idle_trim`] は入力を含む秒の行を
/// 落とさないので、落ちる行は 2 つの規則の和集合になる。`summary` が数えるのは、ここで新たに落とした行だけ。
///
/// # 先読み
///
/// retrospective である: 行を落とすかは、次の入力が `min_input_gap_ms` 以内に来るかで決まる
/// (最大 `min_input_gap_ms` 先まで見る)。
pub fn input_gap_trim(
    feat_ts: &[u64],
    input_ts: &[u64],
    file_first: u64,
    file_last: u64,
    min_input_gap_ms: u64,
) -> (Vec<u64>, IdleTrimSummary) {
    let mut feat: Vec<u64> = feat_ts.to_vec();
    feat.sort_unstable();
    feat.dedup();
    // ⚠️ 境が逆なら何も落とさない (呼び出し側は同じ列の min / max を渡すので起きないが、pub なので守る)。
    if feat.is_empty() || min_input_gap_ms == 0 || file_first > file_last {
        return (feat, IdleTrimSummary::default());
    }
    let mut input: Vec<u64> = input_ts.to_vec();
    input.sort_unstable();
    input.dedup();

    // 区間の両端の点。`true` は入力 (その秒の行を残す)、`false` はファイルの境 (境の行も落とす)。
    let mut points: Vec<(u64, bool)> = Vec::with_capacity(input.len() + 2);
    if input.first().is_none_or(|&i| i > file_first) {
        points.push((file_first, false));
    }
    points.extend(input.iter().map(|&i| (i, true)));
    if input.last().is_none_or(|&i| i < file_last) {
        points.push((file_last, false));
    }

    // 落とす範囲 `[lo, hi]` (両端を含む・昇順・互いに重ならない)。
    // ⚠️ 半開区間にしない —— `file_last == u64::MAX` のとき上端を表せず、最後の行が残る (Codex 指摘・2026-10-11)。
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for pair in points.windows(2) {
        let ((a, a_is_input), (b, b_is_input)) = (pair[0], pair[1]);
        if b - a < min_input_gap_ms {
            continue;
        }
        let lo = if a_is_input {
            // `a` を含む秒の行 (a 以上で最初の行) は残す。無ければ何も落とさない。
            match feat
                .get(feat.partition_point(|&t| t < a))
                .and_then(|&r| r.checked_add(1))
            {
                Some(lo) => lo,
                None => continue,
            }
        } else {
            a
        };
        // `b` が入力なら `b` を含む秒の行 (`b` 以上) は残す。`b > a ≥ 0` なので `b − 1` は下回らない。
        let hi = if b_is_input { b - 1 } else { b };
        if lo <= hi {
            ranges.push((lo, hi));
        }
    }

    let mut kept: Vec<u64> = Vec::with_capacity(feat.len());
    let mut summary = IdleTrimSummary::default();
    let mut open_span: Option<(usize, u64, u64)> = None;
    let mut k = 0;
    for &t in &feat {
        while k < ranges.len() && ranges[k].1 < t {
            k += 1;
        }
        let dropped = k < ranges.len() && ranges[k].0 <= t;
        if dropped {
            summary.dropped_feat_rows += 1;
            open_span = match open_span {
                Some((range, a, _)) if range == k => Some((range, a, t)),
                other => {
                    if let Some((_, a, b)) = other {
                        summary.spans.push((a, b));
                    }
                    Some((k, t, t))
                }
            };
        } else {
            if let Some((_, a, b)) = open_span.take() {
                summary.spans.push((a, b));
            }
            kept.push(t);
        }
    }
    if let Some((_, a, b)) = open_span {
        summary.spans.push((a, b));
    }
    (kept, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN_GAP: u64 = 60_000;
    const MAX_TRIM: u64 = 1_800_000;
    const INPUT_GAP: u64 = 1_800_000;

    /// `[0, to_s]` 秒の `feat` 行に `input_gap_trim` を掛ける (境はその列の最初と最後)。
    fn gap_trim(feat: &[u64], input: &[u64], gap: u64) -> (Vec<u64>, IdleTrimSummary) {
        let first = *feat.iter().min().expect("feat is not empty");
        let last = *feat.iter().max().expect("feat is not empty");
        input_gap_trim(feat, input, first, last, gap)
    }

    #[test]
    fn input_gap_one_ms_short_of_the_threshold_keeps_every_row() {
        let feat = feat_secs(0, 2000);
        let input = [0, INPUT_GAP - 1, 2_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    #[test]
    fn input_gap_exactly_the_threshold_drops_the_rows_between() {
        let feat = feat_secs(0, 2000);
        let input = [0, INPUT_GAP, 2_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        let mut expected = vec![0];
        expected.extend(feat_secs(1800, 2000));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 1799);
        assert_eq!(summary.spans, vec![(1_000, 1_799_000)]);
    }

    /// 前の入力を含む秒の行 (11 秒) と、次の入力を含む秒の行 (1901 秒) は残す。
    #[test]
    fn input_gap_keeps_the_rows_covering_both_inputs() {
        let feat = feat_secs(0, 2000);
        let input = [0, 10_500, 1_900_500, 2_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        let mut expected = feat_secs(0, 11);
        expected.extend(feat_secs(1901, 2000));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 1889);
        assert_eq!(summary.spans, vec![(12_000, 1_900_000)]);
    }

    /// ファイルの最初から最初の入力まで (入力を含む秒の行より前) を落とす。
    #[test]
    fn input_gap_at_the_file_head_drops_from_the_first_row() {
        let feat = feat_secs(0, 2000);
        let (kept, summary) = gap_trim(&feat, &[1_900_000, 2_000_000], INPUT_GAP);
        assert_eq!(kept, feat_secs(1900, 2000));
        assert_eq!(summary.spans, vec![(0, 1_899_000)]);
    }

    /// 最後の入力を含む秒の行の後ろから、ファイルの最後の行までを落とす。
    #[test]
    fn input_gap_at_the_file_tail_drops_to_the_last_row() {
        let feat = feat_secs(0, 2000);
        let (kept, summary) = gap_trim(&feat, &[0, 100_000], INPUT_GAP);
        assert_eq!(kept, feat_secs(0, 100));
        assert_eq!(summary.dropped_feat_rows, 1900);
        assert_eq!(summary.spans, vec![(101_000, 2_000_000)]);
    }

    #[test]
    fn no_input_drops_the_file_only_when_it_is_long_enough() {
        let (kept, summary) = gap_trim(&feat_secs(0, 2000), &[], INPUT_GAP);
        assert!(kept.is_empty());
        assert_eq!(summary.spans, vec![(0, 2_000_000)]);
        let short = feat_secs(0, 1000);
        let (kept, summary) = gap_trim(&short, &[], INPUT_GAP);
        assert_eq!(kept, short);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    /// `min_input_gap_ms == 0` は前処理なし (v1.2 の主と同じ)。
    #[test]
    fn input_gap_zero_keeps_every_row() {
        let feat = feat_secs(0, 2000);
        let (kept, summary) = gap_trim(&feat, &[], 0);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    /// 無入力の区間ごとに 1 本。間の入力を含む秒の行 (1900 秒) は残る。
    #[test]
    fn each_input_gap_is_its_own_span() {
        let feat = feat_secs(0, 4000);
        let input = [0, 1_900_000, 3_900_000, 4_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        let mut expected = vec![0, 1_900_000];
        expected.extend(feat_secs(3900, 4000));
        assert_eq!(kept, expected);
        assert_eq!(
            summary.spans,
            vec![(1_000, 1_899_000), (1_901_000, 3_899_000)]
        );
    }

    /// 長さは壁時計で測る (区間の中の `feat` 行の穴を差し引かない)。穴をまたいでも 1 本。
    #[test]
    fn input_gap_is_measured_in_wall_time_across_a_feat_hole() {
        let mut feat = feat_secs(0, 500);
        feat.extend(feat_secs(1500, 2000));
        let input = [0, 1_900_000, 2_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        let mut expected = vec![0];
        expected.extend(feat_secs(1900, 2000));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 500 + 400);
        assert_eq!(summary.spans, vec![(1_000, 1_899_000)]);
    }

    /// ファイルの最初より前の入力も無入力の長さの起点になる。残すのはそれ以上で最初の行。
    #[test]
    fn input_before_the_file_starts_the_gap() {
        let feat = feat_secs(1000, 2000);
        let input = [0, 1_900_000, 2_000_000];
        let (kept, summary) = gap_trim(&feat, &input, INPUT_GAP);
        let mut expected = vec![1_000_000];
        expected.extend(feat_secs(1900, 2000));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 899);
    }

    /// idle trim の後の列に、元の列の境を渡して掛けると、落ちる行は 2 つの規則の和集合になる。
    /// 数えるのは idle trim が落とさなかった行だけ。
    #[test]
    fn input_gap_trim_after_idle_trim_is_the_union() {
        let mut feat = feat_secs(0, 100);
        feat.extend(feat_secs(200, 2400));
        let input = secs(&[30, 2400]);
        let (after_idle, idle) = idle_trim(&feat, &input, MIN_GAP, MAX_TRIM);
        assert_eq!(idle.spans, vec![(31_000, 100_000)]);
        let (kept, summary) = input_gap_trim(&after_idle, &input, 0, 2_400_000, INPUT_GAP);
        let mut expected = feat_secs(0, 30);
        expected.push(2_400_000);
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 2200);
        assert_eq!(summary.spans, vec![(200_000, 2_399_000)]);
        let (alone, _) = input_gap_trim(&feat, &input, 0, 2_400_000, INPUT_GAP);
        let union: Vec<u64> = alone
            .into_iter()
            .filter(|t| after_idle.binary_search(t).is_ok())
            .collect();
        assert_eq!(kept, union);
    }

    /// 境は idle trim の**前の**列から取る。idle trim が末尾を 30 分削ると後の列の最後は縮むが、
    /// 無入力の長さは元の列で測る (code-reviewer M-1)。★ 後の列の境を渡すと何も落ちない (対照)。
    #[test]
    fn input_gap_is_measured_against_the_bounds_before_idle_trim() {
        // 入力は 0 秒だけで、`feat` 行は 3000 秒まで続く。idle trim は 1201..=3000 秒を落とす。
        let feat = feat_secs(0, 3000);
        let input = [0];
        let (after_idle, idle) = idle_trim(&feat, &input, MIN_GAP, MAX_TRIM);
        assert_eq!(after_idle, feat_secs(0, 1200));
        assert_eq!(idle.dropped_feat_rows, 1800);
        let (kept, summary) = input_gap_trim(&after_idle, &input, 0, 3_000_000, INPUT_GAP);
        assert_eq!(kept, vec![0]);
        assert_eq!(summary.dropped_feat_rows, 1200);
        assert_eq!(summary.spans, vec![(1_000, 1_200_000)]);
        let (wrong, wrong_summary) = input_gap_trim(&after_idle, &input, 0, 1_200_000, INPUT_GAP);
        assert_eq!(wrong, after_idle);
        assert_eq!(wrong_summary, IdleTrimSummary::default());
    }

    /// 時刻が `u64::MAX` の最後の行も落とせる (半開区間だと上端を表せない・Codex 指摘)。
    #[test]
    fn the_last_row_at_u64_max_can_be_dropped() {
        let feat = [u64::MAX - 2, u64::MAX];
        let (kept, summary) = input_gap_trim(&feat, &[], u64::MAX - 2, u64::MAX, 1);
        assert!(kept.is_empty());
        assert_eq!(summary.spans, vec![(u64::MAX - 2, u64::MAX)]);
        // 入力を含む秒の行が `u64::MAX` なら、その後ろには何も無い。
        let (kept, summary) = input_gap_trim(&feat, &[u64::MAX - 1], u64::MAX - 2, u64::MAX, 1);
        assert_eq!(kept, vec![u64::MAX]);
        assert_eq!(summary.dropped_feat_rows, 1);
    }

    /// 境が逆 (`file_first > file_last`) なら何も落とさない (debug ビルドで引き算が下回らない・security-checker LOW)。
    /// ⚠️ ガードを外したときに赤になるのは debug ビルド (overflow 検査) だけである。release では wrap した差が
    /// 閾値を超えても範囲が立たず、緑のままになる (2 巡目の code-reviewer)。
    #[test]
    fn reversed_bounds_drop_nothing() {
        let feat = feat_secs(0, 2000);
        let (kept, summary) = input_gap_trim(&feat, &[], 2_000_000, 0, INPUT_GAP);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    /// `[from_s, to_s]` 秒の 1Hz の `feat` 行 (ms)。
    fn feat_secs(from_s: u64, to_s: u64) -> Vec<u64> {
        (from_s..=to_s).map(|s| s * 1000).collect()
    }

    fn secs(v: &[u64]) -> Vec<u64> {
        v.iter().map(|s| s * 1000).collect()
    }

    #[test]
    fn file_end_drops_rows_after_the_last_input() {
        let feat = feat_secs(0, 100);
        let (kept, summary) = idle_trim(&feat, &secs(&[40]), MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat_secs(0, 40));
        assert_eq!(summary.dropped_feat_rows, 60);
        assert_eq!(summary.spans, vec![(41_000, 100_000)]);
    }

    #[test]
    fn long_gap_drops_rows_before_the_gap() {
        // 0..=100 秒、100 秒の穴、200..=210 秒。入力は 30 秒と、ファイルの最後 (210 秒)。
        let mut feat = feat_secs(0, 100);
        feat.extend(feat_secs(200, 210));
        let (kept, summary) = idle_trim(&feat, &secs(&[30, 205, 210]), MIN_GAP, MAX_TRIM);
        let mut expected = feat_secs(0, 30);
        expected.extend(feat_secs(200, 210));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 70);
        assert_eq!(summary.spans, vec![(31_000, 100_000)]);
    }

    #[test]
    fn gap_one_ms_short_of_min_gap_does_not_trigger() {
        // 穴は 59,999 ms。入力は最後の行と同時刻 (ファイルの最後では何も落ちない)。
        let mut feat = feat_secs(0, 100);
        let after = 100_000 + MIN_GAP - 1;
        feat.extend([after, after + 1000]);
        let (kept, summary) = idle_trim(&feat, &[10_000, after + 1000], MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    #[test]
    fn gap_exactly_min_gap_triggers() {
        let mut feat = feat_secs(0, 100);
        let after = 100_000 + MIN_GAP;
        feat.extend([after, after + 1000]);
        let (kept, summary) = idle_trim(&feat, &[10_000, after + 1000], MIN_GAP, MAX_TRIM);
        let mut expected = feat_secs(0, 10);
        expected.extend([after, after + 1000]);
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 90);
    }

    #[test]
    fn max_trim_caps_how_far_back_the_rows_are_dropped() {
        // 入力は 0 秒だけで、40 分 (2400 秒) 無入力のまま穴に入る。戻すのは 30 分まで。
        let mut feat = feat_secs(0, 2400);
        feat.extend(feat_secs(3000, 3001));
        let (kept, summary) = idle_trim(&feat, &secs(&[0, 3001]), MIN_GAP, MAX_TRIM);
        let mut expected = feat_secs(0, 600);
        expected.extend(feat_secs(3000, 3001));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 1800);
        assert_eq!(summary.spans, vec![(601_000, 2_400_000)]);
    }

    #[test]
    fn max_trim_zero_keeps_every_row() {
        let mut feat = feat_secs(0, 100);
        feat.extend(feat_secs(200, 210));
        let (kept, summary) = idle_trim(&feat, &secs(&[30]), MIN_GAP, 0);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    #[test]
    fn input_after_the_end_point_does_not_count() {
        // 穴の後の入力 (150 秒) は、穴の前の終わりの点 (100 秒) の `L` にならない。
        let mut feat = feat_secs(0, 100);
        feat.extend(feat_secs(150, 160));
        let (kept, _) = idle_trim(&feat, &secs(&[20, 150, 160]), MIN_GAP - 10_000, MAX_TRIM);
        let mut expected = feat_secs(0, 20);
        expected.extend(feat_secs(150, 160));
        assert_eq!(kept, expected);
    }

    /// 入力が `feat` 行の間 (40.5 秒) にあるとき、その入力を含む秒の `feat` 行 (41 秒) は残す。
    /// ★ そうしないと、最後の入力が観測区間の右端より後ろに落ち、区間の外に出る
    /// (2026-10-08 の実記録で、最後の打鍵が区間の外に出て θ の標本が 1 つ減った)。
    #[test]
    fn the_feat_row_covering_the_last_input_is_kept() {
        let feat = feat_secs(0, 100);
        let (kept, summary) = idle_trim(&feat, &[40_500], MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat_secs(0, 41));
        assert_eq!(summary.spans, vec![(42_000, 100_000)]);
        let observed = super::super::stall_onset::observed_intervals(&kept, 2000);
        assert!(observed.iter().any(|&(a, b)| a <= 40_500 && 40_500 <= b));
    }

    #[test]
    fn input_at_the_end_point_drops_nothing() {
        let feat = feat_secs(0, 100);
        let (kept, summary) = idle_trim(&feat, &secs(&[100]), MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    #[test]
    fn idle_run_across_a_short_hole_is_dropped_as_one_span() {
        // 0..=20 秒、15 秒の短い穴 (min_gap 未満)、35..=50 秒、長い穴、200..=210 秒。
        let mut feat = feat_secs(0, 20);
        feat.extend(feat_secs(35, 50));
        feat.extend(feat_secs(200, 210));
        let (kept, summary) = idle_trim(&feat, &secs(&[10, 210]), MIN_GAP, MAX_TRIM);
        let mut expected = feat_secs(0, 10);
        expected.extend(feat_secs(200, 210));
        assert_eq!(kept, expected);
        assert_eq!(summary.dropped_feat_rows, 10 + 16);
        assert_eq!(summary.spans, vec![(11_000, 50_000)]);
    }

    #[test]
    fn no_input_within_max_trim_drops_the_whole_run() {
        // 入力が 1 本も無く、`h − max_trim` が負 → 下限なしで全部落ちる (0 秒の行も)。
        let feat = feat_secs(0, 100);
        let (kept, summary) = idle_trim(&feat, &[], MIN_GAP, MAX_TRIM);
        assert!(kept.is_empty());
        assert_eq!(summary.dropped_feat_rows, 101);
        assert_eq!(summary.spans, vec![(0, 100_000)]);
    }

    #[test]
    fn several_end_points_each_trim_their_own_run() {
        // 2 回寝て 2 回起きた形。入力は各起きている区間の頭だけ。
        let mut feat = feat_secs(0, 50);
        feat.extend(feat_secs(200, 260));
        feat.extend(feat_secs(400, 420));
        let (kept, summary) = idle_trim(&feat, &secs(&[5, 210, 420]), MIN_GAP, MAX_TRIM);
        let mut expected = feat_secs(0, 5);
        expected.extend(feat_secs(200, 210));
        expected.extend(feat_secs(400, 420));
        assert_eq!(kept, expected);
        assert_eq!(summary.spans, vec![(6_000, 50_000), (211_000, 260_000)]);
        assert_eq!(summary.dropped_feat_rows, 45 + 50);
    }

    /// 長い穴をまたいで隣り合って落ちた行は、終わりの点ごとに別の `spans` になる
    /// (1 本 = 1 つの終わりの点で落とした行。`spans` の本数 = 削った回数・code-reviewer M-1)。
    #[test]
    fn drops_on_both_sides_of_a_long_gap_are_separate_spans() {
        // 入力は 30 秒だけ。穴の前 (31..=100 秒) もファイルの最後 (200..=210 秒) も落ち、間に残す行は無い。
        let mut feat = feat_secs(0, 100);
        feat.extend(feat_secs(200, 210));
        let (kept, summary) = idle_trim(&feat, &secs(&[30]), MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat_secs(0, 30));
        assert_eq!(summary.spans, vec![(31_000, 100_000), (200_000, 210_000)]);
        assert_eq!(summary.dropped_feat_rows, 70 + 11);
    }

    /// 入力が 1 本も無く `max_trim_ms == 0` なら、何も落ちない (0 = 前処理なしの保証・code-reviewer L-1)。
    #[test]
    fn no_input_with_max_trim_zero_keeps_every_row() {
        let feat = feat_secs(0, 100);
        let (kept, summary) = idle_trim(&feat, &[], MIN_GAP, 0);
        assert_eq!(kept, feat);
        assert_eq!(summary, IdleTrimSummary::default());
    }

    /// 入力が 1 本も無く `h ≥ max_trim_ms` なら、`h − max_trim_ms` より後だけを落とす。
    #[test]
    fn no_input_trims_back_only_max_trim() {
        let feat = feat_secs(0, 2400);
        let (kept, summary) = idle_trim(&feat, &[], MIN_GAP, MAX_TRIM);
        assert_eq!(kept, feat_secs(0, 600));
        assert_eq!(summary.spans, vec![(601_000, 2_400_000)]);
    }

    #[test]
    fn unsorted_and_duplicated_input_is_normalised() {
        let feat = vec![3000, 1000, 2000, 2000, 0];
        let (kept, summary) = idle_trim(&feat, &[2000, 1000], MIN_GAP, MAX_TRIM);
        assert_eq!(kept, vec![0, 1000, 2000]);
        assert_eq!(summary.spans, vec![(3000, 3000)]);
    }

    #[test]
    fn empty_feat_is_empty() {
        let (kept, summary) = idle_trim(&[], &[1000], MIN_GAP, MAX_TRIM);
        assert!(kept.is_empty());
        assert_eq!(summary, IdleTrimSummary::default());
    }

    /// 2026-10-08 の長い試験 (P998・Surface Pro 8) の形を秒に写したもの。
    /// 最後の打鍵 12:38:09 → 寝る (12:39:08) → 15 秒の穴 (12:54:06 → 12:54:22) → 12:55:19 で長い穴 →
    /// 19:26:44 に復帰して打鍵。前処理後の 1 本目の観測区間は 12:38:09 で終わる。
    #[test]
    fn shape_of_the_2026_10_08_windows_sleep_test() {
        let hms = |h: u64, m: u64, s: u64| (h * 3600 + m * 60 + s) * 1000;
        let mut feat: Vec<u64> = (hms(12, 33, 44)..=hms(12, 54, 6)).step_by(1000).collect();
        feat.extend((hms(12, 54, 22)..=hms(12, 55, 19)).step_by(1000));
        feat.extend((hms(19, 26, 44)..=hms(19, 31, 28)).step_by(1000));
        let input = vec![
            hms(12, 33, 58),
            hms(12, 38, 9),
            hms(19, 27, 0),
            hms(19, 31, 23),
        ];
        let (kept, summary) = idle_trim(&feat, &input, MIN_GAP, MAX_TRIM);
        let observed = super::super::stall_onset::observed_intervals(&kept, 2000);
        assert_eq!(
            observed,
            vec![
                (hms(12, 33, 44), hms(12, 38, 9)),
                (hms(19, 26, 44), hms(19, 31, 23)),
            ]
        );
        assert_eq!(
            summary.spans,
            vec![
                (hms(12, 38, 10), hms(12, 55, 19)),
                (hms(19, 31, 24), hms(19, 31, 28)),
            ]
        );
    }
}
