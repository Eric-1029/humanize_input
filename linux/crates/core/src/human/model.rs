//! Parameters of the human typing model, presets and the three macro sliders.

/// How a botched word is erased before retyping it.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordDelete {
    /// Ctrl+Backspace in known non-terminal apps; one Backspace per character
    /// in terminals (where Ctrl+Backspace usually deletes a single character)
    /// and whenever the target app is unknown. Resolved once the target
    /// window is known.
    #[default]
    Auto,
    /// One Ctrl+Backspace. Natural in editors and browsers; terminals often
    /// treat it as a single Backspace.
    CtrlBackspace,
    /// One Backspace per typed character. Works everywhere.
    Backspaces,
}

/// Application ids (Wayland app id / X11 class) of terminal emulators.
const TERMINALS: &[&str] = &[
    "konsole",
    "yakuake",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "footclient",
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "st",
    "st-256color",
    "tilix",
    "terminator",
    "ghostty",
    "rio",
    "kgx",
    "blackbox",
    "contour",
    "qterminal",
    "lxterminal",
    "sakura",
    "termite",
    "cool-retro-term",
    "warp",
];

impl WordDelete {
    /// Settles [`WordDelete::Auto`] for the window with `app_id`.
    pub fn resolve(self, app_id: Option<&str>) -> Self {
        if self != Self::Auto {
            return self;
        }
        // Unknown target: Backspaces work everywhere, Ctrl+Backspace does not.
        let Some(app) = app_id.map(str::to_ascii_lowercase) else { return Self::Backspaces };
        // Reverse-DNS ids: compare the last component too (org.kde.konsole).
        let last = app.rsplit('.').next().unwrap_or(&app).to_owned();
        let terminal = app.contains("terminal")
            || app.contains("console")
            || TERMINALS.iter().any(|t| *t == app || *t == last);
        if terminal { Self::Backspaces } else { Self::CtrlBackspace }
    }
}

/// Every tunable of the human model. Percentages are 0–100; times are ms.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HumanModel {
    // ── Rhythm ──────────────────────────────────────────────────────────
    /// Speed inside a burst, in words (5 characters) per minute.
    pub wpm: f64,
    /// Log-normal shape of key intervals: higher is more irregular.
    pub interval_sigma: f64,
    /// Interval multiplier when the same finger types two different keys.
    pub same_finger_factor: f64,
    /// Interval multiplier for two keys on the same hand, different fingers.
    pub same_hand_factor: f64,
    /// Interval multiplier when hands alternate (or the thumb is involved).
    pub alternate_hand_factor: f64,
    /// Interval multiplier for common English letter pairs (th, er, …).
    pub common_bigram_factor: f64,
    /// Interval multiplier inside very common words (the, and, …).
    pub familiar_word_factor: f64,
    /// Median time a key stays down.
    pub dwell_ms: f64,
    /// Log-normal shape of the dwell time.
    pub dwell_sigma: f64,
    /// How long Shift / Ctrl goes down before the key it modifies.
    pub modifier_lead_ms: f64,

    // ── Pauses ──────────────────────────────────────────────────────────
    /// Extra pause before starting each word.
    pub word_pause_ms: f64,
    /// Extra pause after , ; :
    pub punctuation_pause_ms: f64,
    /// Extra pause after . ! ? and a space.
    pub sentence_pause_ms: f64,
    /// Extra pause after a blank line (a single line break gets 60 %).
    pub paragraph_pause_ms: f64,
    /// Chance per word of stopping to think.
    pub long_pause_percent: f64,
    /// Median length of such a stop.
    pub long_pause_ms: f64,

    // ── Fatigue ─────────────────────────────────────────────────────────
    /// Slowdown per 1000 typed characters, in percent (capped at 30 %).
    pub fatigue_slowdown_percent: f64,
    /// Error-rate increase per 1000 typed characters, in percent (capped at 50 %).
    pub fatigue_error_percent: f64,

    // ── Mistakes ────────────────────────────────────────────────────────
    /// Hitting an adjacent key, per character.
    pub typo_percent: f64,
    /// Skipping a character.
    pub omission_percent: f64,
    /// Swapping a character with the next one.
    pub transpose_percent: f64,
    /// Pressing a key twice.
    pub double_press_percent: f64,
    /// Shift timing slip, per character where case matters.
    pub case_slip_percent: f64,
    /// Fumbling a whole word and retyping it, per word of 4+ letters.
    pub word_redo_percent: f64,

    // ── Corrections ─────────────────────────────────────────────────────
    /// Share of mistakes that get noticed and fixed.
    pub repair_percent: f64,
    /// Median time before a mistake is noticed.
    pub notice_delay_ms: f64,
    /// Chance of typing 1–2 more characters before noticing a mistake.
    pub lookahead_percent: f64,
    /// Median interval between repeated backspaces.
    pub backspace_interval_ms: f64,
    pub word_delete: WordDelete,

    /// Countdown before typing starts, so the user can focus the target.
    pub lead_in_ms: u32,
}

