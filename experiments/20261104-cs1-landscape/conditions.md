# CS1 の俯瞰の条件（計算の前にコミットする・2026-11-04）

元の俯瞰は `experiments/20261102-public-data-landscape/conditions.md`（以下「元の条件」）。ここでは CS1 打鍵データ [V243] に、元の条件の L1〜L6 を同じ定義で当てる。書いていないことは元の条件に従う。記述だけで、検定をしない。判定の閾値を置かない。GSE の参加者のデータではない。ゲートに載せない。

この時点で見たのは、列名と、出来事の種類（`EventType`・`EditType`）ごとの件数と、キーの名前の列の値の形だけ（`.steering/20261104-cs1-landscape/blockers.md` に、見えてしまったものを書いた）。人ごと・課題ごとの件数、時刻の間隔、長さは見ていない。

## D1 データ

- 2019 年版（doi:10.7910/DVN/6BPCXN・Dataverse 版 5.2）: `~/gse-public-data/cs1/raw/2019/keystrokes.csv`。SHA-256 が `b54d8cf56b7719ae0ec9ecf19e5b3788168f087bd4a50238dca1e06f6bfe7955` でなければ止める。
- 2021 年版（doi:10.7910/DVN/BVOF7S・Dataverse 版 7.0）: `~/gse-public-data/cs1/raw/2021/keystrokes.csv`。SHA-256 が `950a587bb3a410efc4aad0e64974d6442806345fddd13af76676dfb8006a22b6` でなければ止める。
- 読む列: `EventID`・`SubjectID`・`AssignmentID`・`EventType`・`EditType`・`ClientTimestamp` と、キーの名前の列（2019 年は `X-Keystroke`、2021 年は `X-Keystroke` と `X-Action` と `X-Attention` の行の `X-Metadata`）。`InsertText` は長さだけを使う。`DeleteText`・`X-RunInput`・`X-RunOutput`・`SourceLocation`・`CodeStateSection`、`X-Paste`・`X-Copy` の行の `X-Metadata` は読まない（2021 年の `X-Paste`・`X-Copy` の `X-Metadata` は貼り付けた文字そのもの）。
- 1 文字のキーは、カテゴリを決めるためにメモリの中で読むだけで、書き出さない（CoAuthor の `convert.py` と同じ）。
- 並べ方は `ClientTimestamp`、同じ時刻は `EventID` の昇順（readme の指示）。

## D2 単位

- 書き手 = `SubjectID`。課題 = (`SubjectID`, `AssignmentID`)。2019 年の `AssignmentID` は学期の印（s/f）付きなので、2 学期とも受けた人の同じ課題は別の課題になる。
- 作業のまとまり: 課題の中のすべての出来事（D3 で使う種類のすべて）の時刻を並べ、隣接する差が 30 分（1,800,000 ms）以上のところで切る。30 分は P1 の `idle_trim_input_gap_ms`・`idle_trim_max_ms` と同じ値。
  - 理由: 合成の入力で確かめたところ、replay は打鍵の無い 2 時間のあいだも 1 Hz の feat 行を出し続け、穴が無かった。課題（何日にもまたがる）をそのまま渡すと、夜の間が「観測していて手が止まっている」になる。
  - まとまりごとに `convert` → replay（既定）を回し、課題の中のまとまりの feat 行と key 行を 1 本の NDJSON にまとめる（`add_feat.py` と同じ形）。それを replay `--features` に 1 回通す。観測区間はまとまりの間で切れ（feat 行の穴 > `hole_gap_ms`）、θ は課題の中で育つ（元の条件の C3 の「作業の中で θ が育つ」を、課題に当てる）。
  - 押下が 2 未満のまとまりは捨てる（数を報告する）。

## D3 変換の対応表（件数を見る前に固定）

key 行は時刻と category だけ。離しは押下の 100 ms 後に合成し、次の押下の 1 ms 前で切る（CoAuthor と同じ）。

### 2019 年版（Phanon・ブラウザの自作 IDE・補完なし）

| 出来事 | key 行 | 出来事の表 |
|---|---|---|
| `File.Edit`・`Insert`・`X-Keystroke` が 1 文字 | その文字の category（`convert.py` の `char_category`） | — |
| `File.Edit`・`Insert`・`X-Keystroke` が `space` / `return` | `space` / `enter` | — |
| `File.Edit`・`Delete`・`X-Keystroke` が `delete` | `backspace`（Backspace と Delete を分けられない） | — |
| `File.Edit`・`Undo` / `Redo`・`X-Keystroke` あり | `other` | — |
| `File.Edit`・`X-Keystroke` が無い（`nan`）`Insert`・`Delete`・`Undo`・`Redo` | — | `auto-edit`（キーに依らない編集。数だけ） |
| `File.Edit`・`Paste` | — | `paste` |
| `File.Edit`・`Cut` / `Drag` | — | `cut` / `drag` |
| `File.Edit`・`X-Compose` / `X-Checkpoint` | — | 数だけ（どちらにも入れない） |
| `Run.Program` | — | `run` |
| `Submit` | — | `submit` |
| `X-SwitchTask` | — | `switch`（課題の中の 2 つの問題の切り替え） |

2019 年版には移動のキー（矢印など）の記録が無い。編集を起こしたキーだけが記録されている。

### 2021 年版（PyCharm と PyPhanon・補完・括弧の自動の閉じあり）

