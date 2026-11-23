//! Physical keys and key-level actions.
//!
//! Key codes are Linux evdev codes (`linux/input-event-codes.h`): they are a
//! stable, layout-independent numbering of physical keys, and the uinput
//! driver sends them as-is.

use std::fmt;
use std::ops::{BitOr, Sub};
use std::time::Duration;

use crate::progress::TypingProgress;

/// A physical key, identified by its evdev code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyCode(pub u16);

impl fmt::Debug for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyCode({})", self.0)
    }
}

impl KeyCode {
    pub const ESC: Self = Self(1);
    pub const MINUS: Self = Self(12);
    pub const EQUAL: Self = Self(13);
    pub const BACKSPACE: Self = Self(14);
    pub const TAB: Self = Self(15);
    pub const LEFT_BRACE: Self = Self(26);
    pub const RIGHT_BRACE: Self = Self(27);
    pub const ENTER: Self = Self(28);
    pub const LEFT_CTRL: Self = Self(29);
    pub const SEMICOLON: Self = Self(39);
    pub const APOSTROPHE: Self = Self(40);
    pub const GRAVE: Self = Self(41);
    pub const LEFT_SHIFT: Self = Self(42);
    pub const BACKSLASH: Self = Self(43);
    pub const COMMA: Self = Self(51);
    pub const DOT: Self = Self(52);
    pub const SLASH: Self = Self(53);
    pub const KEY_102ND: Self = Self(86);
    pub const RIGHT_SHIFT: Self = Self(54);
    pub const LEFT_ALT: Self = Self(56);
    pub const SPACE: Self = Self(57);
    pub const RIGHT_CTRL: Self = Self(97);
    pub const RIGHT_ALT: Self = Self(100);

    /// Shift, Ctrl and Alt keys (both sides).
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::LEFT_SHIFT
                | Self::RIGHT_SHIFT
                | Self::LEFT_CTRL
                | Self::RIGHT_CTRL
                | Self::LEFT_ALT
                | Self::RIGHT_ALT
        )
    }

    /// The modifier flag this key sets, if it is a modifier.
    pub fn modifier_flag(self) -> Option<Mods> {
        match self {
            Self::LEFT_SHIFT | Self::RIGHT_SHIFT => Some(Mods::SHIFT),
            Self::LEFT_CTRL | Self::RIGHT_CTRL => Some(Mods::CTRL),
            Self::RIGHT_ALT => Some(Mods::ALTGR),
            _ => None,
        }
    }
}

/// A set of modifiers held while pressing a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1);
    /// Level-3 shift (right Alt on most European layouts).
    pub const ALTGR: Self = Self(2);
    pub const CTRL: Self = Self(4);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The single-flag sets contained in `self`, in a fixed order.
    pub fn flags(self) -> impl Iterator<Item = Mods> {
        [Self::CTRL, Self::ALTGR, Self::SHIFT].into_iter().filter(move |&f| self.contains(f))
    }

    /// Only the flags that change which character a key produces.
    pub fn level(self) -> Self {
        Self(self.0 & (Self::SHIFT.0 | Self::ALTGR.0))
    }
}

impl BitOr for Mods {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl Sub for Mods {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 & !rhs.0)
    }
}

/// One key press: the key plus the modifiers that must be down with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stroke {
    pub key: KeyCode,
    pub mods: Mods,
}

impl Stroke {
    pub const fn new(key: KeyCode, mods: Mods) -> Self {
        Self { key, mods }
    }
}

/// One step for the executor at key level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press(KeyCode),
    Release(KeyCode),
    Wait(Duration),
    Progress(TypingProgress),
}

/// The finger that normally presses a key in touch typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Finger {
    LeftPinky,
    LeftRing,
    LeftMiddle,
    LeftIndex,
    Thumb,
    RightIndex,
    RightMiddle,
    RightRing,
    RightPinky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Finger {
    /// `None` for thumbs, which either hand may use.
    pub fn hand(self) -> Option<Hand> {
        use Finger::*;
        match self {
            LeftPinky | LeftRing | LeftMiddle | LeftIndex => Some(Hand::Left),
            RightIndex | RightMiddle | RightRing | RightPinky => Some(Hand::Right),
            Thumb => None,
        }
    }
}