/// Starting points for the model.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    /// 生疏: slow, irregular, many mistakes, careful corrections.
    Novice,
    /// 普通
    Average,
    /// 熟练
    Proficient,
    /// 打字员: fast, steady, few mistakes.
    Professional,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Self::Novice, Self::Average, Self::Proficient, Self::Professional];

    pub fn model(self) -> HumanModel {
        let base = HumanModel {
            wpm: 45.0,
            interval_sigma: 0.35,
            same_finger_factor: 1.35,
            same_hand_factor: 1.08,
            alternate_hand_factor: 0.85,
            common_bigram_factor: 0.82,
            familiar_word_factor: 0.85,
            dwell_ms: 105.0,
            dwell_sigma: 0.25,
            modifier_lead_ms: 60.0,
            word_pause_ms: 40.0,
            punctuation_pause_ms: 300.0,
            sentence_pause_ms: 900.0,
            paragraph_pause_ms: 1800.0,
            long_pause_percent: 1.5,
            long_pause_ms: 3500.0,
            fatigue_slowdown_percent: 4.0,
            fatigue_error_percent: 10.0,
            typo_percent: 2.5,
            omission_percent: 1.0,
            transpose_percent: 0.8,
            double_press_percent: 0.5,
            case_slip_percent: 5.0,
            word_redo_percent: 1.0,
            repair_percent: 88.0,
            notice_delay_ms: 700.0,
            lookahead_percent: 40.0,
            backspace_interval_ms: 140.0,
            word_delete: WordDelete::Auto,
            lead_in_ms: 2500,
        };
        match self {
            Self::Average => base,
            Self::Novice => HumanModel {
                wpm: 28.0,
                interval_sigma: 0.45,
                familiar_word_factor: 0.9,
                dwell_ms: 120.0,
                modifier_lead_ms: 90.0,
                word_pause_ms: 80.0,
                punctuation_pause_ms: 400.0,
                sentence_pause_ms: 1200.0,
                paragraph_pause_ms: 2500.0,
                long_pause_percent: 2.5,
                long_pause_ms: 4000.0,
                fatigue_slowdown_percent: 6.0,
                fatigue_error_percent: 15.0,
                typo_percent: 4.0,
                omission_percent: 1.5,
                transpose_percent: 1.2,
                double_press_percent: 0.8,
                case_slip_percent: 8.0,
                word_redo_percent: 1.5,
                repair_percent: 92.0,
                notice_delay_ms: 900.0,
                lookahead_percent: 45.0,
                backspace_interval_ms: 180.0,
                ..base
            },
            Self::Proficient => HumanModel {
                wpm: 65.0,
                interval_sigma: 0.28,
                familiar_word_factor: 0.8,
                dwell_ms: 95.0,
                modifier_lead_ms: 50.0,
                word_pause_ms: 25.0,
                punctuation_pause_ms: 220.0,
                sentence_pause_ms: 650.0,
                paragraph_pause_ms: 1400.0,
                long_pause_percent: 1.0,
                long_pause_ms: 3000.0,
                fatigue_slowdown_percent: 3.0,
                fatigue_error_percent: 8.0,
                typo_percent: 1.5,
                omission_percent: 0.6,
                transpose_percent: 0.6,
                double_press_percent: 0.3,
                case_slip_percent: 3.0,
                word_redo_percent: 0.6,
                repair_percent: 92.0,
                notice_delay_ms: 500.0,
                lookahead_percent: 35.0,
                backspace_interval_ms: 110.0,
                ..base
            },
            Self::Professional => HumanModel {
                wpm: 90.0,
                interval_sigma: 0.22,
                familiar_word_factor: 0.75,
                dwell_ms: 85.0,
                modifier_lead_ms: 40.0,
                word_pause_ms: 15.0,
                punctuation_pause_ms: 160.0,
                sentence_pause_ms: 450.0,
                paragraph_pause_ms: 1000.0,
                long_pause_percent: 0.6,
                long_pause_ms: 2500.0,
                fatigue_slowdown_percent: 2.0,
                fatigue_error_percent: 5.0,
                typo_percent: 0.8,
                omission_percent: 0.3,
                transpose_percent: 0.4,
                double_press_percent: 0.2,
                case_slip_percent: 1.5,
                word_redo_percent: 0.3,
                repair_percent: 97.0,
                notice_delay_ms: 350.0,
                lookahead_percent: 30.0,
                backspace_interval_ms: 90.0,
                ..base
            },
        }
    }
}

