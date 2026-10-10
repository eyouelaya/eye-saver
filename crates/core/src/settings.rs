//! User settings: defaults, limits, and TOML persistence.
//!
//! Every field carries `#[serde(default)]` so that a config file written by an older or
//! newer build still loads: missing keys fall back to the default and unknown keys are
//! ignored. Values are clamped on load rather than rejected, so a hand-edited file can
//! never leave the application in a state where the timer makes no sense.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// What the menu bar / tray item shows while the timer runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayDisplay {
    /// A live `19:59` countdown, updated once a second.
    #[default]
    MmSs,
    /// A coarse `19m` countdown that only wakes once a minute, switching to seconds for
    /// the final minute.
    Minutes,
    /// Icon only; the remaining time is visible in the tray menu. Fewest wake-ups.
    IconOnly,
}

impl TrayDisplay {
    /// All variants, in the order the settings window lists them.
    pub const ALL: [Self; 3] = [Self::MmSs, Self::Minutes, Self::IconOnly];

    /// Label for this choice in the settings window.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MmSs => "Countdown (19:59)",
            Self::Minutes => "Minutes (19m)",
            Self::IconOnly => "Icon only",
        }
    }
}

/// Smallest and largest accepted work interval.
pub const WORK_SECS: (u64, u64) = (60, 180 * 60);
/// Smallest and largest accepted break length.
pub const BREAK_SECS: (u64, u64) = (5, 300);
/// Smallest and largest accepted idle-credit threshold.
pub const IDLE_SECS: (u64, u64) = (15, 1800);

const DEFAULT_WORK_SECS: u64 = 20 * 60;
const DEFAULT_BREAK_SECS: u64 = 20;
const DEFAULT_IDLE_SECS: u64 = 60;

/// Everything the user can configure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Settings {
    /// How long to work before being reminded to look away.
    pub work_interval_secs: u64,
    /// How long the look-away break lasts.
    pub break_duration_secs: u64,
    /// Being idle (no keyboard or mouse input) for at least this long counts as a break
    /// already taken, and restarts the work interval.
    pub idle_credit_secs: u64,
    /// What the tray item shows.
    pub tray_display: TrayDisplay,
    /// Whether the app launches when the user logs in.
    pub start_at_login: bool,
    /// Whether the reminder notification plays a sound.
    pub notification_sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            work_interval_secs: DEFAULT_WORK_SECS,
            break_duration_secs: DEFAULT_BREAK_SECS,
            idle_credit_secs: DEFAULT_IDLE_SECS,
            tray_display: TrayDisplay::default(),
            start_at_login: false,
            notification_sound: true,
        }
    }
}

impl Settings {
    /// The work interval as a [`Duration`].
    #[must_use]
    pub const fn work_interval(&self) -> Duration {
        Duration::from_secs(self.work_interval_secs)
    }

    /// The break length as a [`Duration`].
    #[must_use]
    pub const fn break_duration(&self) -> Duration {
        Duration::from_secs(self.break_duration_secs)
    }

    /// The idle-credit threshold as a [`Duration`].
    #[must_use]
    pub const fn idle_credit(&self) -> Duration {
        Duration::from_secs(self.idle_credit_secs)
    }

    /// How often idle time needs sampling.
    ///
    /// Sampling has to be frequent enough to notice an absence promptly but is pointless
    /// at a finer grain than that, so it is derived from the threshold instead of being a
    /// fixed poll rate. With the default 60 s threshold this is 15 s.
    #[must_use]
    pub const fn idle_sample_interval(&self) -> Duration {
        let quarter = self.idle_credit_secs / 4;
        Duration::from_secs(if quarter < 5 { 5 } else { quarter })
    }

    /// Returns a copy with every value forced inside its accepted range.
    #[must_use]
    pub fn clamped(mut self) -> Self {
        self.work_interval_secs = self.work_interval_secs.clamp(WORK_SECS.0, WORK_SECS.1);
        self.break_duration_secs = self.break_duration_secs.clamp(BREAK_SECS.0, BREAK_SECS.1);
        self.idle_credit_secs = self.idle_credit_secs.clamp(IDLE_SECS.0, IDLE_SECS.1);
        self
    }

    /// Whether every value already sits inside its accepted range.
    #[must_use]
    pub fn is_clamped(&self) -> bool {
        *self == self.clamped()
    }

    /// Parses a config file, clamping the result.
    ///
    /// Unknown keys are ignored and missing keys take their default, so config files
    /// survive upgrades and downgrades.
    ///
    /// # Errors
    ///
    /// If the text is not valid TOML, or a key holds a value of the wrong type.
    pub fn from_toml(text: &str) -> Result<Self, ParseError> {
        toml::from_str::<Self>(text)
            .map(Self::clamped)
            .map_err(|e| ParseError(e.to_string()))
    }

    /// Parses a config file, falling back to defaults if it is unreadable.
    ///
    /// Used on startup: a corrupt config should not stop the app from running, it should
    /// just be reported and replaced.
    #[must_use]
    pub fn from_toml_lossy(text: &str) -> (Self, Option<ParseError>) {
        match Self::from_toml(text) {
            Ok(settings) => (settings, None),
            Err(e) => (Self::default(), Some(e)),
        }
    }

    /// Renders the settings as a commented TOML document.
    ///
    /// # Panics
    ///
    /// Never in practice: the struct is a flat set of integers, booleans and a unit enum,
    /// all of which are always serialisable.
    #[must_use]
    pub fn to_toml(self) -> String {
        let body = toml::to_string_pretty(&self).expect("Settings is always serialisable");
        format!(
            "# EyeSaver settings. Edit while the app is closed, or use the settings window.\n\
             # work-interval-secs: {}-{}  break-duration-secs: {}-{}  idle-credit-secs: {}-{}\n\
             # tray-display: \"mm-ss\" | \"minutes\" | \"icon-only\"\n\
             {body}",
            WORK_SECS.0, WORK_SECS.1, BREAK_SECS.0, BREAK_SECS.1, IDLE_SECS.0, IDLE_SECS.1,
        )
    }
}

