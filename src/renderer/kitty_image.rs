use itertools::Itertools;

use super::ImageFragment;

pub const IMAGE_PLACEHOLDER: char = '\u{10EEEE}';
include!(concat!(env!("OUT_DIR"), "/kitty_rowcolumn_diacritics.rs"));

pub fn parse_kitty_image_placeholder(
    text: &str,
    start_column: u32,
    color: u32,
    underline_color: u32,
    fragments: &mut Vec<ImageFragment>,
) -> bool {
    if !text.starts_with(IMAGE_PLACEHOLDER) {
        return false;
    }

    if text.len() % 3 != 0 {
        log::warn!("Invalid Kitty placeholder {text}");
    }
    let image_id = color.swap_bytes() >> 8;
    let placement_id = underline_color.swap_bytes() >> 8;

    fragments.extend(
        text.chars()
            .tuples()
            .enumerate()
            .flat_map(|(index, (placeholder, row, column))| {
                if placeholder != IMAGE_PLACEHOLDER {
                    log::warn!("Invalid Kitty placeholder {text}");
                    None
                } else {
                    let col = get_row_or_col(column);
                    let row = get_row_or_col(row);
                    Some((index, col, row))
                }
            })
            // Group consecutive columns together
            .chunk_by(|(index, col, row)| (*col as isize - *index as isize, *row))
            .into_iter()
            .map(|(_, chunk)| {
                let mut chunk_iter = chunk.into_iter();
                let (index, col, row) = chunk_iter.next().unwrap();
                let len = chunk_iter.count() + 1;
                ImageFragment {
                    dst_col: index as u32 + start_column,
                    src_row: row,
                    src_range: col..col + len as u32,
                    image_id,
                    placement_id,
                }
            }),
    );

    true
}
