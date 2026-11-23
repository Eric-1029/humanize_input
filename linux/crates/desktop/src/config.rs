//! Persistent settings in `~/.config/humanize_input/config.toml`.

use std::io;
use std::path::{Path, PathBuf};

use humanize_core::{HumanModel, Macros, Preset, TypingMode, WordDelete};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[serde(rename = "zh-CN")]
    #[default]
    Chinese,
    #[serde(rename = "en-US")]
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follow the desktop's light/dark preference.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Portal trigger syntax, e.g. `CTRL+ALT+S`.
    pub start: String,
    pub pause_resume: String,
    pub cancel: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { start: "CTRL+ALT+S".into(), pause_resume: "CTRL+ALT+P".into(), cancel: "CTRL+ALT+Q".into() }
    }
}

/// How the virtual keyboard identifies itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub polling_hz: u32,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            name: "USB Keyboard".into(),
            vendor: 0x1c4f,
            product: 0x0002,
            version: 0x0110,
            polling_hz: 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: Language,
    pub theme: Theme,
    pub preset: Preset,
    pub macros: Macros,
    /// Full parameter set edited in the advanced panel; replaces the preset
    /// as the base the sliders apply to.
    pub advanced: Option<HumanModel>,
    pub word_delete: WordDelete,
    pub lead_in_ms: u32,
    pub hotkeys: Hotkeys,
    pub device: Device,
    pub start_minimized: bool,
    /// Switch fcitx5 to direct input while typing.
    pub switch_input_method: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: Language::default(),
            theme: Theme::default(),
            preset: Preset::Average,
            macros: Macros::default(),
            advanced: None,
            word_delete: WordDelete::Auto,
            lead_in_ms: 2500,
            hotkeys: Hotkeys::default(),
            device: Device::default(),
            start_minimized: true,
            switch_input_method: true,
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/humanize_input/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
        Some(base.join("humanize_input").join("config.toml"))
    }

    /// Loads `path`; a missing file gives the defaults, and missing or
    /// unknown fields are filled in / ignored. A value of the wrong type or
    /// out of range is an error: callers should then avoid saving over the
    /// file, so the user can fix it. (`model` and `[classic]` from 0.x
    /// files, the removed Original model, are among the ignored fields.)
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// The human model the sliders and advanced panel describe.
    pub fn human_model(&self) -> HumanModel {
        let base = self.advanced.unwrap_or_else(|| self.preset.model());
        HumanModel {
            word_delete: self.word_delete,
            lead_in_ms: self.lead_in_ms,
            ..base.with_macros(self.macros)
        }
    }

    pub fn typing_mode(&self) -> TypingMode {
        TypingMode::Human(self.human_model())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("humanize-config-test-{}-{name}", std::process::id()))
            .join("config.toml")
    }

    #[test]
    fn round_trips_through_toml() {
        let path = temp_path("roundtrip");
        let config = Config {
            language: Language::English,
            preset: Preset::Professional,
            macros: Macros { speed: 70, accuracy: 30, stability: 55 },
            advanced: Some(HumanModel { wpm: 77.0, ..HumanModel::default() }),
            word_delete: WordDelete::Backspaces,
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_file_and_partial_file_use_defaults() {
        let path = temp_path("partial");
        assert_eq!(Config::load(&path).unwrap(), Config::default());

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "language = \"en-US\"\n[macros]\nspeed = 80\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.language, Language::English);
        assert_eq!(config.macros, Macros { speed: 80, ..Macros::default() });
        assert_eq!(config.hotkeys, Hotkeys::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn typing_mode_applies_sliders_and_shared_options() {
        let config = Config {
            lead_in_ms: 1000,
            macros: Macros { speed: 100, ..Macros::default() },
            ..Config::default()
        };
        let TypingMode::Human(model) = config.typing_mode() else { panic!() };
        assert_eq!(model.lead_in_ms, 1000);
        assert!((model.wpm - Preset::Average.model().wpm * 2.0).abs() < 1e-9);
        assert_eq!(model.word_delete, WordDelete::Auto);
    }

    #[test]
    fn files_from_0x_with_the_original_model_load_as_human() {
        let path = temp_path("classic");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old =
            "model = \"classic\"\nlead_in_ms = 1500\n[classic]\nbase_delay_ms = 90\njitter_percent = 20\n";
        std::fs::write(&path, old).unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config, Config { lead_in_ms: 1500, ..Config::default() });
        assert!(matches!(config.typing_mode(), TypingMode::Human(_)));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_values_are_reported_not_silently_defaulted() {
        let path = temp_path("invalid");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in ["[macros]\nspeed = 300\n", "preset = \"expert\"\n", "lead_in_ms = \"soon\"\n"] {
            std::fs::write(&path, bad).unwrap();
            assert!(Config::load(&path).is_err(), "{bad:?} should fail");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
