//! gse-replay — セッション NDJSON リプレイ CLI (Phase 0 P0-4)
//!
//! 既存の NDJSON セッションログを入力に、`analysis::loop_step::step()` を
//! 再実行して `FeatSnapshot` 列を生成する。
//!
//! # 使い方
//!
//! ```text
//! gse-replay <session.ndjson>                        # stdout に JSON 配列を出力
//! gse-replay <session.ndjson> --snapshot <file>      # 既存スナップショットと完全一致比較
//! ```
//!
//! # 設計
//!
//! - 仮想時刻は 1 秒刻みでゲートを発火させつつ、イベントは NDJSON の `t`
//!   タイムスタンプそのものを `step()` に渡す。
//! - `ReplayEvent` は `logger::LogEntry` とは独立した Deserialize 専用 enum。
//!   書き出しと読み込みの責務を分離し、NDJSON のフィールド名 (`t` / `vk` /
//!   `press` 等) に一致させる。
//! - ime_open は NDJSON の `ime_state.on` を追跡して `step()` 引数に渡す。
//! - 出力 JSON は `serde_json::to_string_pretty`（可読性重視、`diff` 容易）。
//!
//! # 関連
//!
//! - `docs/architecture.md` §4.7 沈黙 3 状態機構
//! - `.steering/20260414-improvement-roadmap/design.md` §0.2

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gse_next_lib::analysis::anonymize::KeyCategory;
use gse_next_lib::analysis::engine::{CognitiveStateEngine, HMM_VERSION};
use gse_next_lib::analysis::feat_offline::{ImeShadowEvent, Pole, PoleInputs};
use gse_next_lib::analysis::features::InputEvent;
use gse_next_lib::analysis::heads::TaskDomain;
use gse_next_lib::analysis::loop_step::{
    step, AnalysisState, FeatSnapshot, LoopInput, ENGINE_TICK_MS,
};
use gse_next_lib::analysis::metrics::KeyPress;
use gse_next_lib::logger::TaskKind;

/// 入力 NDJSON のサイズ上限 (500 MB)。
///
/// `BufReader::lines()` + `Vec<ReplayEvent>` 蓄積で読むため、
/// 上限なしだとメモリ枯渇で OOM する可能性がある (security-checker MEDIUM 指摘)。
/// 10 分セッションの NDJSON は通常 数 MB なので 500 MB は十分広いバッファ。
const MAX_SESSION_BYTES: u64 = 500 * 1024 * 1024;

/// `--features` が許す tick 数の上限 (= 1 Hz で約 11.5 日ぶん)。
///
/// ⚠️★★★ **`MAX_SESSION_BYTES` では防げない**別種の暴走を止めるための上限である。
/// tick 数は**入力のバイト数ではなく時刻スパンに比例する**ので、
/// ★ 実測 (2026-09-15): **251 バイト・2 行**の NDJSON に `t=0` と `t=1e8` を並べるだけで
/// **100,001 行**が出た。epoch 混入・壁時計の前方跳躍 (**ADR-030 D9**)・セッション連結の
/// いずれでも発火しうる。⚠️ ギャップを黙って潰さず `Err` で止める (D9 の危険側に落とさない)。
const MAX_FEATURE_TICKS: usize = 1_000_000;

/// `--features` (issue 007・`Q`) が許す `meta.capabilities` の直列化後サイズの上限 (バイト)。
///
/// ⚠️★★★ 2026-09-18 (判断 24・security-checker LOW-3): JSON オブジェクトでない、または
/// この上限を超えるものは「宣言が壊れている」とみなし `null` に倒さず `Err` にする
/// (`null` に倒すと「宣言が無い」と誤報になる)。
///
/// ★ 2026-09-20 (封筒・判断 3): 導入時の理由は「`capabilities` が tick 数ぶん全行へ複製され、
/// 出力 JSON が複製ぶん膨らむ」ことだった。⚠️ **その理由は封筒への移動で消えた**
/// (値は 1 回しか出ない)。★ それでも上限は残す —— **被験者が編集しうる入力**
/// (O1 の NDJSON) を無制限に読み込まないため。
const MAX_CAPABILITIES_BYTES: usize = 4096;

/// `--features` の封筒が `meta` から複写する**文字列**フィールドの上限 (バイト)。
///
/// ⚠️★★★ 2026-09-20 追加 (security-checker M1)。★ 対象は `hmm_version` / `app_version` /
/// `g1_mode` / `o1_mode` / `keyboard_layout`。
/// ⚠️ 旧版は長さも文字種も無制限に複写していた —— ★ 同じ `meta` 行の `capabilities` には
/// [`MAX_CAPABILITIES_BYTES`] があるのに、こちらには何も無かった。
///
/// ★ 実際に書かれる値は semver (`"0.5.0"`) / enum (`"g1"`) / レイアウト識別子で、
/// いずれも数十バイト。256 は十分広い。
///
/// ⚠️★★★ **超過は `Err`（`--features` は exit 1）。`None` に倒さない。**
/// ★ 倒すと出力だけを見て「未宣言」と「壊れた長大値」が区別できなくなる
/// (2026-09-20・Codex `/cross-review`)。`capabilities` と同じ規律である ——
/// **「無い」は `None` / 「壊れている」は `Err`**。
const MAX_META_STRING_BYTES: usize = 256;

/// `provenance.source_sha256_prefix` に出す 16 進の桁数 (= 8 バイトぶん)。
///
/// ⚠️★★★ **フィールド名に `_prefix` を付けている** (2026-09-20・Codex `/cross-review` 2 巡目) ——
/// ★ `source_sha256` だと**完全な SHA-256 が載っている**と読まれる。載っているのは先頭だけである。
///
/// ★ 目的は「同じ入力かどうか」の同定であって暗号学的な衝突耐性ではない。
/// 64 桁を丸ごと出しても人が読めないので先頭だけを出す。
const SOURCE_SHA256_HEX_CHARS: usize = 16;

/// `--features` 出力の封筒 (起票 2・判断 25 #14 逐語 `{ provenance, config, rows }`)。
///
/// ⚠️★★★ **2026-09-20 以前はトップレベルが `FeatureRow` の配列だった。**
/// ★ 配列のままだと**複数セッションの出力を結合したとき、どの行がどの計器条件で
/// 取れたか失われる** (起票 2)。封筒はその 1 点を直す。
///
/// ★ 封筒は **`bin/replay.rs` が組み立てる** —— 中身は `meta` 行 / 入力ファイル /
/// CLI の設定値であり、どれも「セッションの計算」ではなく「実行の由来」だからである
/// (`analysis/feat_offline/` は OS 非依存の計算層で、ファイルパスも CLI 引数も知らない)。
#[derive(Debug, serde::Serialize)]
struct FeatureEnvelope<'a> {
    provenance: FeatureProvenance,
    config: FeatureConfigDto,
    rows: &'a [gse_next_lib::analysis::feat_offline::FeatureRow],
}

/// 「この出力を共有してはいけない」ことと、**なぜ共有できないか**を出力自身に刻む
/// (起票 5・判断 5)。
///
/// ⚠️★★★ **規約は前から在ったが、機械的な門が無かった** —— `docs/glossary.md` の
/// 「特徴行」に逐語「中間成果物であり `gse-dataset` へは共有しない」と書いてあり、
/// `--features` の usage にも書いたが、**検査器が拾える鍵がどこにも無かった**。
/// ★ `.gitignore` に慣用ファイル名を足すのは**名前依存なので代替にならない**
/// (起票 5 の逐語)。→ 中身に鍵を置く。
///
/// ★ `scripts/check_no_feature_output.sh` がこのマーカーを探す。
#[derive(Debug, serde::Serialize)]
struct SharingMarker {
    /// ★ 固定値 `"prohibited"`。⚠️ **真偽値にしない** —— `false` が書ける形は
    /// 「共有してよい `--features` 出力がありうる」と読まれる。そういう出力は無い。
    gse_dataset: &'static str,
    /// ⚠️ `t_ms` を含む全行が `anonymize.rs` を**一度も通っていない**という事実。
    anonymized: bool,
    /// ⚠️ `t_ms` が**絶対 epoch ms** であるという事実 (セッション相対ではない)。
    absolute_epoch_ms: bool,
    /// 規約の在処 (逐語ではなく参照。★ 文言の SSOT は `docs/glossary.md` 側)。
    rule: &'static str,
}

impl Default for SharingMarker {
    fn default() -> Self {
        Self {
            gse_dataset: "prohibited",
            anonymized: false,
            absolute_epoch_ms: true,
            rule: "docs/glossary.md 特徴行",
        }
    }
}

/// 封筒の `provenance` —— 「どの計器条件で・どの入力から・何が出したか」。
///
/// ⚠️★★★ **載せられるのは NDJSON から読めるものだけである。**
/// ★ 起票 2 は「OS」も挙げていたが、`meta` 行に `os` は**無い** (`logger.rs::write_meta_start`
/// を 2026-09-20 に実測)。それを足すのは**計器の変更** (起票 1・ADR-032 の領域) であり、
/// ここで推測して埋めない。
#[derive(Debug, serde::Serialize)]
struct FeatureProvenance {
    sharing: SharingMarker,
    /// ★ 固定値。どのツールが出したか。
    tool: &'static str,
    /// このバイナリの `CARGO_PKG_VERSION`。
    ///
    /// ⚠️★★★ **生成時刻 (`generated_at`) は載せない** (判断 7) —— 載せると
    /// 同じ入力から同じ出力が出なくなり、「出力が変わらないこと」を検査できなくなる
    /// (`reproducibility-discipline` と衝突する)。いつ生成したかは mtime で足りる。
    tool_version: &'static str,
    /// 入力 NDJSON のファイル名 (⚠️ **ディレクトリは含めない**)。
    ///
    /// ⚠️ 実セッションでは参加者 ID (`..._P003.ndjson`) を含みうる。★ それを承知で
    /// 載せる (判断 4) —— 入力は研究者の手元にあるファイルの名前であり、
    /// **新しい漏洩先を作らない**。一方 `rows` には個人を混ぜないという規律は狭めない。
    source_basename: String,
    /// 入力 NDJSON の SHA-256 先頭 [`SOURCE_SHA256_HEX_CHARS`] 桁。
    /// ★ 名前が変わっても同じ入力を同定するため。
    source_sha256_prefix: String,
    /// 以下は `meta`(session_start) 行から読む。★ 行が無い / キーが無いなら `null`。
    /// ⚠️ **欠けていても `--features` は落とさない** (旧ログ互換。`hook_health` と同じ作法)。
    session_start: Option<u64>,
    /// ⚠️ **収集時**の `HMM_VERSION`。★ `config.hmm_version` (**解析時** = このバイナリ) とは
    /// 意味が違う。古いログを新しいバイナリで読むと食い違い、**それが読めること自体が目的**。
    hmm_version: Option<String>,
    app_version: Option<String>,
    g1_mode: Option<String>,
    o1_mode: Option<String>,
    keyboard_layout: Option<String>,
    /// `meta` 行の `capabilities` を加工せずそのまま。宣言が無ければ `null`
    /// (⚠️ 空配列・空オブジェクトで埋めない)。
    ///
    /// ⚠️★★★ **2026-09-20 (判断 3) に `rows[].quality` から移した。**
    /// ★ セッション不変の事実を行ごとに複製する意味が無いため。
    /// ⚠️ 「宣言が無い」という事実は tick 単位の欠測理由として意味を持つので、
    /// `rows[].quality.missing_reasons` の `"capability_undeclared"` は**そのまま残る**。
    capabilities: Option<serde_json::Value>,
    /// 観測区間の列 (ADR-030 D8 (b-2))。`feat` 行の**存在**だけから決まる
    /// (`stall_onset::observed_intervals`)。`feat` 行が 1 本も無ければ**空配列**
    /// (⚠️ `null` にしない —— 「観測区間が 1 つも無い」は観測できた事実である)。
    /// ★ 2026-10-08 (事前宣言 v1.2・ADR-030 補訂 4): 使う `feat` 行は、長い穴とファイルの最後の直前の
    /// 無入力の行を落とした**後の列**である (`idle_trim`・下の [`Self::idle_trim`])。
    ///
    /// ⚠️★★★ **極によって変わる** (ADR-030 D3・`config.pole`)。
    /// ★ 上の定義は `baseline` (極 A ＝ 何も抑制しない) のもの。極 B (ポインタ活動区間を抑制) では
    /// 抑制区間を除いた **実際に評価した区間**を載せる (`decisions.md` 判断 7・issue 006) —— 起票 20 (a) の
    /// 目的 (打ち切りまでの追跡時間 `b − o`) から見ても、評価していない区間は追跡時間に数えられない。
    /// ⚠️ フィールド名は変えない。**どの極の区間か**は封筒の `config.pole` から読む
    /// (⚠️ 別の極の出力と区間を突き合わせるときは、先に `config.pole` が一致することを確かめる)。
    ///
    /// ★★★ **なぜ封筒に載せるか** (2026-09-21・研究者判断・`blockers.md` 起票 20 (a)):
    /// `restart` (`T1d`) の `restop_ms` が `null` の行は**右側打ち切り**だが、
    /// ⚠️ **打ち切りまでの追跡時間 (`b − o`) が行からは読めない** —— `b` が出力に無いため。
    /// 生存解析 (`ADR-031:52` 逐語「主 criterion: … **再停止までの時間** …」) は
    /// `(時間, 事象)` の組を要するので、打ち切り行を捨てるしかなくなっていた。
    /// ★ 区間はセッション不変なので**封筒が 1 回だけ持つ**のが正しい (⚠️ 行ごとに複製しない ——
    /// `capabilities` を行から封筒へ移した判断 3 と同じ理屈)。
    ///
    /// ⚠️★★★ **新しい露出ではない。** 穴の位置と長さは
    /// `rows[].quality.observability_mask.feat` が**既に tick ごとに出している** (issue 007)。
    /// ★ 削除 / 一時停止 / 計器欠損の**原因ラベルは 1 つも無い**まま (ADR-030 D8 (b-2))。
    ///
    /// ⚠️★★★ **極 B ではこの列自体の意味が変わる** (security-checker LOW・issue 006)。
    /// baseline (極 A) では「`feat` 行の穴」(★ 2026-10-08 から: 無入力の末尾の前処理の後の穴。
    /// 前処理は**全極で** `key` / `pointer_bin` 行の時刻を読むので、baseline でも長い穴とファイルの最後ごとに
    /// 最後の入力の秒が 1 つ右端に出る・下の [`IdleTrimDto`]) を表すが、極 B では
    /// **「観測区間 − ポインタ活動区間」= 評価区間**を表す。★ そのため、極 B の
    /// `observed_intervals` から baseline の `observed_intervals` を差し引くと、
    /// **ポインタ活動があった秒の時系列** (1 秒ビン単位・活動の**存在のみ**、
    /// `moved` / `buttons` / `wheel` の値は含まない) を読み取れる —— ゼロ抑制された
    /// `pointer_bin` 行そのものは封筒に出していないが、極 B と baseline の区間差分という
    /// **間接的な形**で活動の有無と大まかなタイミングが漏れる。★ 欄は分けない (issue 006
    /// の裁定): `pole == "pointer_b"` の出力を baseline と突き合わせて読む側の責任とする。
    observed_intervals: Vec<ObservedIntervalDto>,
    /// 観測区間を作る前に落とした `feat` 行の量 (事前宣言 v1.2 §6.1・`analysis::feat_offline::idle_trim`)。
    /// ★ 極によらない (前処理は極の前に 1 回だけ掛かる)。`config.idle_trim_max_ms == 0` なら 0 行・空配列。
    idle_trim: IdleTrimDto,
    /// 長い無入力の前処理で落とした `feat` 行の量 (事前宣言 v1.3 §7.1・`idle_trim::input_gap_trim`)。
    /// ★ `idle_trim` が先に落とした行は数えない (削った行の合計は 2 つの和)。極によらない。
    /// `config.idle_trim_input_gap_ms == 0` なら 0 行・空配列。`spans` は 1 本 = 1 つの無入力の区間。
    /// 露出は [`IdleTrimDto`] と同じ種類 (左端は入力を含む秒の直後・右端は次の入力の秒の直前。
    /// ⚠️ ファイルの最初・最後に接する区間は、その境の行を含む)。加えて次が読める (security-checker LOW・2026-10-11):
    /// - 右端からは (区間の中で `feat` 行が途切れていなければ)、無入力の後の**最初の入力の秒**が読める。解放の行・キーを解決できない行・`pointer_bin` 行なら、
    ///   `rows[]` の `activity` には出ない秒である。
    /// - 「その区間に `key` も `pointer_bin` も `idle_trim_input_gap_ms` 以上無かった」という不在。24 時間のファイルでは、
    ///   `spans` の列がそのまま 1 日の入力の有無の時刻表 (長い不在の時刻) として読める。
    ///
    /// ⚠️ 落とした行は `rows[]` から消えるので、封筒全体から読める量はむしろ減る。封筒は `gse-dataset` へ共有しない。
    input_gap_trim: IdleTrimDto,
}