/// A config file that could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid settings file: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_20_20_20_rule() {
        let s = Settings::default();
        assert_eq!(s.work_interval(), Duration::from_secs(1200));
        assert_eq!(s.break_duration(), Duration::from_secs(20));
        assert_eq!(s.idle_credit(), Duration::from_secs(60));
        assert_eq!(s.tray_display, TrayDisplay::MmSs);
        assert!(!s.start_at_login);
        assert!(s.notification_sound);
        assert!(s.is_clamped());
    }

    #[test]
    fn idle_sample_interval_tracks_threshold_with_a_floor() {
        let mut s = Settings::default();
        assert_eq!(s.idle_sample_interval(), Duration::from_secs(15));
        s.idle_credit_secs = 1800;
        assert_eq!(s.idle_sample_interval(), Duration::from_secs(450));
        s.idle_credit_secs = 15;
        assert_eq!(
            s.idle_sample_interval(),
            Duration::from_secs(5),
            "floor of 5s applies"
        );
        s.idle_credit_secs = 0;
        assert_eq!(s.idle_sample_interval(), Duration::from_secs(5));
    }

    #[test]
    fn clamping_pins_every_field_to_its_range() {
        let low = Settings {
            work_interval_secs: 0,
            break_duration_secs: 0,
            idle_credit_secs: 0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(
            (
                low.work_interval_secs,
                low.break_duration_secs,
                low.idle_credit_secs
            ),
            (WORK_SECS.0, BREAK_SECS.0, IDLE_SECS.0)
        );

        let high = Settings {
            work_interval_secs: u64::MAX,
            break_duration_secs: u64::MAX,
            idle_credit_secs: u64::MAX,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(
            (
                high.work_interval_secs,
                high.break_duration_secs,
                high.idle_credit_secs
            ),
            (WORK_SECS.1, BREAK_SECS.1, IDLE_SECS.1)
        );

        assert!(
            !Settings {
                work_interval_secs: 1,
                ..Settings::default()
            }
            .is_clamped()
        );
    }

    #[test]
    fn clamping_leaves_in_range_values_and_flags_alone() {
        let s = Settings {
            work_interval_secs: 900,
            break_duration_secs: 30,
            idle_credit_secs: 120,
            tray_display: TrayDisplay::IconOnly,
            start_at_login: true,
            notification_sound: false,
        };
        assert_eq!(s.clamped(), s);
    }

    #[test]
    fn toml_round_trips() {
        let s = Settings {
            work_interval_secs: 1500,
            break_duration_secs: 25,
            idle_credit_secs: 90,
            tray_display: TrayDisplay::Minutes,
            start_at_login: true,
            notification_sound: false,
        };
        assert_eq!(Settings::from_toml(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn every_tray_display_variant_round_trips() {
        for display in TrayDisplay::ALL {
            let s = Settings {
                tray_display: display,
                ..Settings::default()
            };
            assert_eq!(
                Settings::from_toml(&s.to_toml()).unwrap().tray_display,
                display
            );
            assert!(!display.label().is_empty());
        }
    }

    #[test]
    fn missing_keys_take_defaults() {
        let s = Settings::from_toml("work-interval-secs = 600").unwrap();
        assert_eq!(s.work_interval_secs, 600);
        assert_eq!(
            s.break_duration_secs,
            Settings::default().break_duration_secs
        );
        assert_eq!(Settings::from_toml("").unwrap(), Settings::default());
    }

    #[test]
    fn unknown_keys_are_ignored_for_forward_compatibility() {
        let s = Settings::from_toml("work-interval-secs = 600\nfuture-feature = \"on\"\n").unwrap();
        assert_eq!(s.work_interval_secs, 600);
    }

    #[test]
    fn out_of_range_file_values_are_clamped_not_rejected() {
        let s = Settings::from_toml("work-interval-secs = 5\nbreak-duration-secs = 9000").unwrap();
        assert_eq!(s.work_interval_secs, WORK_SECS.0);
        assert_eq!(s.break_duration_secs, BREAK_SECS.1);
    }

    #[test]
    fn malformed_toml_reports_an_error() {
        let err = Settings::from_toml("work-interval-secs = = 3").unwrap_err();
        assert!(err.to_string().starts_with("invalid settings file:"));
        assert!(Settings::from_toml("tray-display = \"holographic\"").is_err());
        assert!(Settings::from_toml("work-interval-secs = \"twenty\"").is_err());
    }

    #[test]
    fn lossy_parse_falls_back_to_defaults_and_reports_why() {
        let (settings, err) = Settings::from_toml_lossy("not: valid: toml");
        assert_eq!(settings, Settings::default());
        assert!(err.is_some());

        let (settings, err) = Settings::from_toml_lossy("break-duration-secs = 30");
        assert_eq!(settings.break_duration_secs, 30);
        assert_eq!(err, None);
    }

    #[test]
    fn rendered_toml_documents_the_limits() {
        let text = Settings::default().to_toml();
        assert!(text.contains("# EyeSaver settings"));
        assert!(text.contains("60-10800"));
        assert!(text.contains("tray-display"));
    }

    #[test]
    fn parse_error_is_a_std_error() {
        let err = Settings::from_toml("@").unwrap_err();
        let _: &dyn std::error::Error = &err;
        assert!(format!("{err:?}").contains("ParseError"));
    }
}
