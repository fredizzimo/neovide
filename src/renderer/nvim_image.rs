use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Image {
    /// unique id associated with the image
    pub id: u32,
    /// bytes of the image loaded into memory
    #[serde(with = "serde_bytes")]
    pub data: Option<Vec<u8>>,
    /// path to the image on disk
    pub file: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Relative {
    Win,
    Cursor,
    Mouse,
    Placement,
}

/// class vim.ui.img.Opts
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Opts {
    pub relative: Option<Relative>,
    /// topmost row position (in character cells) of image location
    pub row: Option<u32>,
    /// leftmost column position (in character cells) of image location
    pub col: Option<u32>,
    /// width (in character cells) to resize the image
    pub width: Option<u32>,
    /// height (in character cells) to resize the image
    pub height: Option<u32>,
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

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ImgAdd {
    pub id: u32,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
    // pub width: u32,
    // pub height: u32,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ImgShow {
    pub id: u32,
    pub img_id: u32,
    pub width: u32,
    pub height: u32,
    pub keep_aspect: bool,
}
