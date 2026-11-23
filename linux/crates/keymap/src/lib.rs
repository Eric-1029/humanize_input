//! Builds a [`Layout`] from the desktop's XKB keymap.
//!
//! The compositor applies one keymap to every keyboard, including our
//! virtual one, so to type a character we look up which physical key and
//! modifiers produce it *in that keymap*. Positions and fingers come from the
//! physical keyboard ([`standard_keys`]), characters from XKB.

use std::fmt;
use std::path::{Path, PathBuf};

use humanize_core::layout::{ModifierKeys, standard_keys};
use humanize_core::{KeyCode, Layout, Mods, Stroke};
use xkbcommon::xkb;

/// XKB keycodes are evdev codes plus 8.
const EVDEV_OFFSET: u32 = 8;

/// RMLVO names that select a keymap. Empty strings mean "system default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XkbNames {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
}

impl XkbNames {
    pub fn new(layout: &str, variant: &str) -> Self {
        Self { layout: layout.into(), variant: variant.into(), ..Self::default() }
    }

    /// The layout KDE uses, from `~/.config/kxkbrc`. `index` selects among
    /// several configured layouts (the active one is reported over DBus).
    /// Falls back to the system default when KDE does not manage layouts.
    pub fn from_kde(index: usize) -> Self {
        kxkbrc_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| parse_kxkbrc(&text, index))
            .unwrap_or_default()
    }

    /// A short label such as `us` or `de(nodeadkeys)`.
    pub fn label(&self) -> String {
        let layout = if self.layout.is_empty() { "default" } else { &self.layout };
        if self.variant.is_empty() { layout.to_owned() } else { format!("{layout}({})", self.variant) }
    }
}

fn kxkbrc_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(config.join("kxkbrc"))
}

