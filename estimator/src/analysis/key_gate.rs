//! ★★★ **特徴量へ流すキーの集合**（ADR-016 **D8** のゲート分割の「内側」）。
//!
//! ## ⚠️★★★ ゲートが 2 本になった理由
//!
//! ★ O1 は「打鍵が薄い作業（図面・動画編集・3D）」を「離席」と取り違えないことを保証する。
//! ⚠️ そのためには **修飾キー / F1–F12 / Esc / 矢印**も *記録* しなければならないが、
//! ★ 従来はゲートが 1 本しか無く、**記録に足す＝特徴量にも流れる**構造だった
//! （フック → 分析はチャネルが 1 本で、`FeatureExtractor::process_event` が
//! 受け取った全イベントを特徴量に使う）。
//!
//! → ⚠️★★★ 素朴に広げると **F1〜F6 と `keys` が変わり `HMM_VERSION` bump ＝
//! ゴールデン全崩れ・G1 の既収集ブロックとも比較不能**になる。
//!
//! ```text
//! [hook]   is_observed_key(vk)   ★ 広い（修飾 / F1–F12 / Esc / 矢印）
//!             ↓ channel
//! [分析]   is_feature_key(vk)    ⚠️ 旧 is_typing_key と**同一集合**
//!             ├→ process_event() … ★ 特徴量は 1 バイトも変わらない
//!             └→ logger ………… ★ 広い集合を category 量子化して key 行に記録
//! ```
//!
//! ## ⚠️★★★ なぜ `analysis/` に置くのか（★ `input/` ではない）
//!
//! ★ **`gse-replay` が本番と同じ `loop_step::step()` を通る**からである（2026-08-23 実測）。
//! → ★ ここに 1 か所置けば、**本番とリプレイの両方に同時に効く**。
//! ⚠️ `input/` 側（OS 固有）に置くと replay から呼べず、
//! **新形式ログを再生したとき修飾キーが特徴量に混入する**（＝ Replay-as-Truth の破れ・
//! ADR-016 D8 の「実装時に必ず併せてやること 1」）。
//!
//! ⚠️ OS API を 1 つも呼ばないので `architecture-rules` ルール 2 に抵触しない。