/// [`FeatureProvenance::idle_trim`] と [`FeatureProvenance::input_gap_trim`]。
///
/// 落とした範囲は `observed_intervals` の穴と `rows[].quality.observability_mask.feat` からも読める。
/// 範囲の左端は「最後の入力を含む秒の直後」なので、長い穴とファイルの最後ごとに次が baseline の出力にも出る
/// (security-checker LOW・2026-10-09 に訂正):
/// - 最後の入力が、キーを解決できた**押下**なら `rows[]` の `activity` から既に読める秒である。
///   ⚠️ **解放**の行・キーを解決できない行なら、`activity` には出ない秒 (押下から 1 tick 以内のずれ) が新しく出る。
/// - ⚠️ `pointer_bin` 行なら、**長い穴の直前の最後のポインタ活動の秒**が出る (存在だけ・`moved` などの値は出ない。
///   極 B の `observed_intervals` の差分と同じ種類の間接的な露出)。
/// - 30 分の上限で切られたときは、逆に「その前の 30 分は `key` も `pointer_bin` も無かった」という不在が読める。
///
/// 原因のラベルは無い (ADR-030 D8 (b-2))。封筒は `gse-dataset` へ共有しない (`sharing`)。
#[derive(Debug, serde::Serialize)]
struct IdleTrimDto {
    /// 落とした `feat` 行の数。
    dropped_feat_rows: u64,
    /// 落とした行のつながり (`[a_ms, b_ms]`・両端は落とした最初と最後の `feat` 行の時刻)。
    /// ★ 1 本 = 1 つの終わりの点 (長い穴の直前・ファイルの最後) で落とした行。本数 = 行を落とした終わりの点の数。
    /// ⚠️ 短い穴をまたぐことがあるので、`b_ms − a_ms` を削った時間として読まない (それは `dropped_feat_rows`)。
    spans: Vec<ObservedIntervalDto>,
}

/// 観測区間 1 つ (`[a_ms, b_ms]`・**両端を含む**)。
///
/// ⚠️ 右端 `b_ms` は**確定扱い**である (`stall_onset` の `t0 <= b` と対称)。
#[derive(Debug, serde::Serialize)]
struct ObservedIntervalDto {
    a_ms: u64,
    b_ms: u64,
}

/// 封筒の `config` —— この出力を再現するのに要る設定値。
///
/// ★ 判断 25 #13 (`W` / `q` / `B` / `hole_gap_ms` / `warmup_count` / 分位点の推定法 /
/// `T1a` の窓 を P1 前に 1 本の宣言文書で凍結し、**同時に CLI 注入を作る**) の**受け皿**である。
/// ★ 値は既定で**凍結 JSON (`prereg/p1-episode.json`)** から来る。`--feature-config` /
/// `--set` で外れた値は `overrides` に**明示的に**残る ([`ResolvedFeatureConfig`])。
///
/// ⚠️★★★ **θ_person の分位点はここに入れない** —— θ は**設定ではなく結果**
/// (ADR-008 D4 の `within_session_adaptive`) であり、行の `stall_onset.theta_person_ms` が持つ。
/// 設定と結果を同じ箱に入れない。
#[derive(Debug, serde::Serialize)]
struct FeatureConfigDto {
    /// 条件 A の因果窓長 (ms・ADR-030 D4)。
    w_ms: u64,
    /// θ_person の分位点 (ADR-030 D4)。
    q: f64,
    /// `t0` 直後の buffer (ms・ADR-030 D2)。
    b_ms: u64,
    /// 条件 A の産出判定閾値 (ADR-030 D3)。
    net_diff_min: u64,
    /// 観測区間を切る閾値 (ms・ADR-030 D8 (b-2))。
    hole_gap_ms: u64,
    /// θ の標本数の下限 (ADR-030 D4 点 1)。
    /// ★ 凍結 JSON 経路では常に数値 (`validate` が 0 を拒否する)。`null` は旧出力 (下限なし) との互換のため型に残す。
    warmup_count: Option<usize>,
    /// θ_person の分位点の推定法 (P1 事前宣言 §1・現状 `"nearest_rank"` だけが実装されている)。
    theta_estimator: String,
    /// `T1a` の窓長 (ms・P1 事前宣言 §1)。★ 窓の実装は issue 003。ここは値の記録だけ。
    t1a_window_ms: u64,
    /// 前処理の「長い穴」とみなす `feat` 行の間隔の下限 (ms・P1 事前宣言 v1.2 §6.2)。
    idle_trim_min_gap_ms: u64,
    /// 前処理で戻す幅の上限 (ms・P1 事前宣言 v1.2 §6.2)。`0` は前処理なしの実行 (§6.4)。
    idle_trim_max_ms: u64,
    /// 長い無入力の前処理で、落とす無入力の長さの下限 (ms・P1 事前宣言 v1.3 §7.2)。`0` は前処理なし (§7.4)。
    idle_trim_input_gap_ms: u64,
    /// この出力の**極** (ADR-030 D3・[`Pole::as_str`])。★ 1 実行 1 極で、極を持つのは**封筒だけ**
    /// (行には載せない・判断 3)。⚠️ 上の設定 11 値とは別の軸で、`--set` では変えられない (`--pole` だけ)。
    pole: &'static str,
    /// この入力で、上の極が**計算できたか**。★ 常に出る (`baseline` は常に `true`)。
    /// ⚠️ 極 B / 極 M+ の判定は**入力に依存する** (`capabilities` の宣言 / 修飾キー行の有無)。
    /// ★ 極 M+ は**入力ファイル (セッション) 単位**で、解決後の `KeyCategory` が `Modifier` の `key` 行が
    /// 1 本も無ければ `false` (`decisions.md` 判断 13)。極 B の判定は issue 006 が実装する
    /// (★ それまで極 B は `--pole` が exit 1 で止める)。
    ///
    /// ⚠️★★★ **`false` のとき、行の `stall_onset` / `restart` は baseline と恒等である** —— 修飾キーの行が 0 本なら
    /// 押下区間が 1 つも無く、極 M+ は構造的に極 M− と同じ値になる。★ 値で埋めない (別の値を出さない)。
    pole_applicable: bool,
    /// `pole_applicable == false` の理由コード。★ 適用可のとき `null`。⚠️ 極ごとに判定の根拠が違うので、
    /// 理由の語も別にする (`no_pointer_capability` / `no_pointer_rows` = 極 B・issue 006・
    /// `decisions.md` 判断 26 / `no_modifier_rows` = 極 M+)。
    ///
    /// ⚠️★★★ `no_modifier_rows` は「**計器が無かった**」(2026-08-23 より前のログ・macOS) と
    /// 「**押さなかった**」を**区別しない** (`meta` に `os` が無く、`capabilities` に修飾キーの計器を表す項目も無い)。
    /// ★ 過剰拒否だが**情報は失わない** (行が 0 本なら極 M+ = 極 M− で、上下界の幅への寄与がゼロ)。
    /// ⚠️★★★ `no_pointer_rows` も同型 —— `capabilities.rs:244-245` 逐語「0 行は判定不能」のとおり、
    /// 宣言はコンパイル時の `POINTER_AVAILABLE` でしかなく、実行時の `observe::start` は
    /// 設置に失敗しうる。「活動が無かった」と「設置に失敗した」を区別できないので、
    /// 0 行を所見として `true` にはしない (`decisions.md` 判断 26・極 M+ の判断 13 と揃えた)。
    /// ⚠️ `pointer_evidence_not_supplied`（極 B）／`modifier_evidence_not_supplied`（極 M+）は
    /// **防御の値**であり、実際には出ない (未実装の極は出力の前に止まる。呼び出し側が証拠を
    /// 渡し忘れたときだけ出る。★ 旧称 `not_implemented` は「未実装」と誤読されるため改名した
    /// — code-reviewer LOW)。
    pole_not_applicable_reason: Option<&'static str>,
    /// 極 M+ の判定に使った入力ファイルの**修飾キーの `key` 行数** (`decisions.md` 判断 13 逐語
    /// 「封筒に理由コード (`no_modifier_rows`) と `modifier` 行数を刻み」)。★ 極 M+ 以外では数えないので `null`。
    /// ⚠️ 0 のとき「計器が無かった」と「押さなかった」は区別できない (上の理由コードの doc)。
    pole_modifier_rows: Option<u64>,
    /// 極 M+ の区間の健全性: 観測区間の右端で**打ち切られた**押下区間の本数
    /// (`decisions.md` 判断 24・`stall_onset::ModifierIntervals::truncated_at_right_edge`)。
    /// ★ 極 M+ 以外では数えないので `null`。⚠️ 値の出所は既存の件数 (`collect_modifier_intervals`
    /// が `tracing::warn!` へ出しているのと同じもの) —— warn はそのまま残す。
    /// ⚠️ 0 より大きいと、その本数ぶん区間が本来より短く（＝除外が少なく）数えられている
    /// (`pole_applicable=true` のまま出力は見た目正常だが、内部で打ち切りが起きている)。
    pole_modifier_truncated: Option<u64>,
    /// 極 M+ の区間の健全性: 先行する press が無い release（孤立）の件数
    /// (`decisions.md` 判断 24・`stall_onset::ModifierIntervals::orphan_releases`)。
    /// ★ 極 M+ 以外では数えないので `null`。
    pole_modifier_orphan_releases: Option<u64>,
    /// 極 B の判定に使った入力ファイルの **`pointer_bin` 行数**。★ 極 B 以外では数えないので `null`。
    /// ⚠️★★★ **宣言があっても行が 0 本なら `pole_applicable == false`／理由 `no_pointer_rows`**
    /// (`decisions.md` 判断 26・2026-09-25 是正) —— `capabilities.rs:244-245` 逐語「0 行は判定不能」の
    /// とおり、宣言はコンパイル時の `POINTER_AVAILABLE` でしかなく、実行時の `observe::start` は
    /// 設置に失敗しうる。「活動が無かった」と「設置に失敗した」を**区別できない**ので、0 行を
    /// 所見として `true` にはしない (★ 極 M+ の判断 13 と揃えた)。
    /// ★ 宣言が無い入力 (`no_pointer_capability`) で 0 より大きければ、宣言と行が食い違う壊れたログである
    /// (⚠️ その場合も抑制は**しない** —— 適用不可の出力は baseline と恒等)。
    pole_pointer_rows: Option<u64>,
    /// 採用した事前宣言の `id` / `version` / 内容の SHA-256 先頭桁 (★ 何を読んだかの同定)。
    prereg_id: String,
    prereg_version: String,
    prereg_sha256_prefix: String,
    /// `"frozen"` (埋め込みの凍結 JSON) / `"file"` (`--feature-config`)。
    config_source: &'static str,
    /// 凍結値から**外れた**キーだけ (無指定なら空配列)。⚠️ 外れていない `--set` は載せない。
    overrides: Vec<ConfigOverride>,
    /// [`MAX_FEATURE_TICKS`]。★ この実行に効いていた上限値を記録する。
    ///
    /// ⚠️ **「上限で止められたか」はこれ単独では判別できない** (2026-09-20・code-reviewer LOW)
    /// —— 上限に当たった場合は `--features` が `Err` で **exit 1** し、そもそも出力が無い。
    /// ★ 出力がある以上 `rows.len() <= max_feature_ticks` であり、
    /// ちょうど `max_feature_ticks` 行のセッションと切り捨てを区別する必要は生じない。
    max_feature_ticks: usize,
    /// ⚠️ **解析時** (このバイナリ) の `HMM_VERSION`。`provenance.hmm_version` (収集時) と別物。
    hmm_version: &'static str,
    /// tick 幅 (`loop_step::ENGINE_TICK_MS`)。
    engine_tick_ms: u64,
    /// 「単純 pause」の閾値 (`features::PAUSE_THRESHOLD_MS`)。
    pause_threshold_ms: u64,
}

/// P1 事前宣言の凍結 JSON (`src-tauri/prereg/p1-episode.json`)。**実行ファイルへ埋め込む。**
///
/// ⚠️★★★ 埋め込みが正である (実行時にファイルを探しに行かない)。★ 理由: 出力の
/// `provenance.tool_version` と凍結値が 1 対 1 になる —— 実行時に読むと、同じバイナリが
/// 置かれた場所次第で違う値を使いうる。別の値を使いたいときは `--feature-config` で
/// **明示的に**指す (`config_source == "file"` が出力に刻まれる)。
const FROZEN_P1_EPISODE_JSON: &str = include_str!("../../prereg/p1-episode.json");

/// `--feature-config` で読む外部ファイルの上限 (バイト)。凍結 JSON は 2 KiB 程度。
const MAX_FEATURE_CONFIG_BYTES: u64 = 64 * 1024;

/// `--set` が受け取れるキー (= 凍結 JSON の `operational` の 11 キー・事前宣言 v1.2 で 2 キー、v1.3 で 1 キー足した)。
const P1_OPERATIONAL_KEYS: [&str; 11] = [
    "w_ms",
    "q",
    "b_ms",
    "hole_gap_ms",
    "warmup_count",
    "theta_estimator",
    "t1a_window_ms",
    "net_diff_min",
    "idle_trim_min_gap_ms",
    "idle_trim_max_ms",
    "idle_trim_input_gap_ms",
];

/// `idle_trim_max_ms` / `idle_trim_min_gap_ms` / `idle_trim_input_gap_ms` の上限 (24 時間・事前宣言 v1.2 §6.1・v1.3 §7.1)。
/// ⚠️ 自己 DoS の門ではなく、**24 時間を超える値 (明らかな桁の誤り) だけ**を止める。凍結値 60000 の
/// 1440 倍未満の誤り (例: 1000 倍の 60000000 = 16.7 時間) は止めない —— その値は封筒の `overrides` に残る
/// (研究者裁定 2026-10-09・上限を締めない)。
const IDLE_TRIM_LIMIT_MS: u64 = 24 * 60 * 60 * 1000;

/// 凍結 JSON の `operational` (P1 事前宣言 §1 の 8 値 ＋ v1.2 §6.2 の 2 値 ＋ v1.3 §7.2 の 1 値)。
///
/// ⚠️ `deny_unknown_fields` —— 未知のキーを黙って捨てると、綴りを間違えた設定が
/// 「効いたつもり」で走る (fail-closed)。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct P1Operational {
    w_ms: u64,
    q: f64,
    b_ms: u64,
    hole_gap_ms: u64,
    warmup_count: u64,
    theta_estimator: String,
    t1a_window_ms: u64,
    net_diff_min: u64,
    /// ⚠️ 必須 (`default` を付けない) —— 1.0.0 の形の `--feature-config` は前処理の値を持たないので拒否する
    /// (前処理を黙って無効にした出力を作らない)。
    idle_trim_min_gap_ms: u64,
    idle_trim_max_ms: u64,
    /// ⚠️ 必須 —— 1.1.0 の形の `--feature-config` は長い無入力の前処理の値を持たないので拒否する。
    idle_trim_input_gap_ms: u64,
}

/// 凍結 JSON のうち本ツールが読む部分 (`status` / `search_*` などは読まない)。
#[derive(Debug, serde::Deserialize)]
struct P1PreregFile {
    id: String,
    version: String,
    operational: P1Operational,
}

/// 凍結値から外れたキー 1 つ (`--feature-config` / `--set` の結果)。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct ConfigOverride {
    key: &'static str,
    /// 凍結 JSON の値。
    frozen: serde_json::Value,
    /// この実行で使った値。
    value: serde_json::Value,
}

impl P1Operational {
    /// `(キー, JSON 値)` の列 (`P1_OPERATIONAL_KEYS` と同じ順)。overrides の比較に使う。
    fn as_pairs(&self) -> [(&'static str, serde_json::Value); 11] {
        [
            ("w_ms", self.w_ms.into()),
            ("q", self.q.into()),
            ("b_ms", self.b_ms.into()),
            ("hole_gap_ms", self.hole_gap_ms.into()),
            ("warmup_count", self.warmup_count.into()),
            ("theta_estimator", self.theta_estimator.clone().into()),
            ("t1a_window_ms", self.t1a_window_ms.into()),
            ("net_diff_min", self.net_diff_min.into()),
            ("idle_trim_min_gap_ms", self.idle_trim_min_gap_ms.into()),
            ("idle_trim_max_ms", self.idle_trim_max_ms.into()),
            ("idle_trim_input_gap_ms", self.idle_trim_input_gap_ms.into()),
        ]
    }

    /// `--set <key>=<value>` を 1 件適用する。⚠️ 型が合わない値・未知のキーは `Err`
    /// (黙って無視しない)。範囲の検査は全件を適用した後の [`Self::validate`] が行う。
    fn apply_set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let as_u64 = |v: &str| {
            v.parse::<u64>()
                .map_err(|_| format!("--set {key}: `{v}` は非負の整数ではない"))
        };
        match key {
            "w_ms" => self.w_ms = as_u64(value)?,
            "q" => {
                self.q = value
                    .parse::<f64>()
                    .map_err(|_| format!("--set q: `{value}` は数値ではない"))?
            }
            "b_ms" => self.b_ms = as_u64(value)?,
            "hole_gap_ms" => self.hole_gap_ms = as_u64(value)?,
            "warmup_count" => self.warmup_count = as_u64(value)?,
            "theta_estimator" => self.theta_estimator = value.to_string(),
            "t1a_window_ms" => self.t1a_window_ms = as_u64(value)?,
            "net_diff_min" => self.net_diff_min = as_u64(value)?,
            "idle_trim_min_gap_ms" => self.idle_trim_min_gap_ms = as_u64(value)?,
            "idle_trim_max_ms" => self.idle_trim_max_ms = as_u64(value)?,
            "idle_trim_input_gap_ms" => self.idle_trim_input_gap_ms = as_u64(value)?,
            _ => {
                return Err(format!(
                    "--set: 未知のキー `{key}` (許可: {})",
                    P1_OPERATIONAL_KEYS.join(", ")
                ))
            }
        }
        Ok(())
    }

