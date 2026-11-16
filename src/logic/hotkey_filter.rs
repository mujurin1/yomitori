use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use super::hotkey::Hotkey;

pub const VK_ESCAPE: u32 = 0x1B;

/// 低レベルキーボードフックに来た 1 件のキー
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub virtual_key: u32,
    pub is_down: bool,
}

/// そのキーが来た時点の修飾キーの押下状態（呼び出し側が GetAsyncKeyState で取る）。
/// 自前で押下を追うと、ロック画面（Win+L）等でフックに来なかった離しで状態が狂うため
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifierState {
    pub win: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    /// 範囲指定キャプチャ（既定 Win+Q）
    RegionCapture,
    /// カーソル行キャプチャ（既定 Win+W）
    CursorLine,
    /// 範囲指定中の Esc: 中止
    CancelCapture,
}

/// ホットキーと、それで起動する処理
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub hotkey: Hotkey,
    pub action: HotkeyAction,
}

/// フックがすること
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookDecision {
    /// このキーを握りつぶす（Windows 標準の動作を出さない）
    pub suppress: bool,
    pub fire: Option<HotkeyAction>,
    /// 割り当ての無いキーを 1 つ送る。Win / Alt を押してから離すまでに他のキーが無いと
    /// スタートメニュー / メニューバーが開くので、発火した時点（修飾がまだ押されている間）に送って抑える
    pub send_mask_key: bool,
}

impl HookDecision {
    pub const PASS_THROUGH: Self = Self {
        suppress: false,
        fire: None,
        send_mask_key: false,
    };
    pub const SUPPRESS_ONLY: Self = Self {
        suppress: true,
        fire: None,
        send_mask_key: false,
    };
}

/// ホットキーの判定。修飾は指定どおりちょうどのものが押されている時だけ反応する（指定外の修飾があれば素通し）。
/// Win は左右どちらでもよい。発火したキーは離すまで（自動リピートを含め）握りつぶし、二重に発火しない
#[derive(Debug)]
pub struct HotkeyFilter {
    bindings: Vec<HotkeyBinding>,
    /// 発火して、まだ離されていないキー → 発火したホットキー（Esc は None）
    held: Mutex<HashMap<u32, Option<Hotkey>>>,
    capture_active: AtomicBool,
}

impl HotkeyFilter {
    pub fn new(bindings: Vec<HotkeyBinding>) -> Self {
        Self {
            bindings,
            held: Mutex::default(),
            capture_active: AtomicBool::new(false),
        }
    }

    /// 範囲指定のオーバーレイを出している間 true。Esc を握りつぶして中止を発火する。
    /// オーバーレイが前面になれない時でも Esc で閉じられるようにするため
    pub fn capture_active(&self) -> bool {
        self.capture_active.load(Ordering::Relaxed)
    }

    pub fn set_capture_active(&self, active: bool) {
        self.capture_active.store(active, Ordering::Relaxed);
    }

    pub fn process(&self, key: KeyEvent, modifiers: ModifierState) -> HookDecision {
        // フックの中で panic させないため、毒された Mutex も中身をそのまま使う
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if key.virtual_key == VK_ESCAPE {
            return self.process_escape(&mut held, key);
        }

        if !key.is_down {
            return if held.remove(&key.virtual_key).is_some() {
                HookDecision::SUPPRESS_ONLY
            } else {
                HookDecision::PASS_THROUGH
            };
        }

        if let Some(&fired) = held.get(&key.virtual_key) {
            // 押し続けの自動リピート。発火した時の修飾が離されているなら、離しを取りこぼした古い状態とみなして捨てる
            if fired.is_some_and(|h| h.modifiers_held(modifiers)) {
                return HookDecision::SUPPRESS_ONLY;
            }
            held.remove(&key.virtual_key);
        }

        // 範囲指定中に Esc 以外のキーを押された（Win+Shift+S・Alt+Tab・仮想デスクトップの切り替え等）。
        // 割り込まれた後に受け皿が中途半端に残らないよう中止し、キーは下のアプリへ渡す
        if self.capture_active() && !is_modifier(key.virtual_key) {
            return HookDecision {
                suppress: false,
                fire: Some(HotkeyAction::CancelCapture),
                send_mask_key: false,
            };
        }

        for binding in &self.bindings {
            if binding.hotkey.virtual_key != key.virtual_key || !binding.hotkey.matches(modifiers) {
                continue;
            }
            held.insert(key.virtual_key, Some(binding.hotkey));
            return HookDecision {
                suppress: true,
                fire: Some(binding.action),
                send_mask_key: binding.hotkey.needs_mask_key(),
            };
        }
        HookDecision::PASS_THROUGH
    }