/// Parses the `[Layout]` group of kxkbrc. Returns `None` when KDE does not
/// manage layouts (`Use=false`) or lists none.
fn parse_kxkbrc(text: &str, index: usize) -> Option<XkbNames> {
    let mut in_layout = false;
    let (mut use_kde, mut layouts, mut variants, mut model, mut options) = (true, "", "", "", "");
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_layout = line == "[Layout]";
            continue;
        }
        if !in_layout {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Use" => use_kde = value.trim() != "false",
            "LayoutList" => layouts = value,
            "VariantList" => variants = value,
            "Model" => model = value,
            "Options" => options = value,
            _ => {}
        }
    }
    let layout = layouts.split(',').nth(index).map(str::trim).filter(|l| !l.is_empty())?;
    if !use_kde {
        return None;
    }
    let variant = variants.split(',').nth(index).map(str::trim).unwrap_or("");
    Some(XkbNames {
        rules: String::new(),
        model: model.trim().into(),
        layout: layout.into(),
        variant: variant.into(),
        options: Some(options.trim()).filter(|o| !o.is_empty()).map(Into::into),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapError {
    /// libxkbcommon could not compile the names (unknown layout/variant).
    Compile(String),
    /// A key the typist relies on does something else in this keymap.
    MissingKey(&'static str),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(label) => write!(f, "cannot compile XKB keymap {label}"),
            Self::MissingKey(what) => write!(f, "the keymap has no usable {what} key"),
        }
    }
}

impl std::error::Error for KeymapError {}

/// Compiles `names` and maps every typeable character to a stroke.
pub fn load(names: &XkbNames) -> Result<Layout, KeymapError> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        &names.rules,
        &names.model,
        &names.layout,
        &names.variant,
        names.options.clone(),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .ok_or_else(|| KeymapError::Compile(names.label()))?;

    let base = xkb::State::new(&keymap);
    let sym = |key: KeyCode| base.key_get_one_sym(xkb::Keycode::new(u32::from(key.0) + EVDEV_OFFSET)).raw();
    let is = |key: KeyCode, wanted: &[u32]| wanted.contains(&sym(key));

    use xkeysym::key as ks;
    let shifts = [ks::Shift_L, ks::Shift_R];
    let ctrls = [ks::Control_L, ks::Control_R];
    for (key, wanted, what) in [
        (KeyCode::BACKSPACE, ks::BackSpace, "Backspace"),
        (KeyCode::ENTER, ks::Return, "Enter"),
        (KeyCode::TAB, ks::Tab, "Tab"),
        (KeyCode::SPACE, ks::space, "Space"),
    ] {
        if sym(key) != wanted {
            return Err(KeymapError::MissingKey(what));
        }
    }
    let pick = |candidates: &[KeyCode], wanted: &[u32]| candidates.iter().copied().find(|&k| is(k, wanted));
    let modifiers = ModifierKeys {
        left_shift: pick(&[KeyCode::LEFT_SHIFT, KeyCode::RIGHT_SHIFT], &shifts)
            .ok_or(KeymapError::MissingKey("Shift"))?,
        right_shift: pick(&[KeyCode::RIGHT_SHIFT, KeyCode::LEFT_SHIFT], &shifts)
            .ok_or(KeymapError::MissingKey("Shift"))?,
        ctrl: pick(&[KeyCode::LEFT_CTRL, KeyCode::RIGHT_CTRL], &ctrls),
        altgr: pick(&[KeyCode::RIGHT_ALT], &[ks::ISO_Level3_Shift]),
    };

    let shift_mask = mod_mask(&keymap, xkb::MOD_NAME_SHIFT);
    let level3_mask = mod_mask(&keymap, xkb::MOD_NAME_ISO_LEVEL3_SHIFT).filter(|_| modifiers.altgr.is_some());
    let mut levels = vec![(Mods::NONE, 0)];
    if let Some(shift) = shift_mask {
        levels.push((Mods::SHIFT, shift));
    }
    if let Some(level3) = level3_mask {
        levels.push((Mods::ALTGR, level3));
        if let Some(shift) = shift_mask {
            levels.push((Mods::SHIFT | Mods::ALTGR, shift | level3));
        }
    }

    let keys = standard_keys();
    let mut chars = vec![
        (' ', Stroke::new(KeyCode::SPACE, Mods::NONE)),
        ('\n', Stroke::new(KeyCode::ENTER, Mods::NONE)),
        ('\t', Stroke::new(KeyCode::TAB, Mods::NONE)),
    ];
    // Level order first, so a character reachable without modifiers is
    // always typed that way. The ISO key left of Z comes last: ANSI
    // keyboards lack it, so use it only for characters found nowhere else.
    let typeable = |key: KeyCode| {
        !key.is_modifier()
            && !matches!(key, KeyCode::SPACE | KeyCode::ENTER | KeyCode::TAB | KeyCode::BACKSPACE)
    };
    let (main, extra): (Vec<KeyCode>, Vec<KeyCode>) =
        keys.iter().map(|&(k, _)| k).filter(|&k| typeable(k)).partition(|&k| k != KeyCode::KEY_102ND);
    for group in [main, extra] {
        for &(mods, mask) in &levels {
            let mut state = xkb::State::new(&keymap);
            state.update_mask(mask, 0, 0, 0, 0, 0);
            for &key in &group {
                let code = xkb::Keycode::new(u32::from(key.0) + EVDEV_OFFSET);
                // Dead keys and unassigned levels yield 0 and are skipped.
                if let Some(c) = char::from_u32(state.key_get_utf32(code)).filter(|c| !c.is_control()) {
                    chars.push((c, Stroke::new(key, mods)));
                }
            }
        }
    }

    // Keep plain "us" recognisable so classic mode uses the original table.
    let name = if names.layout == "us" && names.variant.is_empty() { "us".to_owned() } else { names.label() };
    Ok(Layout::new(name, chars, keys).with_modifier_keys(modifiers))
}

/// The layout KDE currently uses (first configured layout), or the system
/// default.
pub fn system_layout() -> Result<(Layout, XkbNames), KeymapError> {
    let names = XkbNames::from_kde(0);
    load(&names).map(|layout| (layout, names))
}