| 出来事 | key 行 | 出来事の表 |
|---|---|---|
| `X-Keystroke`（`X-Metadata` が 1 文字） | その文字の category | — |
| `X-Action`・`Backspace` / `Delete` | `backspace` / `delete` | — |
| `X-Action`・`Left` / `Right` / `Up` / `Down` / `Home` / `End` / `PageUp` / `PageDown` | `navigation` | — |
| `X-Action`・`Enter` / `Tab` / `Escape` | `enter` / `tab` / `escape` | — |
| `X-Action`・`Paste` / `Copy` / `Cut` / `Undefined` | `other` | — |
| `X-Action`・`EditorChange` | — | `switch`（エディタのタブの切り替え） |
| `X-Action`・上に無いもの（`Reformat`・`Move`・複数の語の名前ほか） | — | `ide-action`（数だけ） |
| `File.Edit`・`Insert`・`InsertText` の長さが 2 以上 | — | `multi-insert`（補完・括弧の自動の閉じ・貼り付けの本体。打鍵の列に入れない） |
| `File.Edit`（それ以外） | — | — |
| `X-Paste` | — | `paste` |
| `X-Copy` | — | `copy` |
| `Run.Program`・`X-Metadata` が `Start` | — | `run` |
| `Run.Program`（それ以外）・`X-ReplayAction` | — | 数だけ |
| `X-Attention`・`Lockout` | — | `prompt`（課題に取り組んでいるかの問いの表示と推定する。D5） |
| `X-Attention`・`ON` / `OFF` | — | `prompt-answer`（答えと推定する。答えの値は集計に使わない） |

## D4 取っておく書き手（見ない）

- 集計の前に、年版ごとに書き手の 3 割を `SEED`（20261104）で取っておく。`SubjectID` を昇順に並べた一覧から、2019 年は `random.Random(SEED).sample(一覧, round(0.3 × 人数))`、2021 年は `random.Random(SEED + 1)`。
- 一覧は `~/gse-public-data/cs1/holdout.json`（repo の外）。repo には人数と一覧の SHA-256 だけを出す。集計の入力に一覧の書き手が 1 人も無いことを確かめる（1 人でもあれば止める）。

## D5 共通の定義（元の条件の C4 から変えるところ）

- 押下の間隔: 同じ作業のまとまりの中の隣接する押下の差。まとまりの間の差（30 分以上）は入れず、まとまりの数を別に出す。1 時間あたりの数は、まとまりの長さ（最初の押下から最後の押下まで）の和で割る。
- 2021 年の「問いの近く」: `prompt` の時刻から、次の `prompt-answer` の時刻まで（10 分以内に答えが無ければ `prompt` から 60 秒）を問いの区間とする。押下の間隔・停滞は、区間 [始まり, 終わり]（停滞は [s − 30 秒, 再開]）が問いの区間に少しでもかかれば「問いの近く」、かからなければ「ふつう」。主はふつうとし、全体も併記する（CoAuthor の「AI の近く」と同じ扱い。問いの表示が打鍵を止めうるため）。L4・L5・L6 は全体で数える（元の条件と同じ）。2019 年には問いが無いので全体だけ。
- 書き手の観測時間が 10 分未満の書き手は集計から除き、数を報告する（元の条件と同じ）。
- 集計の単位: 書き手ごとの値を出し、書き手の分布（中央値と四分位）で示す。

## D6 問いと集計（元の条件の C5 から変えるところ）

- L1・L2: 元の条件と同じ。最後の θ は課題ごとの最後の θ の、書き手の中央値。
- L3: 停滞の前と後の窓で、次があった停滞の割合。文字の入力でない押下・削除・移動のキー（2021 年だけ）・`run`・`switch`・`paste`。クリックは無い。
- L4: 受領の出来事は `paste` と `multi-insert`（2021 年だけ。書き手の分布は別々に）。AI の提案は無い（AI 以前）。受領の数え方は元の条件と同じ。180 秒以上の途切れも同じ（まとまりの中だけ）。
- L5: 課題を単位にする。対象は、θ が定義され観測時間が 5 分以上の課題が 5 つ以上ある書き手。課題ごとの停滞の 1 時間あたりの数と最後の θ。元の条件と同じ分解（間の SD・中の SD の平均・級内の割合）と、ポアソンで数え上げだけから出る中の SD を出す。
- L6: 課題の観測時間を前・中・後に 3 等分し（観測区間をつなげた時間で切る）、それぞれで停滞の 1 時間あたりの数・`p_stuck ≥ 0.60` の割合・`ewma_x` の中央値・θ が定義された時間の割合。書き手の値は課題の値の中央値。
- 単位の記述（改善の試験で 2021 年版を使うかを選ぶ材料）: 書き手ごとの課題の数、課題の観測時間・まとまりの数・まとまりの長さの四分位、課題の span（最初から最後の出来事まで・日）の四分位。

## D7 言える範囲（先に置く）

- 停滞の始まりは本人にとって長い正味の産出の止まりで、詰まりの判定ではない（ADR-030 の 9b）。
- CS1 は英語の Python のプログラミングの課題・米国の 1 大学の入門の授業・AI 以前・2019 年はブラウザの自作 IDE（補完なし・移動のキーの記録なし）・2021 年は PyCharm（補完・括弧の自動の閉じあり）。日本語・IME・GSE の参加者・文章を書く作業・アプリをまたぐ作業では言えない。
- 作業のまとまりは 30 分の空白で切った推定で、本人が作業をやめた時刻ではない。30 分未満の離席はまとまりの中に入り、観測していて手が止まっている時間として数える。
- 2021 年の `X-Attention` を問いとその答えと読むのは推定（論文は列を説明していない。表示と答えの数がほぼ同じであることと、研究が問いを出したと書いていることによる）。
- 2021 年の `paste` は課題の外からの貼り付けと、同じファイルの中の切り取りと貼り付けを分けられない。論文は、別の IDE で書いたコードを貼り付けた学生がいたと書いている。ADR-024 の受領と同じ量ではない。
- 推定（`p_stuck`・`ewma_x`）はこの集団で較正されていない。
