use std::{fmt::Display, str::FromStr};

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum, strum::EnumIter)]
pub enum FfiColorScheme {
    Light,
    Dark,
}

#[derive(Default, Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum, strum::EnumIter)]
pub enum ColorSchemeSelection {
    Light,
    Dark,
    #[default]
    System,
}

impl ColorSchemeSelection {
    pub fn as_capitalized_string(&self) -> &'static str {
        match self {
            ColorSchemeSelection::Light => "Light",
            ColorSchemeSelection::Dark => "Dark",
            ColorSchemeSelection::System => "System",
        }
    }
}

impl FromStr for ColorSchemeSelection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ColorSchemeSelection::from(s))
    }
}

impl From<&str> for ColorSchemeSelection {
    fn from(value: &str) -> Self {
        match value {
            "Light" | "light" => Self::Light,
            "Dark" | "dark" => Self::Dark,
            "System" | "system" => Self::System,
            other => match other.to_lowercase().as_str() {
                "light" => Self::Light,
                "dark" => Self::Dark,
                _ => Self::System,
            },
        }
    }
}

impl From<String> for ColorSchemeSelection {
    fn from(value: String) -> Self {
        value.as_str().into()
    }
}

impl AsRef<str> for ColorSchemeSelection {
    fn as_ref(&self) -> &str {
        match self {
            ColorSchemeSelection::Light => "light",
            ColorSchemeSelection::Dark => "dark",
            ColorSchemeSelection::System => "system",
        }
    }
}

impl From<ColorSchemeSelection> for String {
    fn from(value: ColorSchemeSelection) -> Self {
        value.as_ref().to_string()
    }
}

impl Display for ColorSchemeSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

use strum::IntoEnumIterator as _;

#[uniffi::export]
fn all_color_schemes() -> Vec<ColorSchemeSelection> {
    ColorSchemeSelection::iter().collect()
}

#[uniffi::export]
fn color_scheme_selection_capitalized_string(color_scheme: ColorSchemeSelection) -> String {
    color_scheme.as_capitalized_string().to_string()
}
