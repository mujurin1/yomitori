use std::ops::{BitOr, BitOrAssign};
use std::sync::OnceLock;

use super::hotkey_filter::ModifierState;

/// 修飾キーの組み合わせ
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotkeyModifiers(u8);

impl HotkeyModifiers {
    pub const NONE: Self = Self(0);
    pub const WIN: Self = Self(1);
    pub const CTRL: Self = Self(2);
    pub const ALT: Self = Self(4);
    pub const SHIFT: Self = Self(8);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    fn from_state(s: ModifierState) -> Self {
        let mut m = Self::NONE;
        for (held, flag) in [
            (s.win, Self::WIN),
            (s.ctrl, Self::CTRL),
            (s.alt, Self::ALT),
            (s.shift, Self::SHIFT),
        ] {
            if held {
                m |= flag;
            }
        }
        m
    }
}

impl BitOr for HotkeyModifiers {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for HotkeyModifiers {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// 修飾の組み合わせ + 1 つのキー（仮想キーコード）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub modifiers: HotkeyModifiers,
    pub virtual_key: u32,
}

const MODIFIER_NAMES: [(&str, HotkeyModifiers); 4] = [
    ("Win", HotkeyModifiers::WIN),
    ("Ctrl", HotkeyModifiers::CTRL),
    ("Alt", HotkeyModifiers::ALT),
    ("Shift", HotkeyModifiers::SHIFT),
];

fn modifier_by_name(name: &str) -> Option<HotkeyModifiers> {
    MODIFIER_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, m)| m)
}

impl Hotkey {
    pub const fn new(modifiers: HotkeyModifiers, virtual_key: u32) -> Self {
        Self {
            modifiers,
            virtual_key,
        }
    }

    /// 発火後に修飾を離した時、Windows がスタートメニュー（Win）やメニューバー（Alt）を開くので、
    /// マスクキーで抑える必要があるか
    pub fn needs_mask_key(self) -> bool {
        self.modifiers
            .intersects(HotkeyModifiers::WIN | HotkeyModifiers::ALT)
    }

    /// 押されている修飾が指定とちょうど同じか
    pub fn matches(self, state: ModifierState) -> bool {
        HotkeyModifiers::from_state(state) == self.modifiers
    }

    /// 指定の修飾がすべて押されているか（他の修飾が押されていてもよい）
    pub fn modifiers_held(self, state: ModifierState) -> bool {
        HotkeyModifiers::from_state(state).contains(self.modifiers)
    }

    /// 書式 `修飾+修飾+キー` を解釈する。修飾は Win / Ctrl / Alt / Shift（大文字小文字は問わない）。
    /// 修飾なしの単独キーも可。キー名は [`key_names`]。解釈できなければ理由
    pub fn parse(text: &str) -> Result<Hotkey, String> {
        if text.trim().is_empty() {
            return Err("ホットキーが空です".into());
        }

        let parts: Vec<&str> = text.split('+').collect();
        let (key_part, modifier_parts) = parts.split_last().expect("split は 1 つ以上返す");
        let mut modifiers = HotkeyModifiers::NONE;
        for part in modifier_parts {
            let name = part.trim();
            let Some(modifier) = modifier_by_name(name) else {
                return Err(if name.is_empty() {
                    format!("「{text}」に空の部分があります")
                } else {
                    format!("「{name}」は修飾ではありません（Win / Ctrl / Alt / Shift）")
                });
            };
            if modifiers.intersects(modifier) {
                return Err(format!("「{text}」で {name} が重なっています"));
            }
            modifiers |= modifier;
        }

        let key_name = key_part.trim();
        if key_name.is_empty() {
            return Err(format!("「{text}」にキーがありません"));
        }
        if modifier_by_name(key_name).is_some() {
            return Err(format!(
                "「{text}」の最後が修飾です。最後はキーにしてください"
            ));
        }
        let Some(vk) = virtual_key_by_name(key_name) else {
            return Err(format!("「{key_name}」は使えるキー名ではありません"));
        };

        Ok(Hotkey::new(modifiers, vk))
    }
}