    /// 物理的な範囲の検査。★ 通らない値を封筒に刻んだ出力は、下流が「計算できた」と誤読する。
    fn validate(&self) -> Result<(), String> {
        if self.w_ms == 0 {
            return Err("w_ms は 1 以上でなければならない".to_string());
        }
        if !(self.q.is_finite() && self.q > 0.0 && self.q < 1.0) {
            return Err(format!(
                "q は 0 より大きく 1 より小さくなければならない (実際: {})",
                self.q
            ));
        }
        if self.hole_gap_ms == 0 {
            return Err("hole_gap_ms は 1 以上でなければならない".to_string());
        }
        if self.warmup_count == 0 || usize::try_from(self.warmup_count).is_err() {
            return Err(
                "warmup_count は 1 以上で、この環境の usize に収まらなければならない".to_string(),
            );
        }
        if self.t1a_window_ms == 0 {
            return Err("t1a_window_ms は 1 以上でなければならない".to_string());
        }
        // ⚠️ `dispersion::pause_ms_by_window` は窓内の pause を tick ごとに具体化する
        // (`Vec<Vec<u64>>`) ので、窓を無制限に広げるとメモリが窓幅に比例して膨らむ
        // (自己 DoS 防止・security-checker LOW-1・2026-09-25)。事前宣言 (`p1-episode.json`
        // `search_space.t1a_window_ms`) の探索空間の最大は 60000ms なので、それに十分な
        // 安全マージンを取った 1 時間 (3_600_000ms) を運用上の上限に置く。
        if self.t1a_window_ms > 3_600_000 {
            return Err("t1a_window_ms は 3600000 (1 時間) 以下でなければならない".to_string());
        }
        // ⚠️ `stall_onset.rs` は `net_diff_min as i64` で比べる。`i64::MAX` を超える値は負へ丸まり、
        //   `net < net_diff_min` が常に偽になって **onset が黙って 0 本**になる (security-checker LOW・2026-09-25)。
        if self.net_diff_min == 0 || i64::try_from(self.net_diff_min).is_err() {
            return Err("net_diff_min は 1 以上で、i64 に収まらなければならない".to_string());
        }
        // ⚠️ 穴にならない間隔 (`hole_gap_ms` 以下) の前で削ると、観測区間の内側の沈黙まで落とす
        //   (事前宣言 v1.2 §6.1 の検査)。
        if self.idle_trim_min_gap_ms <= self.hole_gap_ms {
            return Err(format!(
                "idle_trim_min_gap_ms ({}) は hole_gap_ms ({}) より大きくなければならない",
                self.idle_trim_min_gap_ms, self.hole_gap_ms
            ));
        }
        if self.idle_trim_max_ms > IDLE_TRIM_LIMIT_MS {
            return Err(format!(
                "idle_trim_max_ms は {IDLE_TRIM_LIMIT_MS} (24 時間) 以下でなければならない"
            ));
        }
        // ⚠️ `idle_trim_min_gap_ms` が大きすぎると長い穴の前の削りが効かなくなり、ファイルの最後だけが削られる
        //   (code-reviewer L-7 / security-checker L3)。`idle_trim_max_ms` と同じ上限で、24 時間を超える値だけを止める
        //   (それ未満の誤りは `overrides` に残る・[`IDLE_TRIM_LIMIT_MS`])。
        if self.idle_trim_min_gap_ms > IDLE_TRIM_LIMIT_MS {
            return Err(format!(
                "idle_trim_min_gap_ms は {IDLE_TRIM_LIMIT_MS} (24 時間) 以下でなければならない"
            ));
        }
        // ⚠️ 長い無入力の前処理 (事前宣言 v1.3 §7.1)。`0` は前処理なし。それ以外は「長い穴」
        //   (`idle_trim_min_gap_ms`) 以上 —— それより短い無入力を落とすと、P1 が見たい停滞 (`W` の桁) そのものを削る。
        //   上限は他の前処理の値と同じ 24 時間 (それを超える桁の誤りだけを止める)。
        if self.idle_trim_input_gap_ms != 0
            && (self.idle_trim_input_gap_ms < self.idle_trim_min_gap_ms
                || self.idle_trim_input_gap_ms > IDLE_TRIM_LIMIT_MS)
        {
            return Err(format!(
                "idle_trim_input_gap_ms ({}) は 0 か、idle_trim_min_gap_ms ({}) 以上 {IDLE_TRIM_LIMIT_MS} (24 時間) 以下でなければならない",
                self.idle_trim_input_gap_ms, self.idle_trim_min_gap_ms
            ));
        }
        // ⚠️ 実装済みの推定法だけを通す。未実装の名前を封筒に刻むと、出力が嘘をつく。
        if self.theta_estimator != "nearest_rank" {
            // ⚠️ 外部ファイル由来の文字列は端末へそのまま出さない（`echo_untrusted`: 80 字で切り詰め・`{:?}` でエスケープ）。
            return Err(format!(
                "theta_estimator は \"nearest_rank\" だけが実装されている (実際: {})",
                echo_untrusted(&self.theta_estimator)
            ));
        }
        Ok(())
    }
}

/// 解決済みの `--features` 設定 (凍結値 → `--feature-config` → `--set` の順に重ねた結果)。
#[derive(Debug, Clone)]
struct ResolvedFeatureConfig {
    operational: P1Operational,
    prereg_id: String,
    prereg_version: String,
    prereg_sha256_prefix: String,
    config_source: &'static str,
    overrides: Vec<ConfigOverride>,
    /// ★ 極 (ADR-030 D3)。⚠️ `operational` の 11 値とは**別の軸**で、`--set` / `--feature-config` では
    /// 変えられない (凍結 JSON に極は無い)。`resolve_feature_config` は常に `Baseline` で返し、
    /// `--pole` の値は [`Self::with_pole`] で足す。
    pole: Pole,
}

impl ResolvedFeatureConfig {
    /// 極を差し替える。⚠️ 呼ぶ前に [`resolve_pole`] を通すこと (未実装の極を持ち込まない)。
    fn with_pole(mut self, pole: Pole) -> Self {
        self.pole = pole;
        self
    }

    /// 検出器へ渡す設定へ写す。⚠️ 呼ぶ前に `validate` 済みであること (`resolve_feature_config` が保証)。
    fn stall_onset_config(&self) -> gse_next_lib::analysis::feat_offline::StallOnsetConfig {
        let o = &self.operational;
        gse_next_lib::analysis::feat_offline::StallOnsetConfig {
            w_ms: o.w_ms,
            q: o.q,
            b_ms: o.b_ms,
            net_diff_min: o.net_diff_min,
            hole_gap_ms: o.hole_gap_ms,
            // `validate` が usize に収まることを確かめている。
            warmup_count: usize::try_from(o.warmup_count).ok(),
            t1a_window_ms: o.t1a_window_ms,
            pole: self.pole,
        }
    }
}

/// `--pole` の値を極へ解決する。★ 出力を 1 バイトも書く前に呼ぶこと (fail-closed)。
///
/// ⚠️★★★ **止める 3 つ** (いずれも `Err` = 呼び出し側は exit 1・stdout は空):
/// 1. `--features` 以外での指定 —— 他のモードは極を読まない。★ 指定を黙って無視すると
///    「極 B で計算した」つもりの出力が baseline になる。
/// 2. 未知の名前 —— ⚠️ 近い名前へ丸めない。
/// 3. 未実装の極 ([`Pole::is_implemented`]) —— ⚠️ **黙って baseline の値を返さない**
///    (出力が嘘をつき、極の比較が無意味になる)。
///
/// 未指定は `Baseline` (現行の計算そのもの・行の値は `--pole` 導入前と一致する)。
fn resolve_pole(features: bool, raw: Option<&str>) -> Result<Pole, String> {
    let Some(name) = raw else {
        return Ok(Pole::Baseline);
    };
    if !features {
        return Err("--pole requires --features".to_string());
    }
    let pole = Pole::from_name(name).ok_or_else(|| {
        let allowed: Vec<&str> = Pole::ALL.iter().map(|p| p.as_str()).collect();
        // ★ `{:?}` でエスケープする (制御文字・ANSI エスケープで端末表示を乱さない・security-checker LOW)。
        format!("--pole: 未知の極 {name:?} (許可: {})", allowed.join(", "))
    })?;
    if !pole.is_implemented() {
        return Err(format!(
            "--pole {name:?}: この極はまだ実装されていない (baseline の値を別の極の名前で返さないため止める)"
        ));
    }
    Ok(pole)
}

/// 封筒 `config.pole_applicable` / `pole_not_applicable_reason` の値 (`(適用可否, 理由コード)`)。
///
/// ★ 網羅 `match` (ワイルドカード無し) —— 極を足すと、ここで判定を決めるまでコンパイルが通らない。
/// `modifier_rows` は入力ファイルの**修飾キーの `key` 行数** ([`ModifierScan::rows`])。★ 極 M+ だけが読む。
/// `None` は「数えていない」 (⚠️ 「0 本」とは別) —— 極 M+ では**防御の値**で止める。
/// ⚠️ 極 B の判定 (`capabilities` の `pointer_bin`) は**この関数では確定しない** —— 入力 (`capabilities`) が
/// 要るので [`FeatureConfigDto::with_pointer_evidence`] が [`pointer_applicability`] で確定する。
fn pole_applicability(pole: Pole, modifier_rows: Option<u64>) -> (bool, Option<&'static str>) {
    match pole {
        // 極 A / 極 M− はどの入力でも計算できる (現行の計算そのもの)。
        Pole::Baseline => (true, None),
        // ⚠️ 極 B は入力 (`capabilities`) を見るまで**適用可と名乗らない** (防御の値。
        //   `with_pointer_evidence` が確定させる。呼び忘れても嘘の `true` を出さない)。
        //   ★ 旧称 `not_implemented` は「未実装」と誤読される (開通済みのため) —— 実態に
        //   合わせて改名した (code-reviewer LOW)。
        Pole::PointerB => (false, Some("pointer_evidence_not_supplied")),
        // ★ 判定単位は**入力ファイル (セッション)**・判定式は「修飾キーの行が 0 本」(判断 13)。
        Pole::ModifierMPlus => match modifier_rows {
            Some(0) => (false, Some("no_modifier_rows")),
            Some(_) => (true, None),
            // 数えずに極 M+ を名乗らせない (防御。実際には main が必ず数える)。
            // ★ 同じ理由で `not_implemented` → `modifier_evidence_not_supplied` に改名した。
            None => (false, Some("modifier_evidence_not_supplied")),
        },
    }
}

/// 封筒 `provenance.capabilities` に `pointer_bin` の**宣言があるか** (極 B の適用可否の根拠・issue 006)。
///
/// ★ `capabilities` は行種の名前の配列を群 (`unconditional` / `threshold_gated` / `config_gated`) ごとに持つ
/// オブジェクトである。**どの群に載っていても**宣言ありとする (⚠️ 群の名前を再宣言しない)。
/// ⚠️ 宣言が無いのは「**計器が無かった**」であって「活動が無かった」ではない
/// (`schema/current.json` の `pointer_bin`: 「行が無い＝活動が無かった」は宣言がある場合に限る)。
/// `None` (`capabilities` 自体が無い旧ログ) も宣言なし。
fn declares_pointer_bin(capabilities: Option<&serde_json::Value>) -> bool {
    capabilities
        .and_then(serde_json::Value::as_object)
        .is_some_and(|groups| {
            groups
                .values()
                .filter_map(serde_json::Value::as_array)
                .flatten()
                .any(|name| name.as_str() == Some("pointer_bin"))
        })
}

/// 極 B の適用可否 (`(適用可否, 理由コード)`)。★ 判定式は 3 状態 (`decisions.md` 判断 26・2026-09-25 是正):
///
/// 1. `capabilities` に `pointer_bin` の宣言が**無い** → 適用不可・`no_pointer_capability`
///    (「**計器が無かった**」)。
/// 2. 宣言は**ある**が行が **0 本** → 適用不可・`no_pointer_rows` (「**判定不能**」——
///    `capabilities.rs:244-245` 逐語「0 行は判定不能」のとおり、宣言はコンパイル時の
///    `POINTER_AVAILABLE` でしかなく、実行時の `observe::start` は設置に失敗しうる。
///    「活動が無かった」と「設置に失敗した」を区別できないので、0 行を所見として
///    `true` にはしない。★ 極 M+ の判断 13 (0 行は `not_applicable`) と揃えた)。
/// 3. 宣言があり行が **1 本以上** → 適用可・`null`。
///
/// ⚠️ 極 M+ (`no_modifier_rows`) とは**根拠が違う**ので理由の語も別にする。
/// `rows` が `None` (証拠を渡し忘れ) は防御の値 (★ 実際には呼び出し側が pole==PointerB のとき
/// 必ず数える。[`pole_applicability`] の `PointerB` 既定値と同じ語にする)。
fn pointer_applicability(declared: bool, rows: Option<u64>) -> (bool, Option<&'static str>) {
    if !declared {
        return (false, Some("no_pointer_capability"));
    }
    match rows {
        Some(0) => (false, Some("no_pointer_rows")),
        Some(_) => (true, None),
        None => (false, Some("pointer_evidence_not_supplied")),
    }
}

/// 事前宣言 JSON の `id` の最大文字数 (`^[a-z0-9][a-z0-9-]{0,63}$` = 先頭 1 + 残り 63)。
const PREREG_ID_MAX_LEN: usize = 64;

/// 事前宣言 JSON の `version` (`major.minor.patch`) の各部の最大桁数。
const PREREG_VERSION_PART_MAX_DIGITS: usize = 9;

/// 形式検査に落ちた外部の値をエラー文へ出すときの最大文字数。
/// ⚠️ 値は外部ファイル由来で、長さも中身も信用できない (端末へ制御文字を流させない・巨大な行を出さない)。
const PREREG_ECHO_MAX_CHARS: usize = 80;

/// `id` の形式: `^[a-z0-9][a-z0-9-]{0,63}$` (issue 007・`decisions.md` 判断 16)。
///
/// ⚠️ `regex` crate は足さない (新規依存は Stop)。バイトで走査するので、非 ASCII・空白・制御文字・
/// **末尾の改行** (`$` が改行の手前で止まる正規表現実装の抜け穴) は文字クラスに入らず落ちる。
fn is_wellformed_prereg_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= PREREG_ID_MAX_LEN
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && rest
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// `version` の形式: `^[0-9]+\.[0-9]+\.[0-9]+$` (各部は 1〜[`PREREG_VERSION_PART_MAX_DIGITS`] 桁)。
///
/// ⚠️ 桁は ASCII の `0-9` だけ (全角数字・符号・空白・末尾の改行は落ちる)。
fn is_wellformed_prereg_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            (1..=PREREG_VERSION_PART_MAX_DIGITS).contains(&part.len())
                && part.bytes().all(|b| b.is_ascii_digit())
        })
}

/// エラー文へ出す外部の値: 切り詰めて `{:?}` でエスケープし、元の文字数を添える。
fn echo_untrusted(value: &str) -> String {
    let shown: String = value.chars().take(PREREG_ECHO_MAX_CHARS).collect();
    format!("{shown:?} ({} 文字)", value.chars().count())
}

/// `id` / `version` の**形式だけ**を検査する (issue 007・起票 P1-G)。
///
/// ★ 凍結 JSON との**一致は求めない** (別の config を使うこと自体は正当。区別は `config_source` /
/// `prereg_sha256_prefix` / `overrides` が担う・`decisions.md` 判断 16)。
/// 封筒 `config.prereg_id` / `config.prereg_version` へ外部の文字列をそのまま写さないための門。
fn validate_prereg_identity(file: &P1PreregFile) -> Result<(), String> {
    if !is_wellformed_prereg_id(&file.id) {
        return Err(format!(
            "事前宣言 JSON の `id` が形式に合わない (`^[a-z0-9][a-z0-9-]{{0,63}}$`): {}",
            echo_untrusted(&file.id)
        ));
    }
    if !is_wellformed_prereg_version(&file.version) {
        return Err(format!(
            "事前宣言 JSON の `version` が形式に合わない (`<数字>.<数字>.<数字>`・各部 1〜{PREREG_VERSION_PART_MAX_DIGITS} 桁): {}",
            echo_untrusted(&file.version)
        ));
    }
    Ok(())
}

/// 事前宣言 JSON の本文を読む (`id` / `version` / `operational`)。
///
/// ★ `id` / `version` の形式もここで検査する (issue 007)。埋め込みの凍結 JSON も外部ファイルも
/// **同じ 1 か所**を通るので、片方だけ検査が抜けることがない。
fn parse_prereg_text(text: &str) -> Result<P1PreregFile, String> {
    let file = serde_json::from_str::<P1PreregFile>(text)
        .map_err(|e| format!("事前宣言 JSON を読めない: {e}"))?;
    validate_prereg_identity(&file)?;
    Ok(file)
}

/// バイト列の SHA-256 先頭 [`SOURCE_SHA256_HEX_CHARS`] 桁 (`sha256_prefix_of_file` のバイト列版)。
fn sha256_prefix_of_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    hex[..SOURCE_SHA256_HEX_CHARS].to_string()
}

