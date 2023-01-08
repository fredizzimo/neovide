use std::sync::Arc;

use crate::editor::style::Style;

pub type GridCell = (String, Option<Arc<Style>>);

#[macro_export]
macro_rules! default_cell {
    () => {
        (" ".to_owned(), None)
    };
}

#[derive(Clone)]
struct GridLine {
    characters: Vec<GridCell>,
}

impl GridLine {
    pub fn new(length: usize) -> GridLine {
        GridLine {
            characters: vec![default_cell!(); length],
        }
    }
}

pub struct CharacterGrid {
    pub width: usize,
    pub height: usize,

    lines: Vec<GridLine>,
}

impl CharacterGrid {
    pub fn new((width, height): (usize, usize)) -> CharacterGrid {
        CharacterGrid {
            width,
            height,
            lines: vec![GridLine::new(width); height],
        }
    }

    pub fn resize(&mut self, (width, height): (usize, usize)) {
        let mut new_lines = vec![GridLine::new(width); height];

        for x in 0..self.width.min(width) {
            for y in 0..self.height.min(height) {
                if let Some(existing_cell) = self.get_cell(x, y) {
                    new_lines[y].characters[x] = existing_cell.clone();
                }
            }
        }

        self.width = width;
        self.height = height;
        self.lines = new_lines;
    }

    pub fn clear(&mut self) {
        self.set_all_characters(default_cell!());
    }

    pub fn get_cell(&self, x: usize, y: usize) -> Option<&GridCell> {
        self.lines
            .get(y)
            .map(|line| line.characters.get(x))
            .flatten()
    }

    pub fn get_cell_mut(&mut self, x: usize, y: usize) -> Option<&mut GridCell> {
        self.lines
            .get_mut(y)
            .map(|line| line.characters.get_mut(x))
            .flatten()
    }

    pub fn set_all_characters(&mut self, value: GridCell) {
        for line in &mut self.lines {
            for grid in &mut line.characters {
                *grid = value.clone()
            }
        }
    }

    pub fn row(&self, row_index: usize) -> Option<&[GridCell]> {
        if row_index < self.height {
            Some(&self.lines[row_index].characters[..])
        } else {
            None
        }
    }

