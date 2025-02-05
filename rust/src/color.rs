#[derive(Debug, Copy, Clone, PartialEq, uniffi::Enum)]
pub enum FfiColor {
    // default colors
    Red(FfiOpacity),
    Blue(FfiOpacity),
    Green(FfiOpacity),
    Yellow(FfiOpacity),
    Orange(FfiOpacity),
    Purple(FfiOpacity),
    Pink(FfiOpacity),
    White(FfiOpacity),
    Black(FfiOpacity),
    Gray(FfiOpacity),

    // other custom colors
    CoolGray(FfiOpacity),

    // any custom
    Custom(Rgb, FfiOpacity),
}

uniffi::custom_newtype!(FfiOpacity, u8);

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FfiOpacity(pub u8);

impl From<u8> for FfiOpacity {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl Default for FfiOpacity {
    fn default() -> Self {
        FfiOpacity(100)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, uniffi::Record)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
