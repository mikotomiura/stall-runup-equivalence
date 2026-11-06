# 登録の外の探索 X16・X17（論文 1 の attack-me manuscript round 3 の直し・計算の前にコミットする・2026-11-07）

事前登録（https://osf.io/mrkxt・逸脱 1 https://osf.io/fqgcp）の取り置きを開けた後の、登録の外の探索 [E-H]。判定に使わない。結果は集計の値だけを `results/explore_round3.json` に書き、書き手ごとの値は出さない。研究者の採否は `.steering/20261105-public-data-findings-paper/decisions.md` 判断 38。

## 共通

- 中間: 取り置きの中間 `~/gse-public-data/cs1/work2019_holdout/`（確認の完走で作ったもの）を確認のスクリプト `experiments/20261105-findings-paper1-prefreeze/scripts/confirm_p1.py`（manifest c821ae03…f6b0 の版）の `load_cs1`（mode=holdout）で読む。取り置きの手順（`run-confirm-holdout.sh`）は回さない。
- 行: 主の設定 MAIN の `h1_writer` を、対照の引きの SEED 20261106（X14・X15 と同じ）で回す。停滞の行に s（条件 A が真になった時刻）を添えるため、`h1_writer` と同じ順で同じ乱数を使う写しをスクリプトに置き、門で元の関数と行が同じことを確かめる。
- 門 G0: 写しの行（区切り・層・信号・対照の id）が `h1_writer` の行と全件同じ、件数が confirm_holdout.json の h1_counts と同じ。

## X16 尺度づけ φ の床と天井（R1）

- 推定器の 5 つの量 f1・f3・f4・f5・f6 は中間に残っていないので、取り置きの各課題を、中間を作ったときと同じ区切り（`blocks`）ごとに、同じ入力の作り方（`build_cs1.py` の `key_rows` と `write_ndjson`）で、論文の replay（`~/gse-public-data/klicke/bin/gse-replay`・SHA-256 26cae610…33bb3）に引数なしでかけ直す。
- 門 G1: 区切りの出力をつないだ (timestamp, p_stuck, ewma_x) が、中間の feat と全課題で全行同じ（浮動小数の完全一致）。
- 門 G2: 補足 S3 の式 φ(f, β) = min(1, max(0, (f − β)/2β))（β = 150 ms・0.06・2.0・0.08 を f1・f3・f5・f6 に）から作った x = 0.30 φ3 + 0.25 φ6 + 0.25 φ1 + 0.20 φ5（[0, 1] に切る）が、replay の raw_x と全行で 1e-9 以内。
- 数えるもの: 主の層（事象の無い層）の停滞の行と、引いた対照（区切り・層の組ごとに異なる対照の id を 1 回）について、u と u − 30 秒の行（`sig4` と同じ snap）の φ1・φ3・φ5・φ6 が 0 の割合と 1 の割合、30 秒の変化 Δφ が厳密に 0 の割合、4 つとも変化 0 の割合。停滞と対照を分けて書く。
- 読み方（先に決める）: u − 30 秒か u で、停滞と対照のどちらかで張り付き（0 か 1）の割合が 1/2 以上の成分があれば、限界と考察に「同等の一部は尺度の床と天井による可能性がある」と書く。全成分で 1/4 未満なら「尺度の張り付きでは説明されない」と書く。その間は割合だけを書く。

## X17 停滞の定義と直前の打鍵の量（R2）

- 停滞の行の s と u から、s − u = 30 秒ちょうど（条件 A が u の押下が窓を出たときに初めて真になった。直前の打鍵の量で s が早まっていない）の停滞を S30、s − u < 30 秒の停滞を S<30 とする。
- それぞれの停滞の行（とその行が引いた対照）だけを残して、X15 と同じ方法（`ml_rows` の 18 の量・`paired_auc`・ε なし・主の層）で、u 以前 30 秒の押下の数（level_presses）と 1 分あたりの文字の押下（level_char_presses_per_min）の AUC を出す。2 つの信号の 30 秒の変化（d_p_stuck・d_ewma_x）も同じ方法で添える。区間は出さない（点と件数だけ・記述）。
- 門 G3: 停滞を全部残すと、X15 の値（`experiments/20261106-findings-paper1-exploration/results/explore_learner_features.json` の level_presses・level_char_presses_per_min）と 1e-12 以内で同じ。
- 読み方（先に決める）: S30 の level_presses の AUC が 0.5 から 0.02 以内なら「学習器の『少ない打鍵』は停滞の定義の作り方で説明しうる」と書く。0.46 以下なら「定義だけでは説明されない」と書く。その間は値だけを書く。S30 が 200 件未満なら読まない（件数だけ書く）。