/// `--features` の設定を解決する。★ 出力を 1 バイトも書く前に呼ぶこと (fail-closed)。
///
/// 1. 既定は**埋め込みの凍結 JSON** (`config_source = "frozen"`)。
/// 2. `feature_config_path` があればその `operational` に置き換える (`"file"`)。
/// 3. `sets` (`key=value`) を順に上書きする。
/// 4. 物理的な範囲を検査し、凍結値から**外れたキー**を `overrides` に残す。
///
/// ⚠️ 未知のキー・型の不一致・範囲外・`id` / `version` の形式違い ([`validate_prereg_identity`]) は
/// **すべて `Err`** (呼び出し側は exit 1)。
fn resolve_feature_config(
    feature_config_path: Option<&Path>,
    sets: &[String],
) -> Result<ResolvedFeatureConfig, String> {
    let frozen = parse_prereg_text(FROZEN_P1_EPISODE_JSON)
        .map_err(|e| format!("埋め込みの凍結 JSON が壊れている (ビルドの不具合): {e}"))?;
    frozen
        .operational
        .validate()
        .map_err(|e| format!("埋め込みの凍結 JSON が範囲外 (ビルドの不具合): {e}"))?;

    let (base, source, sha) = match feature_config_path {
        None => (
            parse_prereg_text(FROZEN_P1_EPISODE_JSON)?,
            "frozen",
            sha256_prefix_of_bytes(FROZEN_P1_EPISODE_JSON.as_bytes()),
        ),
        Some(path) => {
            let file = File::open(path)
                .map_err(|e| format!("--feature-config を開けない ({}): {e}", path.display()))?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(file, MAX_FEATURE_CONFIG_BYTES + 1),
                &mut bytes,
            )
            .map_err(|e| format!("--feature-config を読めない ({}): {e}", path.display()))?;
            if bytes.len() as u64 > MAX_FEATURE_CONFIG_BYTES {
                return Err(format!(
                    "--feature-config が大きすぎる (上限 {MAX_FEATURE_CONFIG_BYTES} バイト)"
                ));
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "--feature-config が UTF-8 ではない".to_string())?;
            (
                parse_prereg_text(text)?,
                "file",
                sha256_prefix_of_bytes(&bytes),
            )
        }
    };

    let mut operational = base.operational;
    for entry in sets {
        let (key, value) = entry.split_once('=').ok_or_else(|| {
            format!("--set は <key>=<value> の形でなければならない (実際: `{entry}`)")
        })?;
        operational.apply_set(key, value)?;
    }
    operational.validate()?;

    let overrides = frozen
        .operational
        .as_pairs()
        .into_iter()
        .zip(operational.as_pairs())
        .filter(|((_, frozen_value), (_, value))| frozen_value != value)
        .map(|((key, frozen_value), (_, value))| ConfigOverride {
            key,
            frozen: frozen_value,
            value,
        })
        .collect();

    Ok(ResolvedFeatureConfig {
        operational,
        prereg_id: base.id,
        prereg_version: base.version,
        prereg_sha256_prefix: sha,
        config_source: source,
        overrides,
        pole: Pole::Baseline,
    })
}

impl FeatureConfigDto {
    /// `modifier_rows` は入力ファイルの修飾キーの `key` 行数 (極 M+ の適用可否・[`pole_applicability`])。
    /// `modifier_truncated` / `modifier_orphan_releases` は極 M+ の区間の健全性カウンタ
    /// (`decisions.md` 判断 24・[`ModifierScan`])。
    fn from_resolved(
        resolved: &ResolvedFeatureConfig,
        max_feature_ticks: usize,
        modifier_rows: Option<u64>,
        modifier_truncated: Option<u64>,
        modifier_orphan_releases: Option<u64>,
    ) -> Self {
        let c = resolved.stall_onset_config();
        let (pole_applicable, pole_not_applicable_reason) =
            pole_applicability(c.pole, modifier_rows);
        Self {
            w_ms: c.w_ms,
            q: c.q,
            b_ms: c.b_ms,
            net_diff_min: c.net_diff_min,
            hole_gap_ms: c.hole_gap_ms,
            warmup_count: c.warmup_count,
            theta_estimator: resolved.operational.theta_estimator.clone(),
            t1a_window_ms: c.t1a_window_ms,
            idle_trim_min_gap_ms: resolved.operational.idle_trim_min_gap_ms,
            idle_trim_max_ms: resolved.operational.idle_trim_max_ms,
            idle_trim_input_gap_ms: resolved.operational.idle_trim_input_gap_ms,
            pole: c.pole.as_str(),
            pole_applicable,
            pole_not_applicable_reason,
            pole_modifier_rows: match c.pole {
                Pole::ModifierMPlus => modifier_rows,
                Pole::Baseline | Pole::PointerB => None,
            },
            pole_modifier_truncated: match c.pole {
                Pole::ModifierMPlus => modifier_truncated,
                Pole::Baseline | Pole::PointerB => None,
            },
            pole_modifier_orphan_releases: match c.pole {
                Pole::ModifierMPlus => modifier_orphan_releases,
                Pole::Baseline | Pole::PointerB => None,
            },
            // ★ 極 B の証拠 (行数・適用可否) は入力が要るので [`Self::with_pointer_evidence`] が足す。
            pole_pointer_rows: None,
            prereg_id: resolved.prereg_id.clone(),
            prereg_version: resolved.prereg_version.clone(),
            prereg_sha256_prefix: resolved.prereg_sha256_prefix.clone(),
            config_source: resolved.config_source,
            overrides: resolved.overrides.clone(),
            max_feature_ticks,
            hmm_version: HMM_VERSION,
            engine_tick_ms: ENGINE_TICK_MS,
            pause_threshold_ms: gse_next_lib::analysis::features::PAUSE_THRESHOLD_MS,
        }
    }

    /// 極 B の**適用可否と `pointer_bin` 行数**を確定する (issue 006)。★ 極 B 以外では何もしない
    /// (`pole_pointer_rows` は `null` のまま・渡された値を漏らさない)。
    ///
    /// `declared` は [`declares_pointer_bin`] (封筒 `provenance.capabilities` の宣言)、`rows` は入力の
    /// `pointer_bin` 行数。⚠️ 極 B で呼ばなければ `pole_applicable == false` /
    /// `pointer_evidence_not_supplied` の防御の値のまま (嘘の `true` を出さない)。
    /// ★ `rows` は [`pointer_applicability`] (判断 26 の 3 状態判定) にも渡す —— 宣言があっても
    /// 0 行なら `pole_applicable == false` になる (「判定不能」)。
    fn with_pointer_evidence(mut self, declared: bool, rows: Option<u64>) -> Self {
        if self.pole == Pole::PointerB.as_str() {
            let (applicable, reason) = pointer_applicability(declared, rows);
            self.pole_applicable = applicable;
            self.pole_not_applicable_reason = reason;
            self.pole_pointer_rows = rows;
        }
        self
    }
}

/// `meta`(session_start) 行から読めるセッション単位の事実 (封筒の `provenance` 用)。
///
/// ⚠️ **`os` は含まない** —— `logger.rs::write_meta_start` が書いていないため
/// (2026-09-20 実測)。★ 足すのは計器の変更 (起票 1・ADR-032) であってここではない。
#[derive(Debug, Default)]
struct MetaSessionFacts {
    session_start: Option<u64>,
    hmm_version: Option<String>,
    app_version: Option<String>,
    g1_mode: Option<String>,
    o1_mode: Option<String>,
    keyboard_layout: Option<String>,
    /// ★ **同じ `meta` 行**から取る (2026-09-20・code-reviewer MEDIUM)。
    /// ⚠️ 別関数が別条件で拾うと、連結ログで**別セグメントの宣言**が混ざる。
    capabilities: Option<serde_json::Value>,
}