/// 使えるキー名 → 仮想キーコード（引くときは大文字小文字を問わない）。README の一覧と揃える
pub fn key_names() -> &'static [(String, u32)] {
    static KEYS: OnceLock<Vec<(String, u32)>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<(String, u32)> = Vec::new();
        keys.extend(
            ('A'..='Z')
                .chain('0'..='9')
                .map(|c| (c.to_string(), u32::from(c))),
        );
        keys.extend((1..=24).map(|i| (format!("F{i}"), 0x70 + i - 1)));
        let named = [
            ("Space", 0x20),
            ("PageUp", 0x21),
            ("PageDown", 0x22),
            ("End", 0x23),
            ("Home", 0x24),
            ("Left", 0x25),
            ("Up", 0x26),
            ("Right", 0x27),
            ("Down", 0x28),
            ("PrintScreen", 0x2C),
            ("Insert", 0x2D),
            ("Delete", 0x2E),
            ("Pause", 0x13),
        ];
        keys.extend(named.iter().map(|&(n, vk)| (n.to_string(), vk)));
        // Esc は範囲指定の中止に使うので割り当てられない
        keys
    })
}

fn virtual_key_by_name(name: &str) -> Option<u32> {
    key_names()
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, vk)| vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Hotkey {
        Hotkey::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn 修飾とキーを解釈する() {
        assert_eq!(
            Hotkey::new(HotkeyModifiers::WIN, 'Q' as u32),
            parse("Win+Q")
        );
    }

    #[test]
    fn 修飾は複数を任意の順で書ける() {
        let all = HotkeyModifiers::WIN
            | HotkeyModifiers::CTRL
            | HotkeyModifiers::ALT
            | HotkeyModifiers::SHIFT;

        assert_eq!(Hotkey::new(all, 'A' as u32), parse("Shift+Alt+Ctrl+Win+A"));
    }

    #[test]
    fn 大文字小文字と前後の空白は問わない() {
        assert_eq!(
            Hotkey::new(HotkeyModifiers::CTRL | HotkeyModifiers::SHIFT, 0x70),
            parse(" ctrl + SHIFT + f1 ")
        );
    }

    #[test]
    fn 修飾なしの単独キーを許す() {
        assert_eq!(Hotkey::new(HotkeyModifiers::NONE, 0x77), parse("F8"));
    }

    #[test]
    fn キー名を仮想キーコードにする() {
        let cases = [
            ("A", 0x41),
            ("Z", 0x5A),
            ("0", 0x30),
            ("9", 0x39),
            ("F1", 0x70),
            ("F12", 0x7B),
            ("F24", 0x87),
            ("Space", 0x20),
            ("PrintScreen", 0x2C),
        ];
        for (name, vk) in cases {
            assert_eq!(vk, parse(name).virtual_key, "{name}");
        }
    }

    #[test]
    fn 解釈できない書式は不正() {
        let cases = [
            "",
            "   ",
            "Win+",
            "+Q",
            "Win++Q",
            "Win",
            "Ctrl+Shift",
            "Win+Win+Q",
            "Hyper+Q",
            "Win+QQ",
            "Win+F25",
            "Win+Esc",
            "Win+Q+W",
            "Win+F01",
            "Win+F+1",
        ];
        for text in cases {
            match Hotkey::parse(text) {
                Ok(h) => panic!("{text:?} が {h:?} と解釈された"),
                Err(e) => assert!(!e.is_empty(), "{text:?}"),
            }
        }
    }

    #[test]
    fn winかaltを含む時だけマスクキーが要る() {
        let cases = [
            ("Win+Q", true),
            ("Alt+Q", true),
            ("Ctrl+Alt+Q", true),
            ("Ctrl+Q", false),
            ("Ctrl+Shift+Q", false),
            ("F8", false),
        ];
        for (text, needs_mask) in cases {
            assert_eq!(needs_mask, parse(text).needs_mask_key(), "{text}");
        }
    }
}
