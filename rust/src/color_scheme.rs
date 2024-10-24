use std::fmt::Display;

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum, strum::EnumIter)]
pub enum FfiColorScheme {
    Light,
    Dark,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, uniffi::Enum, strum::EnumIter)]
pub enum ColorSchemeSelection {
    Light,
    Dark,
    System,
}

impl Default for ColorSchemeSelection {
    fn default() -> Self {
        Self::System
    }
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

impl From<&str> for ColorSchemeSelection {
    fn from(value: &str) -> Self {
        match value {
            "Light" | "light" => Self::Light,
            "Dark" | "dark" => Self::Dark,
            "System" | "system" => Self::System,
            _ => Self::System,
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

mod ffi {
    use super::ColorSchemeSelection;
    use strum::IntoEnumIterator as _;

    #[uniffi::export]
    pub fn all_color_schemes() -> Vec<ColorSchemeSelection> {
        ColorSchemeSelection::iter().collect()
    }

    #[uniffi::export]
    pub fn color_scheme_selection_capitalized_string(color_scheme: ColorSchemeSelection) -> String {
        color_scheme.as_capitalized_string().to_string()
    }
}