    pub fn scroll_region(
        &mut self,
        top: usize,
        bottom: usize,
        left: usize,
        right: usize,
        rows: isize,
        cols: isize,
    ) {
        let mut top_to_bottom;
        let mut bottom_to_top;
        let y_iter: &mut dyn Iterator<Item = usize> = if rows > 0 {
            top_to_bottom = (top as isize + rows) as usize..bottom;
            &mut top_to_bottom
        } else {
            bottom_to_top = (top..(bottom as isize + rows) as usize).rev();
            &mut bottom_to_top
        };

        for y in y_iter {
            let dest_y = y as isize - rows;
            let mut cols_left;
            let mut cols_right;
            if dest_y >= 0 && dest_y < self.height as isize {
                let x_iter: &mut dyn Iterator<Item = usize> = if cols > 0 {
                    cols_left = (left as isize + cols) as usize..right;
                    &mut cols_left
                } else {
                    cols_right = (left..(right as isize + cols) as usize).rev();
                    &mut cols_right
                };

                for x in x_iter {
                    let dest_x = ((x as isize) - cols) as usize;
                    let cell_data = self.get_cell(x, y).cloned();

                    if let Some(cell_data) = cell_data {
                        if let Some(dest_cell) = self.get_cell_mut(dest_x, dest_y as usize) {
                            *dest_cell = cell_data;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::style::Colors;
    use rand::*;

    #[derive(Debug)]
    struct Context {
        none_colors: Colors,
        size: (usize, usize),
        x: usize,
        y: usize,
    }

    impl Context {
        fn new() -> Self {
            let size = (
                (thread_rng().gen::<usize>() % 500) + 1,
                (thread_rng().gen::<usize>() % 500) + 1,
            );
            let (x, y) = (
                thread_rng().gen::<usize>() % size.0,
                thread_rng().gen::<usize>() % size.1,
            );
            Self {
                none_colors: Colors {
                    foreground: None,
                    background: None,
                    special: None,
                },
                size,
                x,
                y,
            }
        }
    }

    fn assert_all_cells_equal_to(context: &Context, grid: &CharacterGrid, cell: &GridCell) {
        for x in 0..context.size.0 {
            for y in 0..context.size.1 {
                assert_eq!(grid.get_cell(x, y), Some(cell));
            }
        }
    }

    fn assert_grid_cell_equal_to_char(grid: &CharacterGrid, x: usize, y: usize, chr: &str) {
        let chr = chr.to_string();
        let value = (chr, None);
        let cell = Some(&value);
        assert_eq!(grid.get_cell(x, y), cell);
    }

    fn create_initialized_grid(lines: &Vec<&str>) -> CharacterGrid {
        let num_lines = lines.len();
        assert_ne!(num_lines, 0);
        let line_lengths: Vec<usize> = lines.iter().map(|s| s.len()).collect();
        let num_columns = line_lengths[0];
        assert_eq!(line_lengths, vec![num_columns; num_lines]);
        let mut grid = CharacterGrid::new((num_columns, num_lines));
        for (row_nr, line) in lines.iter().enumerate() {
            for (col_nr, chr) in line.chars().enumerate() {
                *grid.get_cell_mut(col_nr, row_nr).unwrap() = (chr.to_string(), None);
            }
        }
        grid
    }

    #[test]
    fn new_constructs_grid() {
        let context = Context::new();

        // RUN FUNCTION
        let character_grid = CharacterGrid::new(context.size);
        assert_eq!(character_grid.width, context.size.0);
        assert_eq!(character_grid.height, context.size.1);
        assert_all_cells_equal_to(&context, &character_grid, &default_cell!());
    }

    #[test]
    fn get_cell_returns_expected_cell() {
        let context = Context::new();
        let mut character_grid = CharacterGrid::new(context.size);

        character_grid.lines[context.y].characters[context.x] = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );
        let result = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );

        // RUN FUNCTION
        assert_eq!(
            character_grid.get_cell(context.x, context.y).unwrap(),
            &result
        );
    }

    #[test]
    fn get_cell_mut_modifiers_grid_properly() {
        let context = Context::new();
        let mut character_grid = CharacterGrid::new(context.size);

        character_grid.lines[context.y].characters[context.x] = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );
        let result = (
            "bar".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );

        // RUN FUNCTION
        let cell = character_grid.get_cell_mut(context.x, context.y).unwrap();
        *cell = (
            "bar".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );

        assert_eq!(
            character_grid.get_cell_mut(context.x, context.y).unwrap(),
            &result
        );
    }

    #[test]
    fn set_all_characters_sets_all_cells_to_given_character() {
        let context = Context::new();
        let grid_cell = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );
        let mut character_grid = CharacterGrid::new(context.size);

        // RUN FUNCTION
        character_grid.set_all_characters(grid_cell.clone());
        assert_all_cells_equal_to(&context, &character_grid, &grid_cell);
    }

    #[test]
    fn clear_empties_buffer() {
        let context = Context::new();
        let mut character_grid = CharacterGrid::new(context.size);

        let grid_cell = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors.clone()))),
        );
        character_grid.set_all_characters(grid_cell.clone());

        // RUN FUNCTION
        character_grid.clear();

        assert_eq!(character_grid.width, context.size.0);
        assert_eq!(character_grid.height, context.size.1);
        assert_all_cells_equal_to(&context, &character_grid, &default_cell!());
    }

    #[test]
    fn resize_clears_and_resizes_grid() {
        let context = Context::new();
        let mut character_grid = CharacterGrid::new(context.size);
        let (width, height) = (
            (thread_rng().gen::<usize>() % 500) + 1,
            (thread_rng().gen::<usize>() % 500) + 1,
        );

        let grid_cell = (
            "foo".to_string(),
            Some(Arc::new(Style::new(context.none_colors))),
        );
        character_grid.set_all_characters(grid_cell.clone());

        // RUN FUNCTION
        character_grid.resize((width, height));

        assert_eq!(character_grid.width, width);
        assert_eq!(character_grid.height, height);

        let (original_width, original_height) = context.size;
        for x in 0..original_width.min(width) {
            for y in 0..original_height.min(height) {
                assert_eq!(character_grid.get_cell(x, y).unwrap(), &grid_cell);
            }
        }

        for x in original_width..width {
            for y in original_height..height {
                assert_eq!(character_grid.get_cell(x, y).unwrap(), &default_cell!());
            }
        }
    }

    #[test]
    fn scroll_down_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(0, 4, 0, 4, 2, 0);
        assert_grid_cell_equal_to_char(&grid, 0, 0, "i");
        assert_grid_cell_equal_to_char(&grid, 3, 0, "l");
        assert_grid_cell_equal_to_char(&grid, 0, 1, "m");
    }

    #[test]
    fn scroll_up_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(0, 4, 0, 4, -2, 0);
        assert_grid_cell_equal_to_char(&grid, 0, 3, "e");
        assert_grid_cell_equal_to_char(&grid, 3, 3, "h");
        assert_grid_cell_equal_to_char(&grid, 0, 2, "a");
    }

    #[test]
    fn partial_scroll_lines_down_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(1, 3, 0, 4, 1, 0);
        // The initial line is not touched
        assert_grid_cell_equal_to_char(&grid, 0, 0, "a");

        assert_grid_cell_equal_to_char(&grid, 0, 1, "i");
        assert_grid_cell_equal_to_char(&grid, 3, 1, "l");

        // The last line is not touched either
        assert_grid_cell_equal_to_char(&grid, 0, 3, "m");
    }

    #[test]
    fn partial_scroll_lines_up_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(1, 3, 0, 4, -1, 0);
        // The initial line is not touched
        assert_grid_cell_equal_to_char(&grid, 0, 0, "a");

        assert_grid_cell_equal_to_char(&grid, 0, 2, "e");
        assert_grid_cell_equal_to_char(&grid, 3, 2, "h");

        // The last line is not touched either
        assert_grid_cell_equal_to_char(&grid, 0, 3, "m");
    }

    #[test]
    fn scroll_left_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(0, 4, 0, 4, 0, 1);
        assert_grid_cell_equal_to_char(&grid, 0, 0, "b");
        assert_grid_cell_equal_to_char(&grid, 2, 2, "l");
    }

    #[test]
    fn scroll_right_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(0, 4, 0, 4, 0, -3);
        assert_grid_cell_equal_to_char(&grid, 3, 0, "a");
        assert_grid_cell_equal_to_char(&grid, 3, 3, "m");
    }

    #[test]
    fn scroll_inner_box_diagonally_moves_the_grid_correctly() {
        let mut grid = create_initialized_grid(&["abcd", "efgh", "ijkl", "mnop"].to_vec());

        grid.scroll_region(1, 3, 1, 3, 1, 1);
        // The first row is preserved
        assert_grid_cell_equal_to_char(&grid, 0, 0, "a");
        assert_grid_cell_equal_to_char(&grid, 1, 0, "b");

        // The first character is not touched
        assert_grid_cell_equal_to_char(&grid, 0, 1, "e");

        // Only k is part of the box now
        assert_grid_cell_equal_to_char(&grid, 1, 1, "k");

        // The last character is not touched
        assert_grid_cell_equal_to_char(&grid, 3, 1, "h");

        // The last row is preserved
        assert_grid_cell_equal_to_char(&grid, 0, 3, "m");
    }
}