    fn process_escape(
        &self,
        held: &mut HashMap<u32, Option<Hotkey>>,
        key: KeyEvent,
    ) -> HookDecision {
        if !key.is_down {
            return if held.remove(&VK_ESCAPE).is_some() {
                HookDecision::SUPPRESS_ONLY
            } else {
                HookDecision::PASS_THROUGH
            };
        }
        // 押し続けの自動リピート。中止した後でも離すまでは下のアプリへ流さない
        if held.contains_key(&VK_ESCAPE) {
            return HookDecision::SUPPRESS_ONLY;
        }
        if !self.capture_active() {
            return HookDecision::PASS_THROUGH;
        }
        held.insert(VK_ESCAPE, None);
        HookDecision {
            suppress: true,
            fire: Some(HotkeyAction::CancelCapture),
            send_mask_key: false,
        }
    }
}

/// 修飾キー（単独の押下では範囲指定を中止しない）。左右の区別のあるコードも含む
pub fn is_modifier(vk: u32) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 // Shift・Ctrl・Alt
            | 0x5B | 0x5C // 左右の Win
            | 0xA0..=0xA5 // 左右の Shift・Ctrl・Alt
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: u32 = 0x51;
    const W: u32 = 0x57;
    const A: u32 = 0x41;
    const F8: u32 = 0x77;
    const ESC: u32 = VK_ESCAPE;

    const fn mods(win: bool, ctrl: bool, shift: bool, alt: bool) -> ModifierState {
        ModifierState {
            win,
            ctrl,
            shift,
            alt,
        }
    }

    const NONE: ModifierState = mods(false, false, false, false);
    const WIN_ONLY: ModifierState = mods(true, false, false, false);
    const CTRL_ONLY: ModifierState = mods(false, true, false, false);
    const CTRL_SHIFT: ModifierState = mods(false, true, true, false);
    const ALT_ONLY: ModifierState = mods(false, false, false, true);
    const SUPPRESSED: HookDecision = HookDecision::SUPPRESS_ONLY;
    const PASS: HookDecision = HookDecision::PASS_THROUGH;

    fn fired(action: HotkeyAction, send_mask_key: bool) -> HookDecision {
        HookDecision {
            suppress: true,
            fire: Some(action),
            send_mask_key,
        }
    }

    fn filter(bindings: &[(&str, HotkeyAction)]) -> HotkeyFilter {
        HotkeyFilter::new(
            bindings
                .iter()
                .map(|&(text, action)| HotkeyBinding {
                    hotkey: Hotkey::parse(text).unwrap(),
                    action,
                })
                .collect(),
        )
    }

    fn default_filter() -> HotkeyFilter {
        filter(&[
            ("Win+Q", HotkeyAction::RegionCapture),
            ("Win+W", HotkeyAction::CursorLine),
        ])
    }

    fn down(f: &HotkeyFilter, vk: u32, m: ModifierState) -> HookDecision {
        f.process(
            KeyEvent {
                virtual_key: vk,
                is_down: true,
            },
            m,
        )
    }

    fn up(f: &HotkeyFilter, vk: u32, m: ModifierState) -> HookDecision {
        f.process(
            KeyEvent {
                virtual_key: vk,
                is_down: false,
            },
            m,
        )
    }

    #[test]
    fn win_qで範囲指定を起動しキーを握りつぶしてマスクキーを送る() {
        assert_eq!(
            fired(HotkeyAction::RegionCapture, true),
            down(&default_filter(), Q, WIN_ONLY)
        );
    }

    #[test]
    fn win_wでカーソル行を起動する() {
        assert_eq!(
            fired(HotkeyAction::CursorLine, true),
            down(&default_filter(), W, WIN_ONLY)
        );
    }

    #[test]
    fn 発火したキーの離しも握りつぶす() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);

        assert_eq!(SUPPRESSED, up(&f, Q, WIN_ONLY));
    }

    #[test]
    fn 修飾を先に離してから発火したキーを離しても握りつぶす() {
        let f = default_filter();
        down(&f, W, WIN_ONLY);

        assert!(up(&f, W, NONE).suppress);
    }

    #[test]
    fn 押し続けの自動リピートは握りつぶし二度目は発火しない() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);

        assert_eq!(SUPPRESSED, down(&f, Q, WIN_ONLY));
    }

    #[test]
    fn 離した後にもう一度押せばまた発火する() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);
        up(&f, Q, WIN_ONLY);

        assert_eq!(
            Some(HotkeyAction::RegionCapture),
            down(&f, Q, WIN_ONLY).fire
        );
    }

    #[test]
    fn 指定外の修飾が押されていたら反応しない() {
        for (ctrl, shift, alt) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let f = default_filter();
            let m = mods(true, ctrl, shift, alt);

            assert_eq!(PASS, down(&f, Q, m), "{m:?}");
            assert_eq!(PASS, up(&f, Q, m), "{m:?}");
        }
    }

    #[test]
    fn 指定した修飾が欠けていたら反応しない() {
        let f = filter(&[("Ctrl+Shift+Q", HotkeyAction::RegionCapture)]);

        assert_eq!(PASS, down(&f, Q, CTRL_ONLY));
        assert_eq!(PASS, down(&f, Q, NONE));
    }

    #[test]
    fn 修飾なしのq_wは素通しする() {
        let f = default_filter();

        assert_eq!(PASS, down(&f, Q, NONE));
        assert_eq!(PASS, up(&f, Q, NONE));
        assert_eq!(PASS, down(&f, W, NONE));
    }

    #[test]
    fn 対象外のキーは修飾中でも素通しする() {
        let f = default_filter();

        assert_eq!(PASS, down(&f, A, WIN_ONLY));
        assert_eq!(PASS, up(&f, A, WIN_ONLY));
    }

    #[test]
    fn ctrlとshiftだけの組み合わせはマスクキーを送らない() {
        let f = filter(&[("Ctrl+Shift+Q", HotkeyAction::RegionCapture)]);

        assert_eq!(
            fired(HotkeyAction::RegionCapture, false),
            down(&f, Q, CTRL_SHIFT)
        );
    }

    #[test]
    fn altを含む組み合わせはマスクキーを送る() {
        let f = filter(&[("Alt+Q", HotkeyAction::RegionCapture)]);

        assert_eq!(
            fired(HotkeyAction::RegionCapture, true),
            down(&f, Q, ALT_ONLY)
        );
    }

    #[test]
    fn 修飾なしの単独キーは修飾が無い時だけ発火しマスクキーを送らない() {
        let f = filter(&[("F8", HotkeyAction::CursorLine)]);

        assert_eq!(fired(HotkeyAction::CursorLine, false), down(&f, F8, NONE));
        assert_eq!(SUPPRESSED, up(&f, F8, NONE));
        assert_eq!(PASS, down(&f, F8, CTRL_ONLY));
    }

    #[test]
    fn 同じキーに修飾違いで2つ割り当てるとそれぞれ発火する() {
        let f = filter(&[
            ("Ctrl+Q", HotkeyAction::RegionCapture),
            ("Alt+Q", HotkeyAction::CursorLine),
        ]);

        assert_eq!(
            Some(HotkeyAction::RegionCapture),
            down(&f, Q, CTRL_ONLY).fire
        );
        up(&f, Q, CTRL_ONLY);
        assert_eq!(Some(HotkeyAction::CursorLine), down(&f, Q, ALT_ONLY).fire);
    }

    #[test]
    fn 離しを取りこぼしても修飾なしの次の押下は素通しする() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);
        // ロック画面などで Q の離しがフックに来なかった

        assert_eq!(PASS, down(&f, Q, NONE));
        assert_eq!(PASS, up(&f, Q, NONE));
    }

    #[test]
    fn 離しを取りこぼした後でも押し直すと次は正しく発火する() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);
        down(&f, Q, NONE);

        assert_eq!(
            Some(HotkeyAction::RegionCapture),
            down(&f, Q, WIN_ONLY).fire
        );
    }

    #[test]
    fn qとwは別々に扱う() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);

        assert_eq!(Some(HotkeyAction::CursorLine), down(&f, W, WIN_ONLY).fire);
        assert!(up(&f, Q, WIN_ONLY).suppress);
        assert!(up(&f, W, WIN_ONLY).suppress);
    }

    fn cancel_and_pass() -> HookDecision {
        HookDecision {
            suppress: false,
            fire: Some(HotkeyAction::CancelCapture),
            send_mask_key: false,
        }
    }

    #[test]
    fn 範囲指定中にesc以外のキーを押すと中止しキーは下へ渡す() {
        const S: u32 = 0x53;
        const TAB: u32 = 0x09;
        const RIGHT: u32 = 0x27;
        let cases = [
            (A, NONE),
            (S, mods(true, false, true, false)), // Win+Shift+S
            (TAB, ALT_ONLY),                     // Alt+Tab
            (RIGHT, mods(true, true, false, false)), // Win+Ctrl+→
        ];
        for (vk, m) in cases {
            let f = default_filter();
            f.set_capture_active(true);

            assert_eq!(cancel_and_pass(), down(&f, vk, m), "{vk:#x} {m:?}");
            assert_eq!(PASS, up(&f, vk, m), "{vk:#x} {m:?}");
        }
    }

    #[test]
    fn 範囲指定中でも修飾キーの単独の押下では中止しない() {
        for vk in [
            0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
        ] {
            let f = default_filter();
            f.set_capture_active(true);

            assert_eq!(PASS, down(&f, vk, NONE), "{vk:#x}");
            assert_eq!(PASS, up(&f, vk, NONE), "{vk:#x}");
        }
    }

    #[test]
    fn 範囲指定を始めたホットキーの押し続けは中止せず握りつぶす() {
        let f = default_filter();
        down(&f, Q, WIN_ONLY);
        f.set_capture_active(true);

        assert_eq!(SUPPRESSED, down(&f, Q, WIN_ONLY));
        assert_eq!(SUPPRESSED, up(&f, Q, WIN_ONLY));
    }

    #[test]
    fn 範囲指定中でなければ他のキーは今までどおり() {
        let f = default_filter();

        assert_eq!(PASS, down(&f, A, NONE));
        assert_eq!(
            fired(HotkeyAction::RegionCapture, true),
            down(&f, Q, WIN_ONLY)
        );
    }

    #[test]
    fn 範囲指定中でなければescは素通しする() {
        let f = default_filter();

        assert_eq!(PASS, down(&f, ESC, NONE));
        assert_eq!(PASS, up(&f, ESC, NONE));
    }

    #[test]
    fn 範囲指定中のescは握りつぶして中止を発火する() {
        let f = default_filter();
        f.set_capture_active(true);

        assert_eq!(
            fired(HotkeyAction::CancelCapture, false),
            down(&f, ESC, NONE)
        );
        assert_eq!(SUPPRESSED, up(&f, ESC, NONE));
    }

    #[test]
    fn 中止したescは範囲指定が終わった後もリピートと離しを握りつぶす() {
        let f = default_filter();
        f.set_capture_active(true);
        down(&f, ESC, NONE);
        f.set_capture_active(false);

        assert_eq!(SUPPRESSED, down(&f, ESC, NONE));
        assert!(up(&f, ESC, NONE).suppress);
        assert_eq!(PASS, down(&f, ESC, NONE));
    }
}