fn mod_mask(keymap: &xkb::Keymap, name: &str) -> Option<u32> {
    let index = keymap.mod_get_index(name);
    (index != xkb::MOD_INVALID && index < 32).then(|| 1 << index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_on(layout: &Layout, code: u16, mods: Mods) -> Option<char> {
        layout.char_for(KeyCode(code), mods)
    }

    #[test]
    fn xkb_us_matches_built_in_us() {
        let xkb = load(&XkbNames::new("us", "")).unwrap();
        let builtin = Layout::us_qwerty();
        for c in (' '..='~').chain(['\n', '\t']) {
            assert_eq!(xkb.stroke(c), builtin.stroke(c), "{c:?}");
        }
        assert_eq!(xkb.name(), "us");
        // Plain US has no AltGr: right Alt is just Alt.
        assert_eq!(xkb.modifier_keys(), ModifierKeys { altgr: None, ..ModifierKeys::default() });
    }

    #[test]
    fn german_swaps_y_z_and_uses_altgr() {
        let de = load(&XkbNames::new("de", "")).unwrap();
        assert_eq!(char_on(&de, 21, Mods::NONE), Some('z'));
        assert_eq!(char_on(&de, 44, Mods::NONE), Some('y'));
        // '@' is AltGr+Q on German keyboards.
        assert_eq!(de.stroke('@'), Some(Stroke::new(KeyCode(16), Mods::ALTGR)));
        // '<' lives on the ISO key next to left Shift.
        assert_eq!(de.stroke('<'), Some(Stroke::new(KeyCode::KEY_102ND, Mods::NONE)));
        // Dead keys (^ and `) are not typeable in one stroke.
        assert!(de.stroke('^').is_none());
        assert!(de.find_unsupported("Grüße").len() == 2, "ü and ß are outside the ASCII release");
    }

    #[test]
    fn french_azerty_and_dvorak_move_letters() {
        let fr = load(&XkbNames::new("fr", "")).unwrap();
        assert_eq!(char_on(&fr, 16, Mods::NONE), Some('a'));
        assert_eq!(fr.stroke('1').map(|s| s.mods), Some(Mods::SHIFT));

        let dvorak = load(&XkbNames::new("us", "dvorak")).unwrap();
        assert_eq!(char_on(&dvorak, 32, Mods::NONE), Some('e'));
        assert_eq!(dvorak.name(), "us(dvorak)");
        // Fingers follow the physical key, not the letter.
        let e = dvorak.stroke('e').unwrap().key;
        assert_eq!(e, KeyCode(32));
    }

    #[test]
    fn swapped_ctrl_is_detected() {
        let names = XkbNames { options: Some("ctrl:swap_lalt_lctl".into()), ..XkbNames::new("us", "") };
        let layout = load(&names).unwrap();
        assert_eq!(layout.modifier_keys().ctrl, Some(KeyCode::RIGHT_CTRL));
    }

    #[test]
    fn unknown_layout_fails() {
        assert!(matches!(load(&XkbNames::new("no-such-layout", "")), Err(KeymapError::Compile(_))));
    }

    #[test]
    fn parses_kxkbrc() {
        let text = "[Layout]\nLayoutList=us,de\nVariantList=,nodeadkeys\nModel=pc105\nOptions=caps:escape\nUse=true\n";
        assert_eq!(parse_kxkbrc(text, 0).unwrap().label(), "us");
        let de = parse_kxkbrc(text, 1).unwrap();
        assert_eq!(de.label(), "de(nodeadkeys)");
        assert_eq!(de.model, "pc105");
        assert_eq!(de.options.as_deref(), Some("caps:escape"));
        assert!(parse_kxkbrc(text, 2).is_none());
        assert!(parse_kxkbrc("[Layout]\nLayoutList=us\nUse=false\n", 0).is_none());
        assert!(parse_kxkbrc("[Other]\nLayoutList=us\n", 0).is_none());
    }
}
