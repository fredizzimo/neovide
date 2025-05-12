use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Image {
    /// unique id associated with the image
    pub id: u32,
    /// bytes of the image loaded into memory
    #[serde(with = "serde_bytes")]
    pub bytes: Option<Vec<u8>>,
    /// path to the image on disk
    pub filename: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Relative {
    Win,
    Cursor,
    Mouse,
    Placement,
}

/// vim.ui.img.Unit
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Cell,
    Pixel,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Region {
    pos1: Position,
    pos2: Position,
}

/// vim.ui.img.utils.Position
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub unit: Unit,
}

/// vim.ui.img.utils.Size
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
    pub unit: Unit,
}

/// class vim.ui.img.Opts
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Opts {
    pub relative: Option<Relative>,
    /// portion of image to display
    pub crop: Option<Region>,
    ///  upper-left position of image within editor
    pub pos: Option<Position>,
    /// explicit size to scale the image
    pub size: Option<Size>,
    /// window to use when `relative` is `win`
    pub win: Option<i32>,
    /// z-index of the image with lower values being drawn before higher values
    pub z: Option<i32>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct UploadImage {
    pub img: Image,
    pub more_chunks: bool,
    pub base64: bool,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ShowImage {
    pub opts: Opts,
    pub image_id: u32,
    pub placement_id: u32,
}
