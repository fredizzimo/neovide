use serde::{Deserialize};

#[derive(Deserialize)]
pub struct Image {
    /// unique id associated with the image
    id: i64,
    /// bytes of the image loaded into memory
    bytes: Option<Vec<u8>>,
    /// path to the image on disk
    filename: String,
}

#[derive(Deserialize)]
pub enum Relative {
    win,
    cursor,
    mouse,
}

/// vim.ui.img.Unit 
#[derive(Deserialize)]
pub enum Unit
{
    cell,
    pixel,
}

#[derive(Deserialize)]
pub struct Region {
    pos1: Position,
    pos2: Position,
}


/// vim.ui.img.utils.Position
#[derive(Deserialize)]
pub struct Position {
    x: i32,
    y: i32,
    unit: Unit, 
}


/// vim.ui.img.utils.Size
#[derive(Deserialize)]
pub struct Size {
    width: i32,
    height: i32,
    unit: Unit,
}

/// class vim.ui.img.Opts
#[derive(Deserialize)]
pub struct Opts {
    relative: Option<Relative>,
    /// portion of image to display
    crop: Option<Region>,
    ///  upper-left position of image within editor
    pos: Option<Position>,
    /// explicit size to scale the image
    size: Option<Size>,
    /// window to use when `relative` is `win`
    win: Option<i32>,
    /// z-index of the image with lower values being drawn before higher values
    z: Option<i32> 
}
