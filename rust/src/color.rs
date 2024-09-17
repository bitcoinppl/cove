#[derive(Debug, Copy, Clone, PartialEq, uniffi::Enum)]
pub enum FfiColor {
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
    CoolGray(FfiOpacity),
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

// #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, uniffi::Enum)]
// pub enum DefaultColor {
//     Red,
//     Blue,
//     Green,
//     Yellow,
//     Orange,
//     Purple,
//     Pink,
//     White,
//     Black,
//     Gray,
//     CoolGray,
// }
//
// impl DefaultColor {
//     pub fn to_color(&self) -> FfiColor {
//         match self {
//             DefaultColor::Red => FfiColor::Default(DefaultColor::Red),
//             DefaultColor::Blue => FfiColor::Default(DefaultColor::Blue),
//             DefaultColor::Green => FfiColor::Default(DefaultColor::Green),
//             DefaultColor::Yellow => FfiColor::Default(DefaultColor::Yellow),
//             DefaultColor::Orange => FfiColor::Default(DefaultColor::Orange),
//             DefaultColor::Purple => FfiColor::Default(DefaultColor::Purple),
//             DefaultColor::Pink => FfiColor::Default(DefaultColor::Pink),
//             DefaultColor::White => FfiColor::Default(DefaultColor::White),
//             DefaultColor::Black => FfiColor::Default(DefaultColor::Black),
//             DefaultColor::Gray => FfiColor::Default(DefaultColor::Gray),
//             DefaultColor::CoolGray => FfiColor::Default(DefaultColor::CoolGray),
//         }
//     }
//
//     pub fn with_opacity(&self, opacity: f32) -> FfiColor {
//         FfiColor::WithOpacity(self.clone(), opacity)
//     }
// }
//
// impl From<DefaultColor> for FfiColor {
//     fn from(color: DefaultColor) -> Self {
//         color.to_color()
//     }
// }