/// NDJSON の 1 行を表す Deserialize 専用 enum。
///
/// `logger::LogEntry` とは意図的に別型。`logger` は書き出し専用・`ReplayEvent`
/// は読み込み専用として責務を分離する。JSON のフィールド名は NDJSON 実形式に
/// 完全一致させる（例: `timestamp` ではなく `t`、`vk_code` ではなく `vk`）。
// Some fields were read only by the modes this cut leaves out. They are kept so that each row is
// parsed exactly as in GSE (a required field that is missing still fails the row).
#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
enum ReplayEvent {
    /// `{"type":"meta","session_start":...,"hmm_version":"..."}` または
    /// `{"type":"meta","session_end":...}` のいずれか。
    #[serde(rename = "meta")]
    Meta {
        session_start: Option<u64>,
        #[allow(dead_code)]
        session_end: Option<u64>,
        hmm_version: Option<String>,
    },
    /// `start_session` コマンド発火時刻マーカー。replay では extractor を
    /// リセットして「session 境界」を再現するために使う。
    #[serde(rename = "session_start")]
    SessionStart { t: u64 },
    /// キーストローク。raw ログは `vk` (Windows 規約 VK コード、macOS 側はフック層で変換済み)
    /// を持つ。transfer-pipeline (#27) の `gse-package` が匿名化した NDJSON は本文再構成を
    /// 防ぐため `vk` を落とし `category` (KeyCategory 文字列) を持つ。replay は **両対応**で
    /// 同じ特徴量を再計算する (D6 Replay-as-Truth)。どちらも省略可だが、実際の行は片方を持つ。
    /// 両方欠落 / 未知 category の行は不正として読み飛ばす (`resolve_key_vk` が `None`)。
    #[serde(rename = "key")]
    Key {
        t: u64,
        #[serde(default)]
        vk: Option<u32>,
        #[serde(default)]
        category: Option<String>,
        press: bool,
    },
    /// ペーストイベント (Ctrl+V / Cmd+V) の発生時刻マーカーと文字数。
    /// features/HMM には未使用のため replay では読み飛ばす (`timestamp()==None`)。
    /// 本 variant が無いと NDJSON に paste 行があるとき deserialize でエラーになる。
    /// `external_paste_chars` は新フィールド。旧フィクスチャ (フィールド無し) 互換のため
    /// `#[serde(default)]` で欠落を許容する (None になる)。
    #[serde(rename = "paste")]
    Paste {
        #[allow(dead_code)]
        t: u64,
        #[allow(dead_code)]
        #[serde(default)]
        external_paste_chars: Option<u64>,
    },
    /// IME 状態変化。`on` は IME モード（β 切替用）、`active` は候補ウィンドウ
    /// の開閉（engine pause 用）。
    #[serde(rename = "ime_state")]
    ImeState { t: u64, on: bool, active: bool },
    /// 既存セッションの Feat エントリ。
    ///
    /// `run_replay` (snapshot 検証経路) では step() 再実行で自前生成するため、
    /// 本 variant は読み飛ばし専用 (`timestamp()` も `None` を返し時刻順ソート
    /// から除外)。metrics 経路では `collect_logged_feats()` 側で `LoggedFeatDto`
    /// を使って別途構造化パースする (clock semantic mismatch 対策、
    /// `.steering/20260416-replay-feat-alignment-bug/decisions.md` D1 参照)。
    #[serde(rename = "feat")]
    Feat(#[allow(dead_code)] serde_json::Value),
    /// Phase 1 Ground Truth ラベル。`--metrics` で評価対象として突き合わせる。
    /// `--snapshot` / デフォルト (snapshot 出力) の経路では読み飛ばす。
    #[serde(rename = "user_label")]
    UserLabel {
        t: u64,
        label: String,
        #[allow(dead_code)]
        recovered_at: Option<u64>,
    },
    /// 事後レビュー用ブックマークマーカー。`--segment` でハイライト表示に使う。
    /// `--snapshot` / `--metrics` の経路では読み飛ばす。
    #[serde(rename = "review_marker")]
    ReviewMarker {
        #[allow(dead_code)]
        t: u64,
    },
    /// Phase 2b IME Shadow Features。replay では読み飛ばす (`timestamp()` が `None`)。
    ///
    /// ⚠️★★★ **この variant を型付き構造体にしてはならない。`serde_json::Value` のまま置く。**
    ///
    /// ★ `--features` (`T1b`・issue 002) が中身を要るようになったが、構造化は
    /// [`collect_all_ime_shadow_events`] の中の専用 DTO ([`ImeShadowDto`]) が行う。
    /// ★ **同じ手法の先例は `feat` 行の [`LoggedFeatDto`]** である (本 variant の上の
    /// `ReplayEvent::Feat` の doc を参照)。
    ///
    /// ★ 理由 (2026-09-15・`code-reviewer` / `security-checker` が独立に HIGH で指摘し、
    /// **両者とも実測で再現した**): `run_replay` は**既知タグのパース失敗も `Err` にして
    /// 中断する**ので、ここを厳格にすると「読み飛ばすはずの行」が**リプレイを丸ごと殺す**。
    /// ⚠️ 実測した落ち方 (issue 002 の初版):
    /// - `"abandon":0` (整数) → 逐語 `invalid type: integer 0, expected a boolean`
    /// - `nav` 欠落 → 逐語 `missing field 'nav'`
    /// - `"dwell_ms":null` → 逐語 `invalid type: null, expected u64`
    ///
    /// ⚠️ **`Option` ＋ `#[serde(default)]` でも足りない** (★ 2026-09-15 に実測):
    /// `Option<bool>` は `0` を受け取れないので `"abandon":0` は依然 exit 1 になる。
    /// → ★ **受理集合を 1 バイトも狭めない唯一の形が `Value`** である。
    ///
    /// ★ これは `hook_health` (2026-08-09) / `activity_label` / `starve_window` (2026-09-14) で
    /// 3 度踏んだ失敗様式の 4 度目であり、回帰は
    /// [`tests::partial_ime_shadow_line_does_not_break_run_replay`] が張る。
    /// ⚠️ **issue 007 が `hook_health` を構造化するときも同じ形にすること。**
    #[serde(rename = "ime_shadow")]
    ImeShadow(#[allow(dead_code)] serde_json::Value),
    /// MVP Opportunity 判定ログ。replay では読み飛ばす。
    #[serde(rename = "intervention_decision")]
    InterventionDecision(#[allow(dead_code)] serde_json::Value),
    /// 介入応答 (dismiss 理由) ログ。features/HMM 未使用のため replay では読み飛ばす。
    /// 本 variant が無いと NDJSON に intervention_response 行があるとき deserialize でエラーになる。
    #[serde(rename = "intervention_response")]
    InterventionResponse(#[allow(dead_code)] serde_json::Value),
    /// ⚠️★★ キーボードフックの生存診断 (`input::liveness`)。features/HMM 未使用のため読み飛ばす。
    ///
    /// ★ **この variant が無いと `--snapshot` 経路が丸ごと落ちる。**
    /// `run_replay` は未知タグを `Err` にして中断するので (`--metrics` 側だけが lenient)、
    /// 行型を足したのに ここへ足し忘れると **実セッションが 1 本も replay できなくなる**
    /// (= Replay-as-Truth が切れる)。⚠️ 2026-08-09 に実測で踏んだ:
    /// `parse error at line 2: unknown variant 'hook_health'`。
    /// 回帰は `hook_health_line_is_skipped_by_run_replay` が張っている。
    #[serde(rename = "hook_health")]
    HookHealth(#[allow(dead_code)] serde_json::Value),
    /// ⚠️★★★ **沈黙窓の診断行**（`input::liveness`・`THRESHOLD_GATED`）。★ 読み飛ばし専用。
    ///
    /// ⚠️★★★ **2026-09-14 に *既存の* 欠落として発見した**（★ 新設した
    /// [`tests::every_known_line_type_is_accepted_by_replay_event`] が掘り当てた）。
    /// ★ `logger.rs` は `starve_window` を書くのに、ここに variant が無かった。
    /// → ⚠️ **`starve_window` 行を 1 本でも含む NDJSON は `--snapshot` が丸ごと落ちていた**
    /// （実測: 逐語 `parse error at line 32: unknown variant 'starve_window'`）。
    /// ★ これは `hook_health`（2026-08-09）/ `activity_label`（2026-09-14）と**同一の失敗様式で 3 件目**である。
    #[serde(rename = "starve_window")]
    StarveWindow(#[allow(dead_code)] serde_json::Value),
    /// ⚠️★★★ **O1 (R3) で足した 4 行種**。★ どれも **読み飛ばし専用**である。
    ///
    /// ## ★ なぜ variant を足さなければならないのか（⚠️ 「読まないなら不要」ではない）
    ///
    /// ★ `run_replay` は**未知タグを `Err` にして中断する**（`--metrics` 側だけが lenient）。
    /// → ⚠️★★★ **行種を足したのにここへ足し忘れると、その形式のログが 1 本も replay できなくなる**
    /// （= Replay-as-Truth が切れる）。★ 2026-08-09 に `hook_health` で実測済みの失敗様式である。
    ///
    /// ## ⚠️ 特徴量には 1 バイトも流さない（ADR-014 **D4b**）
    ///
    /// ★ `timestamp()` が `None` を返すので時刻順ソートからも外れ、`step()` を 1 度も呼ばない。
    #[serde(rename = "recording_pause")]
    RecordingPause(#[allow(dead_code)] serde_json::Value),
    /// ⚠️ 読み飛ばし専用（上記 `RecordingPause` の doc を参照）。
    #[serde(rename = "focus_transition")]
    FocusTransition(#[allow(dead_code)] serde_json::Value),
    /// ⚠️ 読み飛ばし専用（同上）。★ 例外は **`--features --pole pointer_b` の
    /// [`collect_pointer_bin_timestamps`] だけ**（issue 006）—— 行の**時刻 `t` だけ**を読み、値は読まない。
    /// ⚠️ `timestamp()` は `None` のままなので HMM 経路（`--snapshot` / `--metrics`）へは 1 バイトも流れない。
    #[serde(rename = "pointer_bin")]
    PointerBin(serde_json::Value),
    /// ⚠️ 読み飛ばし専用（同上）。
    #[serde(rename = "secure_input")]
    SecureInput(#[allow(dead_code)] serde_json::Value),
    /// 読み飛ばし専用。EH3 のその日の条件（記録のみ・推論へ流さない）。
    #[serde(rename = "eh3_day")]
    Eh3Day(#[allow(dead_code)] serde_json::Value),
    /// 読み飛ばし専用。EH3 の招待を出した時刻（記録のみ・推論へ流さない）。
    #[serde(rename = "invitation_present")]
    InvitationPresent(#[allow(dead_code)] serde_json::Value),
    /// ⚠️★★★ **O1 の監査 criterion**（ADR-031 D1）。★ 読み飛ばし専用。
    ///
    /// ⚠️★★★ **2026-09-14 に足し忘れて実測で踏んだ。**★ `activity_label` 行を 1 本足した
    /// NDJSON を `--snapshot` に掛けると逐語
    /// `replay failed: parse error at line 32: unknown variant 'activity_label'`
    /// で**丸ごと落ちた** —— ★ 上の `HookHealth` の doc が「2026-08-09 に実測済みの失敗様式」として
    /// 名指ししているものと**同一**である（★ 同じ轍を 2 度踏んだ）。
    /// ⚠️ **ゴールデンが緑でもこの事故は検出できない**（★ 既存 fixture に行が無いため）。
    /// → ★ 回帰は [`tests::activity_label_line_is_skipped_by_run_replay`] が張る。
    ///
    /// ★ 自己申告は **記録のみ・推論に 1 バイトも流さない**（ADR-014 D4b / ADR-031 D2）。
    #[serde(rename = "activity_label")]
    ActivityLabel(#[allow(dead_code)] serde_json::Value),
    /// v0.3 較正スモーク: セッション宣言。`--calibration` 経路で track を抽出する。
    /// snapshot/run_replay 経路では `timestamp()==None` で読み飛ばす。
    #[serde(rename = "session_meta")]
    SessionMeta {
        track: String,
        #[allow(dead_code)]
        task_kind: String,
        #[allow(dead_code)]
        #[allow(dead_code)]
        intervention_enabled: bool,
        // 新フィールド (WBS 0.11 / 0.4)。旧フィクスチャ互換のため `#[serde(default)]`。
        #[allow(dead_code)]
        #[serde(default)]
        condition: Option<String>,
        #[allow(dead_code)]
        #[serde(default)]
        label_pool: Option<String>,
        #[allow(dead_code)]
        ts: u64,
    },
    /// v0.3 較正スモーク: ランダムプローブ割当の監査記録。`--calibration` で件数を数える。
    #[serde(rename = "probe_assignment")]
    ProbeAssignment {
        #[allow(dead_code)]
        probe_id: String,
        #[allow(dead_code)]
        scheduled_ts: u64,
        #[allow(dead_code)]
        session_elapsed_ms: u64,
    },
    /// v0.3 較正スモーク: 主データ (リアルタイム/自己マーカー回答)。`--calibration` で ProbeRecord 化する。
    #[serde(rename = "realtime_label")]
    RealtimeLabel {
        #[allow(dead_code)]
        probe_id: Option<String>,
        source: String,
        q1_friction: Option<u8>,
        q2_progress: Option<u8>,
        answered: bool,
        skip_reason: String,
        latency_ms: u64,
        analysis_window_start_ms: u64,
        analysis_window_end_ms: u64,
        #[allow(dead_code)]
        ts: u64,
    },
    /// v0.3 較正スモーク: 尺度理解テスト回答。`--calibration` で VignetteRecord 化する。
    #[serde(rename = "vignette_response")]
    VignetteResponse {
        vignette_id: String,
        expected_quadrant: String,
        q1_friction: u8,
        q2_progress: u8,
        /// 2026-08-17 以降のログにのみ存在。⚠️ **旧ログでは `None`** であり、
        /// `0`（＝即答）と混同してはならない
        /// (`deny_unknown_fields` を付けていないので前方互換・後方互換とも壊れない)。
        #[serde(default)]
        elapsed_ms: Option<u64>,
        #[serde(default)]
        changes: Option<u16>,
        #[allow(dead_code)]
        ts: u64,
    },
}

impl ReplayEvent {
    /// タイムスタンプを取り出す（Meta は `session_start` フィールドから）。
    /// `None` は時刻順ソートから除外する（既存 Feat など）。
    fn timestamp(&self) -> Option<u64> {
        match self {
            ReplayEvent::Meta { session_start, .. } => *session_start,
            ReplayEvent::SessionStart { t } => Some(*t),
            ReplayEvent::Key { t, .. } => Some(*t),
            // Paste は記録専用 (回数集計用)。step() 再実行に影響しないため除外する。
            ReplayEvent::Paste { .. } => None,
            ReplayEvent::ImeState { t, .. } => Some(*t),
            ReplayEvent::Feat(_) => None,
            // UserLabel は再実行対象ではない (ゲート進行を動かさない)。
            // timestamp() から None を返して時刻順ソートから除外し、
            // metrics 算出経路で別途抽出する。
            ReplayEvent::UserLabel { .. } => None,
            ReplayEvent::ReviewMarker { .. } => None,
            ReplayEvent::ImeShadow(_) => None,
            ReplayEvent::InterventionDecision(_) => None,
            ReplayEvent::InterventionResponse(_) => None,
            // 診断行。ゲート進行を動かさないので時刻順ソートから除外する。
            ReplayEvent::HookHealth(_) => None,
            ReplayEvent::StarveWindow(_) => None,
            // ⚠️★★★ O1 (R3) の 4 行種。★ **記録のみ・推論に 1 バイトも流さない**
            //   (ADR-014 D4b) ので、`None` を返して時刻順ソートから外す。
            ReplayEvent::RecordingPause(_) => None,
            ReplayEvent::FocusTransition(_) => None,
            ReplayEvent::PointerBin(_) => None,
            ReplayEvent::SecureInput(_) => None,
            // EH3 の 2 行種も記録のみで推論へ流さない。
            ReplayEvent::Eh3Day(_) => None,
            ReplayEvent::InvitationPresent(_) => None,
            // ⚠️★★★ 監査 criterion（ADR-031）。★ 記録のみで推論へ流さない。
            ReplayEvent::ActivityLabel(_) => None,
            // v0.3 較正 variant は --calibration 専用コレクタで別途抽出する。
            // run_replay (snapshot 経路) の時刻順ソートからは除外する (UserLabel と同方針)。
            ReplayEvent::SessionMeta { .. } => None,
            ReplayEvent::ProbeAssignment { .. } => None,
            ReplayEvent::RealtimeLabel { .. } => None,
            ReplayEvent::VignetteResponse { .. } => None,
        }
    }
}

/// vk_code から is_backspace を再計算。
///
/// 既存の hook_windows.rs:304 と hook_macos.rs:340 の判定を複製:
/// Windows 規約 0x08 (VK_BACK) / 0x2E (VK_DELETE) が true。
fn is_backspace(vk: u32) -> bool {
    vk == 0x08 || vk == 0x2E
}

/// 匿名化 NDJSON の `category` (KeyCategory 文字列) を **代表 VK コード**へ逆写像する。
///
/// `gse-package` の匿名化は個々の VK を `KeyCategory` に潰すため (本文再構成不能化)、
/// replay 側で特徴量を再計算するには代表 VK を 1 つ復元すればよい。特徴量に効くのは
/// is_backspace (F3/F6) と文字生成キー区分・(Windows) リピート検出のみで、いずれも
/// **category 単位**で決まるため、同じ category に再分類される代表値で raw と同じ
/// 特徴量が再現する。
///
/// 不変条件: 任意の既知 category `c` について
/// `KeyCategory::from_vk(category_representative_vk(c).unwrap()).as_str() == c`。
/// `category_representative_vk_roundtrips` テストが lib の `KeyCategory::from_vk` で drift を検出する。
/// 未知 category は `None` (forward-compat、呼び出し側で件数を warn してスキップ)。
///
/// **`is_text_producing` との関係 (decisions.md D-RC3 / ADR-014 判断 6)**:
/// **2026-08-23 に D-RC3 の制約は解消した。** `KeyCategory` に `Punctuation` (OEM 句読点/記号 +
/// テンキー演算子) と `Tab` を足したので、**`is_text_producing` が true な VK は 1 つ残らず
/// 非 `other` category に入る**。したがって `other` の代表 `0x07` (非 text-producing) は
/// **保守的な倒し込みではなく厳密に正しい**値になり、`--metrics` の `manual_insert` 系の
/// 過小評価 (旧: OEM 句読点/Tab/テンキー演算子の件数分) は **ゼロ**になった。
///
/// 旧仕様: これらは `Other` へ畳まれ、匿名化後に text-producing と判別できなかった。
/// `manual_contribution_ratio` は `research-positioning.md §5` **H-Y の主要アウトカム**であり、
/// 構造的な過小評価は研究上許容できない (`.steering/20260823-r3-accept-and-role-category/` D-7)。
fn category_representative_vk(category: &str) -> Option<u32> {
    // anonymize.rs `KeyCategory::from_vk` の逆。各 category の代表 VK を 1 つ選ぶ。
    let vk = match category {
        "alphabetic" => 0x41, // 'A' (0x41..=0x5A) — text-producing
        "numeric" => 0x30,    // '0' (0x30..=0x39) — text-producing
        // VK_OEM_1 (';') — text-producing。⚠️ 0xBE / 0xBC は SentencePunctuation なので使えない。
        "punctuation" => 0xBA,
        // VK_OEM_PERIOD ('.') — text-producing。boundary keys の「文間」。
        "sentence_punctuation" => 0xBE,
        "tab" => 0x09,        // VK_TAB — text-producing
        "backspace" => 0x08,  // VK_BACK — is_backspace
        "delete" => 0x2E,     // VK_DELETE — is_backspace
        "space" => 0x20,      // VK_SPACE — text-producing
        "enter" => 0x0D,      // VK_RETURN — text-producing
        "navigation" => 0x25, // VK_LEFT (0x21..=0x28)
        "modifier" => 0x10,   // VK_SHIFT
        "function" => 0x70,   // VK_F1 (0x70..=0x7B)
        "escape" => 0x1B,     // VK_ESCAPE
        // from_vk の既定 (どのレンジにも入らず Other へ)。
        // 2026-08-23 以降、raw の Other に text-producing キーは混在しない (上記 doc)。
        "other" => 0x07,
        _ => return None,
    };
    Some(vk)
}

/// `key` イベントの実効 VK コードを解決する (raw / 匿名化 両対応)。
///
/// - raw ログ: `vk` をそのまま使う。
/// - 匿名化ログ: `category` を `category_representative_vk` で代表 VK へ逆写像する。
///
/// `vk` を優先し (両方ある異常入力でも raw を尊重)、無ければ `category`、
/// どちらも解決できなければ `None`。
fn resolve_key_vk(vk: Option<u32>, category: Option<&str>) -> Option<u32> {
    vk.or_else(|| category.and_then(category_representative_vk))
}

/// コマンドライン引数をパース。
///
/// This cut keeps two paths only: the no-flag path (FeatSnapshot rows on stdout) and `--features`.
/// The other modes of the GSE original (`--snapshot`, `--metrics`, `--segment(s)`, `--intervention`,
/// `--calibration`, `--sweep`, `--nudge-diag`, `--corr-floor`, `--json`) are not part of this crate;
/// their flags are not recognised and so count as extra positional arguments (exit 2).
struct Args {
    session_path: PathBuf,
    /// --features: ADR-030 の Tier 別 1 Hz tick 単位特徴行を出力する (issue 001-007)。
    /// ⚠️ 手元の解析用中間成果物であり `gse-dataset` へ共有しない (`docs/glossary.md` 逐語)。
    features: bool,
    /// --feature-config <path>: `--features` の設定を外部 JSON (凍結 JSON と同じ形) から読む。
    feature_config_path: Option<PathBuf>,
    /// --set <key>=<value> (繰り返し可): `--features` の設定を 1 キーずつ上書きする。
    /// ⚠️ 生の文字列のまま持つ。検証 (未知キー / 型 / 範囲) は `resolve_feature_config` が行う。
    feature_sets: Vec<String>,
    /// --pole <baseline|pointer_b|modifier_m_plus>: `--features` の極 (ADR-030 D3・1 実行 1 極)。
    /// ⚠️ 生の文字列のまま持つ。検証 (`--features` 専用 / 未知の名前 / 未実装) は `resolve_pole` が行う
    /// (★ `--set` と同じ作法。未知の名前は exit 1)。
    pole: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1).collect())
}

/// `parse_args()` の本体。`Vec<String>` を直接受けるため、`std::env::args()`
/// (プロセス全体で 1 つ) に依存せず単体テストできる。
fn parse_args_from(mut args: Vec<String>) -> Result<Args, String> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Err(usage());
    }

    // --features フラグ (ADR-030 の Tier 別 1 Hz tick 単位特徴行を出力。issue 001-007)
    let features = if let Some(pos) = args.iter().position(|a| a == "--features") {
        args.remove(pos);
        true
    } else {
        false
    };

    // --feature-config <path> / --set <key>=<value> (繰り返し可): --features 専用の設定注入。
    let feature_config_path = if let Some(pos) = args.iter().position(|a| a == "--feature-config") {
        if pos + 1 >= args.len() {
            return Err("--feature-config requires a path argument".to_string());
        }
        let path = args.remove(pos + 1);
        args.remove(pos);
        Some(PathBuf::from(path))
    } else {
        None
    };
    let mut feature_sets: Vec<String> = Vec::new();
    while let Some(pos) = args.iter().position(|a| a == "--set") {
        if pos + 1 >= args.len() {
            return Err("--set requires a <key>=<value> argument".to_string());
        }
        feature_sets.push(args.remove(pos + 1));
        args.remove(pos);
    }
    if !features && (feature_config_path.is_some() || !feature_sets.is_empty()) {
        return Err("--feature-config and --set require --features".to_string());
    }

    // --pole <name>: `--features` の極。⚠️ 名前の検証は `resolve_pole` (main が出力の前に呼ぶ)。
    let pole = if let Some(pos) = args.iter().position(|a| a == "--pole") {
        if pos + 1 >= args.len() {
            return Err("--pole requires a <name> argument".to_string());
        }
        let name = args.remove(pos + 1);
        args.remove(pos);
        // ⚠️ 2 回目の `--pole` を位置引数として残すと「positional が 3 個」という理由の分からない
        //   エラーになる (security-checker LOW・2026-09-25)。→ 重複は名指しで止める。
        if args.iter().any(|a| a == "--pole") {
            return Err("--pole may be specified only once (1 実行 1 極)".to_string());
        }
        Some(name)
    } else {
        None
    };

    // positional 引数の検証: 厳密に 1 個。
    if args.is_empty() {
        return Err("expected at least 1 positional argument (session.ndjson)".to_string());
    }
    if args.len() != 1 {
        return Err(format!(
            "expected 1 positional argument (session.ndjson), got {}",
            args.len()
        ));
    }
    let session_path = PathBuf::from(args.remove(0));

    Ok(Args {
        session_path,
        features,
        feature_config_path,
        feature_sets,
        pole,
    })
}

fn usage() -> String {
    String::from(
        "gse-replay — NDJSON セッションを再実行し FeatSnapshot 列を生成\n\n\
         USAGE:\n    \
         gse-replay <session.ndjson>\n    \
         gse-replay <session.ndjson> --features [--pole <name>] [--feature-config <cfg.json>] [--set <key>=<value> ...]\n\n\
         OPTIONS:\n    \
         (no flag)           FeatSnapshot rows (the estimator's per-tick output) as a JSON array on stdout\n    \
         --features          ADR-030 の Tier 別 1 Hz tick 単位特徴行を出力\n    \
         出力は { provenance, config, rows } の封筒形 (2026-09-20・起票 2)\n    \
         provenance に由来 (meta 由来の事実 / 入力の basename・sha256 / capabilities)\n    \
         *** t_ms は絶対 epoch ms で、匿名化 (anonymize.rs) を通っていない ***\n    \
         設定の既定は埋め込みの凍結 JSON (prereg/p1-episode.json)。config.overrides に外れた値だけが残る\n    \
         --feature-config <path>  凍結 JSON と同じ形の外部ファイルから設定を読む (config.config_source = \"file\")\n    \
         --set <key>=<value>      設定を 1 キー上書きする (繰り返し可)。未知キー / 型の不一致 / 範囲外は exit 1 で出力なし\n    \
         --pole <name>            極 (ADR-030 D3): baseline (既定) / pointer_b / modifier_m_plus。1 実行 1 極で、封筒 config.pole に刻む。\n    \
         --features 以外との併用 / 未知の名前 / 未実装の極 (現状 baseline 以外はすべて) は exit 1 で出力なし\n    \
         -h, --help          このヘルプを表示\n",
    )
}

fn main() -> ExitCode {
    // 最小 tracing subscriber (stderr 出力、INFO 以上)。
    // rust-standards ルール 3 準拠: 警告・エラー系は tracing 経由で出す。
    // ただし CLI の「人向け結果メッセージ」(usage / OK 一致 / MISMATCH 差分) は
    // ログではなくコマンド出力として writeln!(stderr, ...) に残す。
    // `try_init` は既に subscriber が登録済みでもエラーにならない。
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .try_init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(std::io::stderr(), "{}", msg);
            return ExitCode::from(2);
        }
    };

    // ⚠️★★★ `--pole` は**どのモードの出力よりも前**に解決する (fail-closed)。
    // `--features` 以外での指定 / 未知の名前 / 未実装の極は exit 1 で、stdout には 1 バイトも出さない。
    // ★ 未指定なら `Baseline` (他のモードは極を読まないので何も変わらない)。
    let pole = match resolve_pole(args.features, args.pole.as_deref()) {
        Ok(v) => v,
        Err(msg) => {
            tracing::error!("invalid --pole: {}", msg);
            return ExitCode::from(1);
        }
    };

    // --features 経路: ADR-030 の Tier 別 1 Hz tick 単位特徴行を JSON 配列で出力する
    // (issue 001)。`features.rs` / `engine.rs` を経由しない独立経路 (D1)。
    if args.features {
        // ⚠️★★★ 設定は**何かを読む・書く前に**解決する (fail-closed)。
        // 未知キー / 型の不一致 / 範囲外は exit 1 で、stdout には 1 バイトも出さない。
        let resolved_config =
            match resolve_feature_config(args.feature_config_path.as_deref(), &args.feature_sets) {
                Ok(v) => v.with_pole(pole),
                Err(msg) => {
                    tracing::error!("invalid --features configuration: {}", msg);
                    return ExitCode::from(1);
                }
            };
        let keys = match collect_key_events(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect key events: {}", msg);
                return ExitCode::from(1);
            }
        };
        // ⚠️ `collect_paste_events` ではなく**全 paste 行の時刻**を使う。
        // ★ `T0` の `pastes` は件数なので、文字数欠落の行を落とすと 0 に潰れる。
        let paste_ts = match collect_all_paste_timestamps(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect paste events: {}", msg);
                return ExitCode::from(1);
            }
        };
        let ime_events = match collect_all_ime_shadow_events(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect ime_shadow events: {}", msg);
                return ExitCode::from(1);
            }
        };
        let raw_feat_ts = match collect_all_feat_timestamps(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect feat timestamps: {}", msg);
                return ExitCode::from(1);
            }
        };
        // ★ 事前宣言 v1.2 §6.1 (ADR-030 補訂 4): 長い穴とファイルの最後の直前の、入力の無い `feat` 行を落とす。
        //   ⚠️ 以後は**落とした後の列だけ**を使う (観測区間・行・品質のマスク・封筒が同じ列を見る)。
        //   入力は `key` 行 (押下・解放とも) と `pointer_bin` 行だけ (`collect_idle_trim_input_timestamps`)。
        let idle_trim_input_ts = match collect_idle_trim_input_timestamps(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect input timestamps for idle trim: {}", msg);
                return ExitCode::from(1);
            }
        };
        let (after_idle_trim, idle_trim_summary) =
            gse_next_lib::analysis::feat_offline::idle_trim::idle_trim(
                &raw_feat_ts,
                &idle_trim_input_ts,
                resolved_config.operational.idle_trim_min_gap_ms,
                resolved_config.operational.idle_trim_max_ms,
            );
        // ★ 事前宣言 v1.3 §7.1 (ADR-030 補訂 5): 入力の間が `idle_trim_input_gap_ms` 以上空いた区間の `feat` 行を落とす。
        //   ⚠️ 境 (ファイルの最初と最後) は idle trim の**前の**列から取る —— 後の列から取ると、idle trim が
        //   末尾を削ったぶんだけ無入力が短く測られ、落とすべき区間を見落とす。落ちる行は 2 つの規則の和集合になる。
        let (feat_ts, input_gap_trim_summary) =
            match (raw_feat_ts.iter().min(), raw_feat_ts.iter().max()) {
                (Some(&first), Some(&last)) => {
                    gse_next_lib::analysis::feat_offline::idle_trim::input_gap_trim(
                        &after_idle_trim,
                        &idle_trim_input_ts,
                        first,
                        last,
                        resolved_config.operational.idle_trim_input_gap_ms,
                    )
                }
                _ => (after_idle_trim, Default::default()),
            };
        let task_kind_declarations = match collect_task_kind_declarations(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect task_kind declarations: {}", msg);
                return ExitCode::from(1);
            }
        };
        // 起票 2 (封筒) ＋ issue 007 (`Q`): `meta`(session_start) 行から
        // セッション単位の事実と `capabilities` を **1 パスで**読む。
        // ⚠️★★★ 2026-09-20: 以前は `capabilities` を別関数が**別の条件**で拾っており、
        //   連結ログで別セグメントの宣言が混ざった (実測)。→ 同じ行から取る。
        // ⚠️ capabilities は「無い」では落とさない (AC7) が、「形が壊れている」
        // (JSON オブジェクトでない / 4096 バイト超) では落とす (判断 24・AC15)。
        let meta_facts = match collect_meta_session_facts(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect meta session facts: {}", msg);
                return ExitCode::from(1);
            }
        };
        let capabilities = meta_facts.capabilities;
        // ⚠️ hook_health は「無い / 形が合わない」で --features を落とさない (旧ログ互換・AC12)。
        let hook_health_events = match collect_all_hook_health_events(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to collect hook_health events: {}", msg);
                return ExitCode::from(1);
            }
        };
        // 起票 2 (封筒): 入力ファイルの同一性。
        // ⚠️ 「行の計算」ではなく「実行の由来」なので、`feat_offline` ではなくここ (CLI) が読む。
        let source_sha256_prefix = match sha256_prefix_of_file(&args.session_path) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to hash session file: {}", msg);
                return ExitCode::from(1);
            }
        };
        let stall_onset_config = resolved_config.stall_onset_config();
        // ★ 観測区間は 1 度だけ作り、極 M+ の打ち切りと封筒 `provenance` の両方へ渡す
        //   (⚠️ 区間の定義をここで作り直さない)。
        let observed = gse_next_lib::analysis::feat_offline::stall_onset::observed_intervals(
            &feat_ts,
            stall_onset_config.hole_gap_ms,
        );
        // ⚠️★★★ 修飾キーの押下区間を要るのは**極 M+ だけ**。他の極では走査しない
        //   (★ baseline の出力・実行経路を 1 バイトも変えない・`--snapshot` / `--metrics` へも配線しない)。
        let modifier_scan = if pole == Pole::ModifierMPlus {
            match collect_modifier_intervals(&args.session_path, &observed) {
                Ok(v) => Some(v),
                Err(msg) => {
                    tracing::error!("failed to collect modifier intervals: {}", msg);
                    return ExitCode::from(1);
                }
            }
        } else {
            None
        };
        // ⚠️★★★ 極 B の抑制区間に `pointer_bin` の行を要るのは**極 B だけ**。他の極ではここで走査しない。
        //   ⚠️ ただし 2026-10-08 から、無入力の末尾の前処理 (`collect_idle_trim_input_timestamps`) が
        //   **全極で** `pointer_bin` 行の時刻を読む (baseline の観測区間の右端にも効く・上の `idle_trim`)。
        let pointer_rows: Option<Vec<u64>> = if pole == Pole::PointerB {
            match collect_pointer_bin_timestamps(&args.session_path) {
                Ok(v) => Some(v),
                Err(msg) => {
                    tracing::error!("failed to collect pointer_bin timestamps: {}", msg);
                    return ExitCode::from(1);
                }
            }
        } else {
            None
        };
        // ★ 極 B の適用可否は `capabilities` の宣言で決める (⚠️ `analysis/` へは意味を持ち込まない)。
        //   宣言が無い入力は「計器が無かった」= 適用不可 → **抑制しない** (空を渡す = baseline と恒等)。
        let pointer_declared = declares_pointer_bin(capabilities.as_ref());
        let no_pointer_rows: Vec<u64> = Vec::new();
        let pointer_active_ts: Option<&[u64]> = match &pointer_rows {
            Some(rows) if pointer_declared => Some(rows.as_slice()),
            Some(_) => Some(no_pointer_rows.as_slice()),
            None => None,
        };
        let pole_inputs = PoleInputs {
            modifier_intervals: modifier_scan.as_ref().map(|s| s.intervals.as_slice()),
            pointer_active_ts,
        };
        // ★ 封筒の `provenance.observed_intervals` は「実際に評価した区間」。極 B では抑制区間を除いた区間で、
        //   ⚠️ 検出と**同じ 2 つの関数** (`pointer_suppression_intervals` / `evaluation_intervals`) から作る
        //   (区間の定義をここで作り直さない)。極 B 以外は観測区間そのまま。
        let evaluated_intervals: Vec<(u64, u64)> = match pointer_active_ts {
            Some(ts) => {
                use gse_next_lib::analysis::feat_offline::stall_onset::{
                    evaluation_intervals, pointer_suppression_intervals,
                };
                evaluation_intervals(
                    &observed,
                    &pointer_suppression_intervals(ts, ENGINE_TICK_MS),
                )
            }
            None => observed.clone(),
        };
        let stall_onset_inputs = gse_next_lib::analysis::feat_offline::StallOnsetInputs {
            feat_ts: &feat_ts,
            task_kind_declarations: &task_kind_declarations,
            config: stall_onset_config,
        };
        // ⚠️★★★ 2026-09-20 改訂 (判断 3・封筒): 行には `capabilities` の**値を渡さない**。
        // 宣言の有無だけが tick ごとの `missing_reasons` に効き、値は封筒が 1 回だけ持つ。
        // ★ これで 2026-09-18 に入れた `Arc` 共有 (判断 24・code-reviewer M4) も不要になった。
        let quality_inputs = gse_next_lib::analysis::feat_offline::QualityInputs {
            capabilities_declared: capabilities.is_some(),
            hook_health: &hook_health_events,
        };
        let rows = match gse_next_lib::analysis::feat_offline::build_feature_rows_with_pole_inputs(
            &keys,
            &paste_ts,
            &ime_events,
            MAX_FEATURE_TICKS,
            &stall_onset_inputs,
            &quality_inputs,
            &pole_inputs,
        ) {
            Ok(v) => v,
            Err(msg) => {
                tracing::error!("failed to build feature rows: {}", msg);
                return ExitCode::from(1);
            }
        };
        let envelope = FeatureEnvelope {
            provenance: FeatureProvenance {
                sharing: SharingMarker::default(),
                tool: "gse-replay",
                tool_version: env!("CARGO_PKG_VERSION"),
                source_basename: source_basename_of(&args.session_path),
                source_sha256_prefix,
                session_start: meta_facts.session_start,
                hmm_version: meta_facts.hmm_version,
                app_version: meta_facts.app_version,
                g1_mode: meta_facts.g1_mode,
                o1_mode: meta_facts.o1_mode,
                keyboard_layout: meta_facts.keyboard_layout,
                capabilities,
                // ★ 行と同じ SSOT (`stall_onset::observed_intervals`) から作った `observed`
                //   (極 B では抑制区間を除いた評価区間・上の `evaluated_intervals`)。
                //   ⚠️ 区間の定義をここで作り直さない。
                observed_intervals: evaluated_intervals
                    .iter()
                    .map(|&(a_ms, b_ms)| ObservedIntervalDto { a_ms, b_ms })
                    .collect(),
                idle_trim: IdleTrimDto {
                    dropped_feat_rows: idle_trim_summary.dropped_feat_rows,
                    spans: idle_trim_summary
                        .spans
                        .iter()
                        .map(|&(a_ms, b_ms)| ObservedIntervalDto { a_ms, b_ms })
                        .collect(),
                },
                input_gap_trim: IdleTrimDto {
                    dropped_feat_rows: input_gap_trim_summary.dropped_feat_rows,
                    spans: input_gap_trim_summary
                        .spans
                        .iter()
                        .map(|&(a_ms, b_ms)| ObservedIntervalDto { a_ms, b_ms })
                        .collect(),
                },
            },
            config: FeatureConfigDto::from_resolved(
                &resolved_config,
                MAX_FEATURE_TICKS,
                modifier_scan.as_ref().map(|s| s.rows),
                modifier_scan.as_ref().map(|s| s.truncated_at_right_edge),
                modifier_scan.as_ref().map(|s| s.orphan_releases),
            )
            .with_pointer_evidence(
                pointer_declared,
                pointer_rows.as_ref().map(|rows| rows.len() as u64),
            ),
            rows: &rows,
        };
        // ⚠️★★★ 2026-09-18 改訂 (判断 24・security-checker LOW-3): `to_string_pretty` で
        // 一旦 `String` に全体を確保してから書き出すのをやめ、`to_writer_pretty` で
        // `BufWriter` へ直接流す (巨大な出力を 2 重に確保しない)。
        // ★ 2026-09-20 (起票 2): 流し込む対象が配列から封筒へ変わったが、
        //   `rows` を**参照で**持つので行を複製しない点は変わらない。
        let stdout = std::io::stdout();
        let mut writer = std::io::BufWriter::new(stdout.lock());
        if let Err(e) = serde_json::to_writer_pretty(&mut writer, &envelope) {
            tracing::error!("failed to serialize feature envelope: {}", e);
            return ExitCode::from(1);
        }
        if let Err(e) = writer.write_all(b"\n") {
            tracing::error!("failed to write feature envelope: {}", e);
            return ExitCode::from(1);
        }
        if let Err(e) = writer.flush() {
            tracing::error!("failed to flush feature envelope: {}", e);
            return ExitCode::from(1);
        }
        return ExitCode::SUCCESS;
    }

    // snapshot 経路 (デフォルト JSON 出力 / --snapshot 比較) は run_replay で
    // step() 再実行し決定論を検証する。こちらは従来通り。
    let snapshots = match run_replay(&args.session_path) {
        Ok(s) => s,
        Err(msg) => {
            tracing::error!("replay failed: {}", msg);
            return ExitCode::from(1);
        }
    };

    // stdout に JSON 配列を出力（データ出力、ログではない）
    let json = match serde_json::to_string_pretty(&snapshots) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to serialize snapshots: {}", e);
            return ExitCode::from(1);
        }
    };
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{}", json);
    ExitCode::SUCCESS
}