/// The three main-screen sliders, each 0–100 with 50 meaning "as the preset".
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Macros {
    /// Faster typing and shorter pauses. 0 halves the speed, 100 doubles it.
    pub speed: u8,
    /// Fewer mistakes. 0 makes them 4× as likely, 100 a quarter as likely.
    pub accuracy: u8,
    /// Steadier rhythm: less irregularity, fewer long stops, less fatigue.
    pub stability: u8,
}

impl Default for Macros {
    fn default() -> Self {
        Self { speed: 50, accuracy: 50, stability: 50 }
    }
}

impl HumanModel {
    /// Applies the macro sliders on top of this model.
    pub fn with_macros(mut self, m: Macros) -> Self {
        let level = |v: u8, span: f64| 2f64.powf((f64::from(v.min(100)) - 50.0) / span);
        let speed = level(m.speed, 50.0);
        let errors = 1.0 / level(m.accuracy, 25.0);
        let wobble = 1.0 / level(m.stability, 50.0);

        self.wpm *= speed;
        self.dwell_ms /= speed.sqrt();
        for pause in [
            &mut self.word_pause_ms,
            &mut self.punctuation_pause_ms,
            &mut self.sentence_pause_ms,
            &mut self.paragraph_pause_ms,
            &mut self.long_pause_ms,
            &mut self.notice_delay_ms,
            &mut self.backspace_interval_ms,
        ] {
            *pause /= speed;
        }
        for rate in [
            &mut self.typo_percent,
            &mut self.omission_percent,
            &mut self.transpose_percent,
            &mut self.double_press_percent,
            &mut self.case_slip_percent,
            &mut self.word_redo_percent,
        ] {
            *rate *= errors;
        }
        self.interval_sigma *= wobble;
        self.long_pause_percent *= wobble;
        self.fatigue_slowdown_percent *= wobble;
        self.fatigue_error_percent *= wobble;
        self.sanitized()
    }

