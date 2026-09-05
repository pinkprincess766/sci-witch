use std::fs;
use std::path::PathBuf;

use sciwhisper_core::Domain;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hotkey::Combo;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Push-to-talk: hold to record. Example: "Ctrl+Shift+Space"
    #[serde(default = "default_ptt")]
    pub ptt: String,
    /// Start/stop recording by pressing either Control key twice.
    #[serde(default = "default_double_control")]
    pub double_control: bool,
    #[serde(default = "default_ptt_latex")]
    pub ptt_latex: String,
    #[serde(default = "default_ptt_word")]
    pub ptt_word: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    /// How the ordinary words around a formula are treated:
    /// `mixed` keeps the whole sentence and replaces only proven spans,
    /// `scientific` drops a recognised dictation shell («ну запиши …»).
    /// The default is the safe one: nothing the user said is deleted.
    #[serde(default = "default_dictation")]
    pub dictation: String,
    /// auto | unicode | latex | word
    #[serde(default = "default_output")]
    pub output: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default)]
    pub persist_history: bool,
    /// Input device name from `sciwhisper_asr::capture::input_devices()`;
    /// `None` uses the system default microphone.
    #[serde(default)]
    pub mic: Option<String>,
}

fn default_ptt() -> String {
    "Ctrl+Shift+Space".into()
}
fn default_double_control() -> bool {
    true
}
fn default_ptt_latex() -> String {
    "Ctrl+Shift+L".into()
}
fn default_ptt_word() -> String {
    "Ctrl+Shift+W".into()
}
fn default_output() -> String {
    "auto".into()
}
fn default_dictation() -> String {
    "mixed".into()
}

fn default_domain() -> String {
    "auto".into()
}
fn default_lang() -> String {
    "ru".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ptt: default_ptt(),
            double_control: default_double_control(),
            ptt_latex: default_ptt_latex(),
            ptt_word: default_ptt_word(),
            domain: default_domain(),
            dictation: default_dictation(),
            output: default_output(),
            model: None,
            language: default_lang(),
            persist_history: false,
            mic: None,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        if let Some(path) = std::env::var_os("SCIWHISPER_CONFIG") {
            return PathBuf::from(path);
        }
        if cfg!(windows) {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("SciWhisper")
                .join("config.yaml")
        } else {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/sciwhisper/config.yaml")
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save_to(path)?;
            return Ok(cfg);
        }
        let raw = fs::read_to_string(path)?;
        let cfg: Config = serde_yaml::from_str(&raw)
            .map_err(|e| Error::Message(format!("config {}: {e}", path.display())))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let raw = serde_yaml::to_string(self)
            .map_err(|e| Error::Message(format!("serialize config: {e}")))?;
        fs::write(path, raw)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        self.domain
            .parse::<Domain>()
            .map_err(|e| Error::Message(format!("domain: {e}")))?;
        OutputMode::try_parse(&self.output)?;
        if self.language.trim().is_empty() {
            return Err(Error::Message("language must not be empty".into()));
        }
        for (name, value) in [
            ("ptt", &self.ptt),
            ("ptt_latex", &self.ptt_latex),
            ("ptt_word", &self.ptt_word),
        ] {
            Combo::parse(value).map_err(|e| Error::Message(format!("{name}: {e}")))?;
        }
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let key = key.trim().to_ascii_lowercase().replace('-', "_");
        let value = value.trim();
        match key.as_str() {
            "domain" => {
                let domain = value
                    .parse::<Domain>()
                    .map_err(|e| Error::Message(format!("domain: {e}")))?;
                self.domain = domain.as_str().into();
            }
            "output" | "format" => {
                self.output = OutputMode::try_parse(value)?.as_str().into();
            }
            "model" => {
                self.model = match value {
                    "" | "-" | "none" | "default" => None,
                    _ => Some(value.into()),
                };
            }
            "mic" | "microphone" | "input_device" => {
                self.mic = match value {
                    "" | "-" | "none" | "default" => None,
                    _ => Some(value.into()),
                };
            }
            "language" | "lang" => {
                if value.is_empty() {
                    return Err(Error::Message("language must not be empty".into()));
                }
                self.language = value.into();
            }
            "ptt" => {
                Combo::parse(value).map_err(Error::Message)?;
                self.ptt = value.into();
            }
            "double_control" | "double_ctrl" => {
                self.double_control = parse_bool(value)?;
            }
            "ptt_latex" | "latex_hotkey" => {
                Combo::parse(value).map_err(Error::Message)?;
                self.ptt_latex = value.into();
            }
            "ptt_word" | "word_hotkey" => {
                Combo::parse(value).map_err(Error::Message)?;
                self.ptt_word = value.into();
            }
            "persist_history" | "history" => {
                self.persist_history = parse_bool(value)?;
            }
            _ => {
                return Err(Error::Message(format!(
                    "unknown setting '{key}'. Expected domain, output, model, language, mic, ptt, double_control, ptt_latex, ptt_word or persist_history"
                )));
            }
        }
        self.validate()
    }

    pub fn domain(&self) -> Domain {
        self.domain.parse().unwrap_or(Domain::Auto)
    }

    /// An unreadable value falls back to the mode that cannot delete text.
    pub fn dictation_mode(&self) -> sciwhisper_core::UtteranceMode {
        self.dictation
            .parse()
            .unwrap_or(sciwhisper_core::UtteranceMode::MixedText)
    }

    pub fn output(&self) -> OutputMode {
        OutputMode::parse(&self.output)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Auto,
    Unicode,
    Latex,
    Word,
}

impl OutputMode {
    pub fn try_parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(OutputMode::Auto),
            "unicode" | "plain" => Ok(OutputMode::Unicode),
            "latex" | "tex" => Ok(OutputMode::Latex),
            "word" | "omml" | "native" => Ok(OutputMode::Word),
            _ => Err(Error::Message(format!(
                "unknown output '{s}'. Expected auto, unicode, latex or word"
            ))),
        }
    }

    pub fn parse(s: &str) -> Self {
        Self::try_parse(s).unwrap_or(OutputMode::Auto)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OutputMode::Auto => "auto",
            OutputMode::Unicode => "unicode",
            OutputMode::Latex => "latex",
            OutputMode::Word => "word",
        }
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        _ => Err(Error::Message(format!(
            "invalid boolean '{value}'. Expected true or false"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_updates_are_validated() {
        let mut config = Config::default();
        config.set("domain", "chemistry").unwrap();
        config.set("format", "latex").unwrap();
        config.set("double_control", "false").unwrap();
        config.set("history", "yes").unwrap();
        assert_eq!(config.domain, "chemistry");
        assert_eq!(config.output, "latex");
        assert!(!config.double_control);
        assert!(config.persist_history);
        assert!(config.set("output", "magic").is_err());
        assert!(config.set("ptt", "DefinitelyNotAKey").is_err());
    }

    #[test]
    fn mic_setting_round_trips_and_clears_to_default() {
        let mut config = Config::default();
        assert_eq!(config.mic, None);
        config.set("mic", "USB Microphone").unwrap();
        assert_eq!(config.mic.as_deref(), Some("USB Microphone"));
        config.set("microphone", "default").unwrap();
        assert_eq!(config.mic, None);
    }

    #[test]
    fn config_round_trip_uses_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut config = Config::default();
        config.set("model", "/models/base.pt").unwrap();
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.model.as_deref(), Some("/models/base.pt"));
    }
}