/// v0.4: NDJSON の `{"type":"paste", ...}` 行から `PasteEvent` 列を抽出する。
///
/// `external_paste_chars` が Some の paste 行のみ収集する (capture-schema-freeze 以降)。
/// 旧 NDJSON (フィールド欠落 → None) の paste は文字数不明のため outcome 集計から除外する
/// (`.steering/20260623-outcome-metrics/decisions.md` CD-OM3)。中身は一切読まない (数値のみ)。
/// 戻り値 = (capture 済 paste 列, 文字数欠落で除外した paste 件数)。後者は
/// `OutcomeMetrics.excluded_paste_count` に載せ、偏りの透明性を確保する。
/// `--features` 用: NDJSON の `{"type":"paste", ...}` 行の**時刻をすべて**返す。
///
/// ⚠️★★★ [`collect_paste_events`] と**用途が違う**ので分けてある。
/// ★ あちらは `external_paste_chars` を持つ行だけを集める (文字数が要る `--metrics` 用)。
/// ⚠️ `T0` の `pastes` は **件数**なので、文字数が欠落した行を落とすと
/// 「paste が無かった」と「paste は在ったが文字数が取れなかった」が `0` に潰れる
/// (★ 実測: `tests/fixtures/minimal.ndjson:22` は文字数の無い paste 行で、
///  分けなかった初版では全 tick が `pastes: 0` になっていた。2026-09-15・レビュー MEDIUM)。
/// ⚠️ 中身は一切読まない (時刻のみ)。
fn collect_all_paste_timestamps(session_path: &PathBuf) -> Result<Vec<u64>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<u64> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::Paste { t, .. } = ev {
            out.push(t);
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_all_paste_timestamps: some lines failed to parse as ReplayEvent"
        );
    }
    out.sort_unstable();
    Ok(out)
}

/// `--features` 専用の `ime_shadow` DTO。
///
/// ⚠️★★★ **`ReplayEvent::ImeShadow` は `serde_json::Value` のまま置き、構造化はここで行う。**
/// ★ `--snapshot` / `--metrics` の受理集合を 1 バイトも狭めないための分離であり、
/// **`feat` 行の [`LoggedFeatDto`] と同じ手法**である (`ReplayEvent::ImeShadow` の doc を参照)。
/// ★ フィールド名は `logger.rs::format_ime_shadow_line` の NDJSON 実形式に一致させる。
#[derive(Debug, serde::Deserialize)]
struct ImeShadowDto {
    t: u64,
    dwell_ms: u64,
    nav: u32,
    abandon: bool,
}

/// `--features` (`T1b`・issue 002) 用: NDJSON の `{"type":"ime_shadow", ...}` 行を
/// [`ImeShadowEvent`] 列として抽出する。中身は `dwell_ms` / `nav` / `abandon` の 3 値のみ
/// (IME 局面は既存 `ime_shadow` フィールドの範囲に閉じる。`.idea/survey §6.3` の追加候補は
/// `capability matrix` (ADR-032 M7) が先)。
///
/// ⚠️★★★ **形が合わない `ime_shadow` 行は捨てる (0 に畳まない)。**
/// ★ ここで `unwrap_or(0)` すると **「変換が 0 だった」と「値が取れなかった」が同じ 0 になる** ——
/// `capabilities.rs` 逐語「0 行は依然『判定不能』であって『信号が無かった』ではない」に反する。
/// → ★ 捨てたことは**件数だけ** `warn` に残す (⚠️ 行の中身は出さない)。
/// ⚠️ 「何行捨てたか」の下流への伝達は **issue 007 (`Q`) の担当**である (本 issue では出力に載せない)。
fn collect_all_ime_shadow_events(session_path: &PathBuf) -> Result<Vec<ImeShadowEvent>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<ImeShadowEvent> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut incomplete_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::ImeShadow(v) = ev {
            // ⚠️ 形が合った行だけを採る。欠け・型違いは 0 に畳まず捨てる (関数 doc 参照)。
            match serde_json::from_value::<ImeShadowDto>(v) {
                Ok(dto) => out.push(ImeShadowEvent {
                    t_ms: dto.t,
                    dwell_ms: dto.dwell_ms,
                    nav: dto.nav,
                    abandon: dto.abandon,
                }),
                Err(_) => incomplete_lines += 1,
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_all_ime_shadow_events: some lines failed to parse as ReplayEvent"
        );
    }
    if incomplete_lines > 0 {
        tracing::warn!(
            skipped = incomplete_lines,
            "collect_all_ime_shadow_events: ime_shadow lines whose dwell_ms/nav/abandon did not \
             match the logger format were skipped (not folded into 0)"
        );
    }
    out.sort_by_key(|e| e.t_ms);
    Ok(out)
}

/// `meta.capabilities` の**形と大きさ**を検査する (2026-09-18・security-checker LOW-3・判断 24)。
///
/// JSON オブジェクトでない、または直列化して [`MAX_CAPABILITIES_BYTES`] を超えるなら `Err`
/// (`--features` は exit 1)。★ 理由は出すが、**値の中身は出さない**。
/// ⚠️ `null` に倒さない —— 倒すと「宣言が無い」と誤報になる。
fn validate_meta_capabilities(
    cap: &serde_json::Value,
    lineno: usize,
) -> Result<serde_json::Value, String> {
    if !cap.is_object() {
        return Err(format!(
            "line {}: meta.capabilities is not a JSON object",
            lineno
        ));
    }
    let serialized = serde_json::to_string(cap)
        .map_err(|e| format!("line {}: failed to serialize capabilities: {}", lineno, e))?;
    if serialized.len() > MAX_CAPABILITIES_BYTES {
        return Err(format!(
            "line {}: meta.capabilities serializes to {} bytes, exceeds limit {} bytes",
            lineno,
            serialized.len(),
            MAX_CAPABILITIES_BYTES
        ));
    }
    Ok(cap.clone())
}