    /// Clamps every field into a range the planner handles sensibly.
    pub fn sanitized(mut self) -> Self {
        let clamp = |v: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { lo };
        self.wpm = clamp(self.wpm, 5.0, 250.0);
        self.interval_sigma = clamp(self.interval_sigma, 0.0, 1.2);
        for f in [
            &mut self.same_finger_factor,
            &mut self.same_hand_factor,
            &mut self.alternate_hand_factor,
            &mut self.common_bigram_factor,
            &mut self.familiar_word_factor,
        ] {
            *f = clamp(*f, 0.3, 3.0);
        }
        self.dwell_ms = clamp(self.dwell_ms, 30.0, 200.0);
        self.dwell_sigma = clamp(self.dwell_sigma, 0.0, 0.8);
        self.modifier_lead_ms = clamp(self.modifier_lead_ms, 15.0, 250.0);
        for ms in [
            &mut self.word_pause_ms,
            &mut self.punctuation_pause_ms,
            &mut self.sentence_pause_ms,
            &mut self.paragraph_pause_ms,
            &mut self.long_pause_ms,
            &mut self.notice_delay_ms,
        ] {
            *ms = clamp(*ms, 0.0, 60_000.0);
        }
        self.backspace_interval_ms = clamp(self.backspace_interval_ms, 30.0, 1000.0);
        for pct in [
            &mut self.long_pause_percent,
            &mut self.fatigue_slowdown_percent,
            &mut self.fatigue_error_percent,
            &mut self.typo_percent,
            &mut self.omission_percent,
            &mut self.transpose_percent,
            &mut self.double_press_percent,
            &mut self.case_slip_percent,
            &mut self.word_redo_percent,
            &mut self.repair_percent,
            &mut self.lookahead_percent,
        ] {
            *pct = clamp(*pct, 0.0, 100.0);
        }
        self
    }
}

impl Default for HumanModel {
    fn default() -> Self {
        Preset::Average.model()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_macros_keep_the_preset() {
        for preset in Preset::ALL {
            assert_eq!(preset.model().with_macros(Macros::default()), preset.model().sanitized());
        }
    }

    #[test]
    fn macros_move_in_the_right_direction() {
        let base = Preset::Average.model();
        let fast = base.with_macros(Macros { speed: 100, ..Macros::default() });
        let sloppy = base.with_macros(Macros { accuracy: 0, ..Macros::default() });
        let steady = base.with_macros(Macros { stability: 100, ..Macros::default() });
        assert!((fast.wpm - base.wpm * 2.0).abs() < 1e-9);
        assert!(fast.sentence_pause_ms < base.sentence_pause_ms);
        assert!((sloppy.typo_percent - base.typo_percent * 4.0).abs() < 1e-9);
        assert!(steady.interval_sigma < base.interval_sigma);
    }

    #[test]
    fn sanitized_rejects_nonsense() {
        let m =
            HumanModel { wpm: f64::NAN, typo_percent: 400.0, dwell_ms: 10_000.0, ..HumanModel::default() }
                .sanitized();
        assert_eq!(m.wpm, 5.0);
        assert_eq!(m.typo_percent, 100.0);
        assert_eq!(m.dwell_ms, 200.0);
    }

    #[test]
    fn auto_word_delete_avoids_ctrl_backspace_in_terminals() {
        for app in [
            "org.kde.konsole",
            "kitty",
            "Alacritty",
            "org.wezfurlong.wezterm",
            "gnome-terminal-server",
            "org.gnome.Console",
        ] {
            assert_eq!(WordDelete::Auto.resolve(Some(app)), WordDelete::Backspaces, "{app}");
        }
        for app in ["org.kde.kate", "firefox", "code", "org.kde.dolphin"] {
            assert_eq!(WordDelete::Auto.resolve(Some(app)), WordDelete::CtrlBackspace, "{app}");
        }
        assert_eq!(WordDelete::Auto.resolve(None), WordDelete::Backspaces);
        assert_eq!(WordDelete::Backspaces.resolve(Some("firefox")), WordDelete::Backspaces);
    }
}