/// このキーを **特徴量に流してよい**か。
///
/// ⚠️★★★ **この集合を 1 ビットでも変えたら `HMM_VERSION` を bump すること。**
/// ★ 集合は 2026-08-23 時点の `hook_windows.rs::is_typing_key` と**完全に同一**である
/// （[`tests::feature_set_is_the_frozen_typing_key_set`] が範囲の端で固定する）。
///
/// | 範囲 | 何 |
/// |---|---|
/// | `0x41..=0x5A` | 英字 A–Z |
/// | `0x30..=0x39` | 数字 0–9 |
/// | `0x60..=0x6F` | テンキー（⚠️ **Windows のみ到達する** —— [`is_observed_key`] の OS 非対称・下記） |
/// | `0xBA..=0xC0` / `0xDB..=0xDF` | OEM 記号・句読点 |
/// | `0x08` / `0x0D` / `0x20` / `0x09` / `0x2E` | BS / Enter / Space / Tab / Delete |
///
/// [`is_observed_key`]: crate::input::hook
pub fn is_feature_key(vk_code: u32) -> bool {
    matches!(
        vk_code,
        0x41..=0x5A       // Letters A-Z
        | 0x30..=0x39     // Digits 0-9
        | 0x60..=0x6F     // Numpad keys
        | 0xBA..=0xC0     // Symbol / punctuation keys (OEM)
        | 0xDB..=0xDF
        | 0x08            // Backspace
        | 0x0D            // Enter
        | 0x20            // Space
        | 0x09            // Tab
        | 0x2E            // Delete
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️★★★ **凍結集合**（旧 `is_typing_key` と 1 ビットも違わないこと）。
    ///
    /// ★ 範囲の**両端とその外側**を押さえる。⚠️ ここが動いたら `HMM_VERSION` の話になる。
    #[test]
    fn feature_set_is_the_frozen_typing_key_set() {
        // 通すもの (範囲の端)
        for (vk, why) in [
            (0x41, "A"),
            (0x5A, "Z"),
            (0x30, "0"),
            (0x39, "9"),
            (0x60, "Numpad 0 (下端)"),
            (0x6F, "Numpad Divide (上端)"),
            (0xBA, "OEM 記号 1 群目 (下端)"),
            (0xC0, "OEM 記号 1 群目 (上端)"),
            (0xDB, "OEM 記号 2 群目 (下端)"),
            (0xDF, "OEM 記号 2 群目 (上端)"),
            (0x08, "Backspace"),
            (0x0D, "Enter"),
            (0x20, "Space"),
            (0x09, "Tab"),
            (0x2E, "Delete"),
        ] {
            assert!(is_feature_key(vk), "{why} は特徴量キーである");
        }
        // 落とすもの (範囲の外側)
        for (vk, why) in [
            (0x40, "A-Z の直前"),
            (0x5B, "A-Z の直後 (VK_LWIN)"),
            (0x2F, "0-9 の直前"),
            (0x3A, "0-9 の直後"),
            (0x5F, "Numpad の直前"),
            (0x70, "Numpad の直後 (VK_F1)"),
            (0xB9, "OEM 1 群目の直前"),
            (0xC1, "OEM 1 群目の直後"),
            (0xDA, "OEM 2 群目の直前"),
            (0xE0, "OEM 2 群目の直後"),
        ] {
            assert!(!is_feature_key(vk), "{why} は特徴量キーではない");
        }
    }

    /// ⚠️★★★ **観測だけして特徴量に流さないキー**（＝ ゲート分割の存在理由）。
    ///
    /// ★ ここが 1 つでも `true` になったら、**広げた瞬間に特徴量が変わる**。
    #[test]
    fn observed_only_keys_never_reach_the_feature_path() {
        for (vk, why) in [
            (0x10, "VK_SHIFT"),
            (0x11, "VK_CONTROL"),
            (0x12, "VK_MENU"),
            (0x5B, "VK_LWIN"),
            (0x5C, "VK_RWIN"),
            (0xA0, "VK_LSHIFT (LL フックが返す形)"),
            (0xA1, "VK_RSHIFT"),
            (0xA2, "VK_LCONTROL"),
            (0xA3, "VK_RCONTROL"),
            (0xA4, "VK_LMENU"),
            (0xA5, "VK_RMENU"),
            (0x21, "VK_PRIOR (Navigation 下端)"),
            (0x25, "VK_LEFT"),
            (0x28, "VK_DOWN (Navigation 上端)"),
            (0x1B, "VK_ESCAPE"),
            (0x70, "VK_F1"),
            (0x7B, "VK_F12"),
            (0x07, "replay の `other` 代表 VK"),
        ] {
            assert!(!is_feature_key(vk), "{why} が特徴量へ流れてしまう");
        }
    }

    /// ⚠️★★★ **macOS のテンキーを観測集合に入れられない理由**を、**両 OS で走る**形に残す。
    ///
    /// ## ★ なぜ `hook_macos.rs` のテストだけでは足りないのか
    ///
    /// ⚠️ `hook_macos.rs` は `#[cfg(target_os = "macos")]` でしかコンパイルされないので、
    /// ★ **Windows の CI では 1 行も走らない**。→ ⚠️ Windows で作業している人が
    /// 「テンキーも足しておこう」と `macos_vk_to_vk` に写像を入れても、**手元では何も落ちない**。
    /// → ★ 判断の**理由**（写像先が特徴量集合の中に入る）は OS 非依存なので、ここに置く。
    ///
    /// ⚠️ 対になる macOS 側の実測テストは `hook_macos.rs::numpad_is_not_observed_on_macos`。
    #[test]
    fn macos_numpad_cannot_be_observed_because_both_mappings_land_inside_the_feature_set() {
        // ① 素通し (`macos_vk_to_vk` の既定 `other => other as u32`) の場合。
        //    ⚠️ mac の keycode がそのまま Windows VK として解釈される。
        for (mac_keycode, collides_with) in [
            (0x52u32, "VK_R (英字)"),
            (0x53, "VK_S (英字)"),
            (0x41, "VK_A (英字)"),
            (0x4B, "VK_K (英字)"),
        ] {
            assert!(
                is_feature_key(mac_keycode),
                "mac のテンキー {mac_keycode:#04X} は素通しで {collides_with} と衝突し特徴量へ入る"
            );
        }
        // ② 明示写像で Windows のテンキー VK (0x60..=0x6F) へ送った場合。
        //    ⚠️ そちらも `is_feature_key` の**中**である。
        for vk in [0x60u32, 0x6F] {
            assert!(
                is_feature_key(vk),
                "Windows テンキー VK {vk:#04X} は特徴量集合の中にある"
            );
        }
        // → ★★★ **どちらの道も特徴量を変える。だから macOS では観測集合に入れない。**
    }

    /// ⚠️★★★ **replay の代表 VK 逆写像が特徴量を汚さない**こと（ADR-016 D8 の「併せてやること 1」）。
    ///
    /// ★ `bin/replay.rs` の `category_representative_vk` は
    /// **category → 代表 VK** へ逆写像する。⚠️ 新形式ログで初めて現れる 5 カテゴリの代表 VK が
    /// `is_feature_key` に入っていたら、**リプレイだけ特徴量が変わる**（Replay-as-Truth の破れ）。
    #[test]
    fn replay_representative_vks_of_new_categories_are_not_feature_keys() {
        // (category, 代表 VK) — `bin/replay.rs::category_representative_vk` と対で維持する。
        for (category, vk) in [
            ("navigation", 0x25u32),
            ("modifier", 0x10),
            ("function", 0x70),
            ("escape", 0x1B),
            ("other", 0x07),
        ] {
            assert!(
                !is_feature_key(vk),
                "category {category:?} の代表 VK {vk:#04X} が特徴量へ流れる"
            );
        }
        // ★ 逆に、従来から特徴量へ流れていたカテゴリの代表 VK は通ること (非回帰)。
        for (category, vk) in [
            ("alphabetic", 0x41u32),
            ("numeric", 0x30),
            ("punctuation", 0xBA),
            ("sentence_punctuation", 0xBE),
            ("tab", 0x09),
            ("backspace", 0x08),
            ("delete", 0x2E),
            ("space", 0x20),
            ("enter", 0x0D),
        ] {
            assert!(
                is_feature_key(vk),
                "category {category:?} の代表 VK {vk:#04X} が特徴量から外れた (非回帰違反)"
            );
        }
    }
}