/// `--features` の封筒用: NDJSON の `meta` 行からセッション単位の事実を **1 パスで**取り出す
/// (起票 2・`capabilities` を含む)。
///
/// ⚠️★★★ **`ReplayEvent::Meta` に型付きフィールドとして持たせない**
/// (2026-09-18 改訂・security-checker LOW-1: 型付きフィールドを足したことで `run_replay`
/// 側の受理集合が狭まった)。★ 行をまず `serde_json::Value` として読む
/// (`ImeShadow` / `HookHealth` と同じ「まず Value で受け、構造化は専用の関数が担う」手法)。
/// これにより `--snapshot` (`run_replay`) 側の `Meta` 型は元の 3 フィールドのまま変わらない。
///
/// ★ 採るのは **`session_start` を持つ最初の `meta` 行**で、`capabilities` も**その同じ行**から取る。
///
/// ⚠️★★★ **2026-09-20 改訂 (封筒・code-reviewer MEDIUM)**: 旧版は `capabilities` を
/// 別関数 (`collect_meta_capabilities`) が**別の条件**——「`capabilities` を持つ最初の `meta` 行」
/// ——で拾っており、★ **同じ行である保証が無かった**。
/// ⚠️ 実測 (2026-09-20): ローテートで 2 セグメントを連結し、前半の `meta` に `capabilities` が
/// 無い NDJSON を食わせると、封筒が **前半の `session_start` / `hmm_version` / `app_version` と
/// 後半の `capabilities`** を並べて提示した。★ 封筒は「どの計器条件で取れた行か」を保証する
/// ために入れたので、ここが混ざるのは目的を直撃する。→ **1 行・1 パスに統合した。**
/// ★ 副産物として、`meta` 行が無い入力に対する**全行パースのフルパスが 1 本減った**。
///
/// ⚠️★★★ **欠けていても `Err` にしない。** キーが無い / 値の型が違う / `meta` 行そのものが
/// 無い場合は該当フィールドが `None` になるだけである (旧ログ互換。`hook_health` と同じ作法・
/// 007 の AC12)。★ ただし `capabilities` が**在るのに壊れている**ときだけは `Err`
/// ([`validate_meta_capabilities`]・判断 24・AC15) —— 「無い」と「壊れている」は別である。
fn collect_meta_session_facts(session_path: &PathBuf) -> Result<MetaSessionFacts, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut parse_failed_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let raw: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if raw.get("type").and_then(|t| t.as_str()) != Some("meta") {
            continue;
        }
        // ★ `session_end` 形の `meta` 行には `session_start` が無いので、ここで弾かれる。
        let Some(session_start) = raw.get("session_start").and_then(|v| v.as_u64()) else {
            continue;
        };
        // ⚠️★★★ **自由文の上限を課す** (2026-09-20・security-checker M1)。
        //   ★ 信頼境界: O1 は被験者端末で生成された NDJSON を研究者が復号して食わせるので、
        //     `meta` 行の中身は**被験者が編集しうる入力**である。
        //   ⚠️ 旧版は長さも文字種も無制限に封筒へ複写していた —— ★ 同じ `meta` 行から採る
        //     `capabilities` には 4096 バイトの上限があるのに、ここには何も無かった。
        //
        // ⚠️★★★ **超過は `Err`。`None` に倒さない** (2026-09-20 改訂・Codex /cross-review 1 巡目)。
        //   ★ 最初は「切り詰めずに `None`」にしたが、⚠️ **出力だけを見ると「未宣言」と
        //     「壊れた長大値」が区別できない**（tracing の警告はバッチ実行で流れる）。
        //   ★ `capabilities` と同じ規律に揃える —— **「無い」は `None` / 「壊れている」は `Err`**。
        //     ⚠️ 正常な値は semver / enum / レイアウト識別子で数十バイトなので、
        //     256 を超えるのは編集された入力だけである。
        let str_field = |k: &str| -> Result<Option<String>, String> {
            let Some(v) = raw.get(k).and_then(|v| v.as_str()) else {
                return Ok(None);
            };
            if v.len() > MAX_META_STRING_BYTES {
                // ★ 理由と大きさは出すが、⚠️ **値の中身は出さない** (`capabilities` と同じ作法)。
                return Err(format!(
                    "line {}: meta.{} is {} bytes, exceeds limit {} bytes",
                    lineno + 1,
                    k,
                    v.len(),
                    MAX_META_STRING_BYTES
                ));
            }
            Ok(Some(v.to_string()))
        };
        let capabilities = match raw.get("capabilities") {
            // ★ 在るなら形を検査する。⚠️ 壊れていれば `Err`（`null` に倒さない）。
            Some(v) if !v.is_null() => Some(validate_meta_capabilities(v, lineno + 1)?),
            // キーが無い、または明示的に `null`: 宣言が無い (⚠️ 空配列・空オブジェクトで埋めない)。
            _ => None,
        };
        return Ok(MetaSessionFacts {
            session_start: Some(session_start),
            hmm_version: str_field("hmm_version")?,
            app_version: str_field("app_version")?,
            g1_mode: str_field("g1_mode")?,
            o1_mode: str_field("o1_mode")?,
            keyboard_layout: str_field("keyboard_layout")?,
            capabilities,
        });
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_meta_session_facts: some lines failed to parse as JSON"
        );
    }
    Ok(MetaSessionFacts::default())
}

/// `--features` の封筒用: 入力ファイルの SHA-256 先頭 [`SOURCE_SHA256_HEX_CHARS`] 桁を返す。
///
/// ⚠️ **ファイル全体をメモリに載せない** —— 8 KiB ずつ読んで流し込む
/// (`MAX_SESSION_BYTES` は 500 MB を許すので、一括読みは OOM 経路になる)。
///
/// ★ 目的は「同じ入力かどうか」の同定であって改竄検知ではない。
fn sha256_prefix_of_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let file = File::open(path).map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf)
            .map_err(|e| format!("read error on {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    Ok(hex[..SOURCE_SHA256_HEX_CHARS].to_string())
}

/// `--features` の封筒用: 入力パスからファイル名だけを取り出す。
///
/// ⚠️ **ディレクトリを含めない** —— パスには利用者のホームディレクトリ名が含まれうる。
/// ★ ファイル名が取れない入力 (末尾が `..` 等) では `"<unknown>"` を返す
/// (⚠️ ここで `Err` にして `--features` を落とさない —— 由来が 1 つ欠けることは
/// 特徴行が出せない理由にならない)。
fn source_basename_of(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

/// `--features` (issue 007・`Q`) 専用の `hook_health` DTO。★ フィールド名は
/// `schema_dto.rs::LoggedLine::HookHealth` の NDJSON 実形式に一致させる。
/// `dropped_delta` は 2026-08-19 より前のログに無いので `#[serde(default)]` で欠落を許容する。
/// ⚠️★★★ 2026-09-18 改訂 (判断 24・code-reviewer M1): 欠落を `0` に正規化**しない**
/// (旧版はここで `0` に畳んでいたが、「値が無い」と「`0` だった」は区別できる情報であり、
/// `0` に畳むと情報が失われる)。`Option<u64>` のまま `quality::HookHealthEvent` へ渡し、
/// tick 単位の欠測判定 (`null` / 飽和加算) は `quality.rs` 側に委ねる。
#[derive(Debug, serde::Deserialize)]
struct HookHealthDto {
    t: u64,
    alive: bool,
    starved_ms: u64,
    rehook: bool,
    #[serde(default)]
    dropped_delta: Option<u64>,
}

/// `--features` (issue 007・`Q`) 用: NDJSON の `{"type":"hook_health", ...}` 行を
/// [`gse_next_lib::analysis::feat_offline::HookHealthEvent`] 列として抽出する。
///
/// ⚠️★★★ **形が合わない `hook_health` 行 (`alive` 欠落 / 型違い) は捨てる (0 に畳まない・AC12)。**
/// ★ `run_replay` 自体は `ReplayEvent::HookHealth` を `serde_json::Value` のまま読み飛ばすので
/// 落ちない (`ReplayEvent::HookHealth` の doc 参照)。ここは `--features` 専用の追加パースであり、
/// 捨てたことは件数だけ `warn` に残す (⚠️ 行の中身は出さない)。
fn collect_all_hook_health_events(
    session_path: &PathBuf,
) -> Result<Vec<gse_next_lib::analysis::feat_offline::HookHealthEvent>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<gse_next_lib::analysis::feat_offline::HookHealthEvent> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut incomplete_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::HookHealth(v) = ev {
            // ⚠️ 形が合った行だけを採る。欠け・型違いは 0 に畳まず捨てる (関数 doc 参照)。
            match serde_json::from_value::<HookHealthDto>(v) {
                Ok(dto) => out.push(gse_next_lib::analysis::feat_offline::HookHealthEvent {
                    t_ms: dto.t,
                    alive: dto.alive,
                    starved_ms: dto.starved_ms,
                    rehook: dto.rehook,
                    dropped_delta: dto.dropped_delta,
                }),
                Err(_) => incomplete_lines += 1,
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_all_hook_health_events: some lines failed to parse as ReplayEvent"
        );
    }
    if incomplete_lines > 0 {
        tracing::warn!(
            skipped = incomplete_lines,
            "collect_all_hook_health_events: hook_health lines whose shape did not match the \
             logger format were skipped (not folded into 0)"
        );
    }
    out.sort_by_key(|e| e.t_ms);
    Ok(out)
}

/// `--features` (issue 008) 専用の `feat` 行の `t` だけを取り出す DTO。
///
/// ⚠️★★★ `ReplayEvent::Feat` は `serde_json::Value` のまま置き、構造化はここで行う
/// (`ImeShadowDto` / `collect_all_ime_shadow_events` と同じ手法)。★ 形の合わない `feat` 行
/// (`t` 欠落 / 文字列 / `null`) を捨てても `run_replay` は影響を受けない
/// (`ReplayEvent::Feat` は既に `Value` なので、行の形に関わらずパース自体は必ず成功し、
/// `timestamp()` も `None` を返す)。
#[derive(Debug, serde::Deserialize)]
struct FeatTimestampDto {
    t: u64,
}

/// `--features` (issue 008・ADR-030 D8 (b-2)) 用: NDJSON の `{"type":"feat", ...}` 行の
/// **時刻だけ**を返す。★ `feat` 行の値 (`keys` / `bs_keys` 等) は 1 バイトも読まない
/// (D3 逐語「使ってよいのは feat 行の存在だけ」)。観測区間の判定にのみ使う。
///
/// ⚠️★★★ **形が合わない `feat` 行は捨てる (0 に畳まない)。** 件数だけ `warn` に残す
/// (`collect_all_ime_shadow_events` と同じ手法・判断 14 HIGH-1)。
fn collect_all_feat_timestamps(session_path: &PathBuf) -> Result<Vec<u64>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<u64> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut malformed_feat_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::Feat(v) = ev {
            match serde_json::from_value::<FeatTimestampDto>(v) {
                Ok(dto) => out.push(dto.t),
                Err(_) => malformed_feat_lines += 1,
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_all_feat_timestamps: some lines failed to parse as ReplayEvent"
        );
    }
    if malformed_feat_lines > 0 {
        tracing::warn!(
            skipped = malformed_feat_lines,
            "collect_all_feat_timestamps: feat lines whose t did not match the logger \
             format were skipped (not folded into a placeholder)"
        );
    }
    out.sort_unstable();
    Ok(out)
}

/// `--features --pole pointer_b` (issue 006) 用の `pointer_bin` 行の `t` だけを取り出す DTO。
///
/// ⚠️★★★ **`t` の 1 フィールドだけ**を持つ —— 移動量・ボタン・ホイールの値は**読まない**
/// (ゼロ抑制されているので**行の存在＝その秒に活動があった**であり、値は要らない・粗い量子化スカラでも触らない)。
/// ★ `Serialize` はテスト専用 (`cfg_attr`) —— `tests::pointer_bin_timestamp_dto_has_only_the_t_field`
/// が serde 出力キーが `["t"]` だけであることを固定する (フィールドが増える回帰を検出する門・LOW 是正)。
#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
struct PointerBinTimestampDto {
    /// **ビンの終端時刻** (`analysis/pointer_bin.rs`)。1 行が覆うのは `(t − 1000, t]`。
    t: u64,
}

/// `--features --pole pointer_b` (issue 006) 用: NDJSON の `{"type":"pointer_bin", ...}` 行の**時刻だけ**を返す
/// (昇順)。★ `collect_all_feat_timestamps` と同じ形・**`--features` 経路からだけ呼ぶ**
/// (`--snapshot` / `--metrics` / `--calibration` へは配線しない)。
///
/// ⚠️★★★ **形が合わない行は捨てる (0 に畳まない)。** 件数だけ `warn` に残す。
fn collect_pointer_bin_timestamps(session_path: &PathBuf) -> Result<Vec<u64>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let mut out: Vec<u64> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut malformed_lines: u64 = 0;
    for (lineno, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::PointerBin(v) = ev {
            match serde_json::from_value::<PointerBinTimestampDto>(v) {
                Ok(dto) => out.push(dto.t),
                Err(_) => malformed_lines += 1,
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_pointer_bin_timestamps: some lines failed to parse as ReplayEvent"
        );
    }
    if malformed_lines > 0 {
        tracing::warn!(
            skipped = malformed_lines,
            "collect_pointer_bin_timestamps: pointer_bin lines whose t did not match the \
             logger format were skipped (not folded into a placeholder)"
        );
    }
    out.sort_unstable();
    Ok(out)
}

/// `--features` の前処理 (事前宣言 v1.2 §6.1・`analysis::feat_offline::idle_trim`) が「入力」とみなす行の**時刻だけ**を返す (昇順)。
///
/// ★ 入力は **`key` 行 (押下・解放とも・`vk` / `category` を解決できない行も含む) と `pointer_bin` 行**だけ。
/// ⚠️ [`collect_key_events`] は使わない —— あちらは押下だけで、キーを解決できない行を捨てるので、
/// 「最後の `key` 行」とずれる。⚠️ `ime_*` / `focus_transition` / `clipboard` などは入れない
/// (システムが出しうる・研究者裁定 2026-10-08 判断 1)。
/// ★ 読むのは**行の種類と時刻だけ** ([`RowKindTimestampDto`]・`vk` / `category` / `press` / ポインタの値は読まない)。
/// → `press` などが欠けた `key` 行も入力に数える (行全体を [`ReplayEvent`] として読むと、読めない行が
///   「入力が無い」に化け、観測できていた `feat` 行まで削られる・Codex 指摘 2026-10-09)。
///
/// ⚠️ **fail-closed**: 種類が `key` / `pointer_bin` なのに時刻 `t` を非負の整数として読めない行があれば `Err`
/// (呼び出し側は exit 1)。入力の時刻が分からないまま削ると、削る範囲が黙って広がるため。
/// JSON として読めない行 (種類が分からない) は数えて警告し、読み飛ばす (他の収集関数と同じ)。
fn collect_idle_trim_input_timestamps(session_path: &PathBuf) -> Result<Vec<u64>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let mut out: Vec<u64> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    for (lineno, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let row: RowKindTimestampDto = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        let kind = row.kind.as_deref();
        if kind != Some("key") && kind != Some("pointer_bin") {
            continue;
        }
        match row.t.as_ref().and_then(serde_json::Value::as_u64) {
            Some(t) => out.push(t),
            // ⚠️ 値は出さない (行番号と種類だけ)。
            None => {
                return Err(format!(
                    "line {}: a `{}` row has no readable timestamp `t`; refusing to trim feat rows \
                     without knowing the input times (idle trim is fail-closed)",
                    lineno + 1,
                    kind.unwrap_or_default()
                ))
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_idle_trim_input_timestamps: some lines were not JSON objects and were skipped"
        );
    }
    out.sort_unstable();
    Ok(out)
}

/// [`collect_idle_trim_input_timestamps`] 用: 行の**種類と時刻だけ**を読む DTO (ほかのキーは読まない)。
/// ★ `t` は `Value` で受け、非負の整数かを呼び出し側で確かめる (型の違いを「行が無い」に化けさせない)。
#[derive(Debug, serde::Deserialize)]
struct RowKindTimestampDto {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    t: Option<serde_json::Value>,
}

/// 修飾キーの `key` 行から読んだ edge (`(時刻, press なら true)`) と、読み飛ばした行の件数。
#[derive(Debug, Default)]
struct ModifierEdges {
    /// **解決後の `KeyCategory` が `Modifier`** の `key` 行 (press と release の**両方**)。
    edges: Vec<(u64, bool)>,
    parse_failed_lines: u64,
    /// `vk` も既知の `category` も無く、種別を解決できなかった `key` 行。
    unresolved_keys: u64,
}

/// NDJSON から修飾キーの edge を 1 パスで集める ([`collect_modifier_intervals`] の読み取り部)。
///
/// ★ 旧ログ (`vk`) も匿名化ログ (`category`) も [`resolve_key_vk`] を通して同じ規則で判定する
/// (`collect_key_events` と同じ解決)。⚠️ **release の行も読む** —— `collect_key_events` は press しか
/// 残さないので、押下区間はそちらからは復元できない (ADR-030 D3「`key` 行は press と release の両方が出る」)。
fn read_modifier_edges(reader: impl BufRead) -> Result<ModifierEdges, String> {
    let mut out = ModifierEdges::default();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                out.parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::Key {
            t,
            vk,
            category,
            press,
        } = ev
        {
            match resolve_key_vk(vk, category.as_deref()) {
                Some(resolved) if KeyCategory::from_vk(resolved) == KeyCategory::Modifier => {
                    out.edges.push((t, press));
                }
                Some(_) => {}
                None => out.unresolved_keys += 1,
            }
        }
    }
    Ok(out)
}

/// 修飾キーの走査結果 (`--pole modifier_m_plus` 用)。
#[derive(Debug)]
struct ModifierScan {
    /// 修飾キーの `key` 行数 (press と release の合計)。★ 極 M+ の適用可否の判定式 (判断 13)。
    rows: u64,
    /// 押下区間 (`stall_onset::modifier_press_intervals`・両端を含む)。
    intervals: Vec<(u64, u64)>,
    /// 先行する press が無い release の件数 (`decisions.md` 判断 24 の健全性カウンタ・
    /// `stall_onset::ModifierIntervals::orphan_releases` の写し)。
    orphan_releases: u64,
    /// 観測区間の右端で打ち切られた押下区間の本数 (`decisions.md` 判断 24 の健全性カウンタ・
    /// `stall_onset::ModifierIntervals::truncated_at_right_edge` の写し)。
    truncated_at_right_edge: u64,
}

/// `--features --pole modifier_m_plus` (issue 005) 用: 修飾キーの**押下区間**を 1 パスで作る。
///
/// ⚠️★★★ `KeyPress` (`metrics.rs`・HMM 経路と共有) は release を持たないので、押下区間は
/// `collect_key_events` からは作れない。★ `KeyPress` を変えず、専用の走査を足す
/// (`--snapshot` / `--metrics` / `--calibration` へ波及させない)。**`--features` 経路からだけ呼ぶ。**
/// ⚠️ 区間の作り方 (カウント方式・観測区間の右端で打ち切り) は
/// `stall_onset::modifier_press_intervals` が SSOT。`observed` はその打ち切りに使う観測区間。
///
/// ⚠️ 形の合わない行は捨てて**件数だけ `warn`** する (0 に畳まない・`collect_all_feat_timestamps` と同じ手法)。
fn collect_modifier_intervals(
    session_path: &PathBuf,
    observed: &[(u64, u64)],
) -> Result<ModifierScan, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let scanned = read_modifier_edges(BufReader::new(file))?;
    if scanned.parse_failed_lines > 0 {
        tracing::warn!(
            failed = scanned.parse_failed_lines,
            "modifier scan: some lines failed to parse as ReplayEvent"
        );
    }
    if scanned.unresolved_keys > 0 {
        tracing::warn!(
            skipped = scanned.unresolved_keys,
            "modifier scan: key entries with neither vk nor a known category were skipped"
        );
    }
    let built = gse_next_lib::analysis::feat_offline::stall_onset::modifier_press_intervals(
        &scanned.edges,
        observed,
    );
    if built.orphan_releases > 0 {
        // ⚠️ 孤立 release (先行する press が無い) は区間を作らない。件数だけ残す。
        tracing::warn!(
            orphan_releases = built.orphan_releases,
            "modifier scan: modifier release rows with no preceding press made no interval"
        );
    }
    if built.truncated_at_right_edge > 0 {
        tracing::warn!(
            truncated = built.truncated_at_right_edge,
            "modifier scan: modifier presses still held at an observed interval's right edge were closed there"
        );
    }
    Ok(ModifierScan {
        rows: scanned.edges.len() as u64,
        intervals: built.intervals,
        orphan_releases: built.orphan_releases,
        truncated_at_right_edge: built.truncated_at_right_edge,
    })
}

/// `--features` (issue 008) 用: NDJSON の `{"type":"session_meta", ...}` 行から
/// `(ts, task_kind)` の宣言列を抽出する。
///
/// ⚠️ 既存 `run_replay` の「1 ファイルに相異なる `task_kind` が複数あれば Err」
/// (`declared_task_kinds` の検査) はここでは**適用しない** —— こちらは `stall_onset`
/// が `s` 時点で有効な宣言を引くための時系列そのものが要るので、複数の宣言を
/// そのまま返す (run_replay 側の制約とは別の用途)。
///
/// ⚠️★★★ **`task_kind` の検証は `logger::TaskKind::parse` を SSOT として通す**
/// (`run_replay` の `domain` 解決と同じ経路。新しい許可リストを作らない)。`None` になる
/// 未知の値は宣言として**採らない** (= その時点は未宣言のまま back-off する。値そのものは
/// 出力に出さず、捨てた件数だけ `warn` する)。
fn collect_task_kind_declarations(session_path: &PathBuf) -> Result<Vec<(u64, String)>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut unknown_task_kind_lines: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::SessionMeta { task_kind, ts, .. } = ev {
            match TaskKind::parse(&task_kind) {
                Some(kind) => out.push((ts, kind.as_str().to_string())),
                None => {
                    unknown_task_kind_lines += 1;
                }
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            failed = parse_failed_lines,
            "collect_task_kind_declarations: some lines failed to parse as ReplayEvent"
        );
    }
    if unknown_task_kind_lines > 0 {
        // ⚠️ 値そのものは出さない (件数だけ)。
        tracing::warn!(
            skipped = unknown_task_kind_lines,
            "collect_task_kind_declarations: some session_meta declarations had a task_kind \
             unrecognized by logger::TaskKind::parse and were dropped (treated as undeclared)"
        );
    }
    out.sort_by_key(|(ts, _)| *ts);
    Ok(out)
}

/// v0.4: NDJSON の `{"type":"key", ...}` 行から press イベントを `KeyPress` 列として抽出する。
///
/// **旧ログ**は `vk` を保持し、**現行の logger と匿名化 NDJSON** は `category` を書く。
/// どちらも `resolve_key_vk` が吸収する (`vk` を優先し、無ければ代表 VK へ逆写像)。
/// Unicode 内容には触れない。
///
/// ⚠️★★★ **2026-08-23 (ADR-014 判断 6) で D-RC3 の過小評価は解消した。**
/// `KeyCategory` に `Punctuation` / `SentencePunctuation` / `Tab` を足したので、
/// **`is_text_producing` が true な VK は 1 つ残らず非 `Other` の category に入る**。
/// → **`is_backspace` も `is_text_producing` も raw と完全一致する**
/// (代表 VK `other => 0x07` は「保守的な倒し込み」ではなく**厳密に正しい**値になった)。
///
/// **旧仕様 (2026-08-23 以前)**: OEM 句読点 (0xBA-0xC0/0xDB-0xDF/0xE2)・Tab(0x09)・
/// テンキー演算子(0x6A-0x6F) が `Other` へ畳まれ、匿名化後に text-producing と判別できず、
/// `--metrics` の manual_insert 系がその件数だけ過小評価されていた
/// (`.steering/20260624-replay-category/decisions.md` D-RC3)。
/// `manual_contribution_ratio` は `research-positioning.md §5` **H-Y の主要アウトカム**である。
///
/// ★ **`--snapshot` / 特徴量再計算 (D6) には元から無影響** (run_replay は is_text_producing を使わない)。
fn collect_key_events(session_path: &PathBuf) -> Result<Vec<KeyPress>, String> {
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }
    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);
    let mut out: Vec<KeyPress> = Vec::new();
    let mut parse_failed_lines: u64 = 0;
    let mut unresolved_keys: u64 = 0;
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                parse_failed_lines += 1;
                continue;
            }
        };
        if let ReplayEvent::Key {
            t,
            vk,
            category,
            press,
        } = ev
        {
            if press {
                match resolve_key_vk(vk, category.as_deref()) {
                    Some(resolved_vk) => out.push(KeyPress {
                        timestamp: t,
                        vk: resolved_vk,
                    }),
                    None => unresolved_keys += 1,
                }
            }
        }
    }
    if parse_failed_lines > 0 {
        tracing::warn!(
            skipped = parse_failed_lines,
            "collect_key_events: NDJSON lines failed to parse as ReplayEvent"
        );
    }
    if unresolved_keys > 0 {
        tracing::warn!(
            skipped = unresolved_keys,
            "collect_key_events: key entries with neither vk nor a known category were skipped"
        );
    }
    out.sort_by_key(|k| k.timestamp);
    Ok(out)
}

/// NDJSON を読み込み、step() を再実行して FeatSnapshot 列を返す。
fn run_replay(session_path: &PathBuf) -> Result<Vec<FeatSnapshot>, String> {
    // ファイルサイズガード (OOM 防止、security-checker MEDIUM)
    let metadata = std::fs::metadata(session_path)
        .map_err(|e| format!("cannot stat {}: {}", session_path.display(), e))?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err(format!(
            "session file too large: {} bytes exceeds limit {} bytes",
            metadata.len(),
            MAX_SESSION_BYTES
        ));
    }

    let file = File::open(session_path)
        .map_err(|e| format!("cannot open {}: {}", session_path.display(), e))?;
    let reader = BufReader::new(file);

    let mut events: Vec<ReplayEvent> = Vec::new();
    let mut meta_hmm_version: Option<String> = None;

    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read error at line {}: {}", lineno + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }
        let ev: ReplayEvent = serde_json::from_str(&line)
            .map_err(|e| format!("parse error at line {}: {}", lineno + 1, e))?;
        if let ReplayEvent::Meta {
            hmm_version: Some(v),
            ..
        } = &ev
        {
            meta_hmm_version = Some(v.clone());
        }
        events.push(ev);
    }

    // hmm_version 検証（不一致は WARN のみ、exit はしない）
    if let Some(v) = &meta_hmm_version {
        if v != HMM_VERSION {
            tracing::warn!(
                "session hmm_version = {}, current engine = {}. snapshots may not be comparable.",
                v,
                HMM_VERSION
            );
        }
    }

    // session_start を決定: Meta.session_start → SessionStart.t → 最初のイベントの t
    let session_start_ms = events
        .iter()
        .find_map(|e| match e {
            ReplayEvent::Meta {
                session_start: Some(t),
                ..
            } => Some(*t),
            _ => None,
        })
        .or_else(|| {
            events.iter().find_map(|e| match e {
                ReplayEvent::SessionStart { t } => Some(*t),
                _ => None,
            })
        })
        .or_else(|| events.iter().find_map(|e| e.timestamp()))
        .ok_or_else(|| "no events with timestamps found".to_string())?;

    // ★ ADR-006 Phase 2b: 宣言された task_kind が domain head を選ぶ。
    //   ⚠️ **この抽出を省くとリプレイが本番の挙動を再現できない** (session_meta 行は
    //     timestamp() == None で下のソート対象から落ちるため、メインループでは見えない)。
    //   ⚠️ ファイルの 2 パス読みではない。events は既に全行メモリ上にある。
    //   task_kind が無い / 未知の場合は Writing head へフォールバックする
    //   (既存 Phase1 ログはそもそも session_meta を持たない)。
    let declared_task_kinds: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            ReplayEvent::SessionMeta { task_kind, .. } => Some(task_kind.as_str()),
            _ => None,
        })
        .collect();

    // ⚠️★ **1 ファイルに相異なる task_kind が複数あるなら、黙って先頭勝ちにしない。**
    //   非 G1 モードではログをローテートしないため、「開始 → 戻る → 別 task_kind で開始」で
    //   1 ファイルに複数の session_meta が並びうる。本番は 2 個目で head を差し替えるが、
    //   replay は区間を持たないので先頭を全区間へ適用してしまい、**本番挙動を再現しない**。
    //   再現できないものを再現したふりをするのが最悪なので、明示的に落とす。
    //   (security-checker 指摘・2026-08-18)
    let mut distinct: Vec<&str> = declared_task_kinds.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() > 1 {
        return Err(format!(
            "session_meta declares {} different task_kinds in one file ({:?}) —              replay cannot reproduce per-session head switching. Split the file per session.",
            distinct.len(),
            distinct
        ));
    }

    // ⚠️ session_meta が 1 種類でも、session_start が複数あるなら同じ理由で再現できない
    //   (本番は start_session ごとに engine.reset() = head を Writing へ戻す)。
    let session_start_count = events
        .iter()
        .filter(|e| matches!(e, ReplayEvent::SessionStart { .. }))
        .count();
    if !declared_task_kinds.is_empty() && session_start_count > 1 {
        return Err(format!(
            "file has {session_start_count} session_start rows with a session_meta declaration —              production resets the head on each start, replay cannot. Split the file per session."
        ));
    }

    let domain = match declared_task_kinds.first() {
        None => TaskDomain::Writing,
        Some(s) => match TaskKind::parse(s) {
            Some(kind) => TaskDomain::from(kind),
            None => {
                // ⚠️ 全長を出さない (1 行は最大 MAX_SESSION_BYTES まで許容されるため)。
                let shown: String = s.chars().take(32).collect();
                tracing::warn!(
                    "unknown task_kind {:?} in session_meta — falling back to writing head",
                    shown
                );
                TaskDomain::Writing
            }
        },
    };

    // 時刻順にソート（既存 Feat は timestamp() == None で除外）
    let mut ordered: Vec<ReplayEvent> = events
        .into_iter()
        .filter(|e| e.timestamp().is_some())
        .collect();
    ordered.sort_by_key(|e| e.timestamp().unwrap_or(0));

    // state + engine を初期化
    let mut state = AnalysisState::new(session_start_ms, 600);
    let engine = CognitiveStateEngine::new_for_domain(domain);
    let mut ime_open = false;
    let mut snapshots: Vec<FeatSnapshot> = Vec::new();
    // vk も category も解決できなかった不正 key 行の件数 (内容は出さず件数のみ warn)。
    let mut unresolved_keys: u64 = 0;

    // 各イベントの前に、そのイベントの時刻までゲートを進める
    for ev in ordered {
        let ev_ms = ev.timestamp().unwrap();

        // イベント時刻に達するまでゲートを進める (1Hz 推論を仮想時刻で駆動)
        while state.next_engine_update_ms <= ev_ms {
            let tick_ms = state.next_engine_update_ms;
            let out = step(&mut state, &engine, None, tick_ms, ime_open);
            if let Some(f) = out.feat {
                snapshots.push(f);
            }
            // 無限ループ防止セーフティガード: step() が next_engine_update_ms を
            // 進めなかった場合（通常は engine pause で gate スキップ時）。
            // 通常運用では発生しないが、step() の実装に回帰があった際は
            // tracing::error + debug_assert でバグとして可視化する。
            if state.next_engine_update_ms == tick_ms {
                tracing::error!(
                    tick_ms = tick_ms,
                    "step() did not advance next_engine_update_ms — forcing +ENGINE_TICK_MS"
                );
                debug_assert!(
                    false,
                    "step() must advance next_engine_update_ms on each gate tick"
                );
                state.next_engine_update_ms = tick_ms + ENGINE_TICK_MS;
            }
        }

        // イベント処理
        match ev {
            ReplayEvent::Meta { .. }
            | ReplayEvent::Feat(_)
            | ReplayEvent::Paste { .. }
            | ReplayEvent::UserLabel { .. }
            | ReplayEvent::ReviewMarker { .. }
            | ReplayEvent::ImeShadow(_)
            | ReplayEvent::InterventionDecision(_)
            | ReplayEvent::InterventionResponse(_)
            | ReplayEvent::HookHealth(_)
            | ReplayEvent::StarveWindow(_)
            // ⚠️★★★ O1 (R3) の 4 行種は **読み飛ばす**。★ 特徴量へ流さない (ADR-014 D4b)。
            | ReplayEvent::RecordingPause(_)
            | ReplayEvent::FocusTransition(_)
            | ReplayEvent::PointerBin(_)
            | ReplayEvent::SecureInput(_)
            // EH3 の 2 行種も読み飛ばす。
            | ReplayEvent::Eh3Day(_)
            | ReplayEvent::InvitationPresent(_)
            // ⚠️★★★ 監査 criterion (ADR-031)。★ 記録のみで推論へ流さない。
            | ReplayEvent::ActivityLabel(_)
            | ReplayEvent::SessionMeta { .. }
            | ReplayEvent::ProbeAssignment { .. }
            | ReplayEvent::RealtimeLabel { .. }
            | ReplayEvent::VignetteResponse { .. } => {
                // Paste は回数記録専用で features/HMM に未使用 → 追加処理なし。
                // Meta / Feat / UserLabel / ReviewMarker はゲート進行済み、追加処理不要
                // (UserLabel は metrics 経路で collect_user_labels 側で抽出)
                // (ReviewMarker は --segment 経路でセグメントハイライトに使用)
                // (v0.3 較正 variant は --calibration 経路の collect_calibration_session で抽出)
                // これらは timestamp()==None で ordered から除外されるが、match の網羅性のため列挙する。
            }
            ReplayEvent::SessionStart { t } => {
                // 本番の start_session リセットを再現
                state.reset(t, 600);
            }
            ReplayEvent::Key {
                t,
                vk,
                category,
                press,
            } => {
                // raw は vk、匿名化は category。代表 VK を解決して従来通り step する。
                let resolved_vk = match resolve_key_vk(vk, category.as_deref()) {
                    Some(v) => v,
                    None => {
                        // vk も category も無い / 未知 category の不正 key 行。特徴量を汚さない
                        // よう step せずスキップする (ゲートは前段で進行済み)。件数は後段で warn。
                        unresolved_keys += 1;
                        continue;
                    }
                };
                let event = InputEvent {
                    vk_code: resolved_vk,
                    timestamp: t,
                    is_press: press,
                    is_repeat: false, // step() が pressed_keys で検出する
                    is_backspace: is_backspace(resolved_vk),
                    is_paste: false, // replay は key 行から再構成。paste は別 variant で記録済み
                };
                let out = step(
                    &mut state,
                    &engine,
                    Some(LoopInput::Key(event)),
                    t,
                    ime_open,
                );
                if let Some(f) = out.feat {
                    snapshots.push(f);
                }
            }
            ReplayEvent::ImeState { t, on, active } => {
                ime_open = on;
                let out = step(
                    &mut state,
                    &engine,
                    Some(LoopInput::ImeActivate { active }),
                    t,
                    ime_open,
                );
                if let Some(f) = out.feat {
                    snapshots.push(f);
                }
            }
        }
    }

    if unresolved_keys > 0 {
        tracing::warn!(
            skipped = unresolved_keys,
            "run_replay: key entries with neither vk nor a known category were skipped \
             (malformed or future KeyCategory — features unaffected)"
        );
    }

    Ok(snapshots)
}
