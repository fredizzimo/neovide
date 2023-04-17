use std::collections::VecDeque;


struct ScrollbackBuffer<LineType> {
    actual_lines: Vec<Option<LineType>>,
    scrollback_lines: VecDeque<(isize, LineType)>,
    actual_position: isize,
    scroll_position: f64,
}

impl<LineType: Clone> ScrollbackBuffer<LineType> {
    fn new(size: usize) -> Self {
        Self {
            actual_lines: vec![None; size],
            scrollback_lines: VecDeque::new(),
            actual_position: 0,
            scroll_position: 0.0,
        }
    }

    fn scroll_internal(&mut self, top: isize, bottom: isize, rows: isize) {
        let mut top_to_bottom;
        let mut bottom_to_top;
        let y_iter: &mut dyn Iterator<Item = isize > = if rows > 0 {
            top_to_bottom = top + rows..bottom;
            &mut top_to_bottom
        } else {
            bottom_to_top = (top..(bottom + rows)).rev();
            &mut bottom_to_top
        };

        // Swap the lines instead of copying since the source lines will be overwritten anyway
        for y in y_iter {
            let dest_y = (y - rows) as usize;
            self.actual_lines.swap(dest_y, y as usize);
        }
    }

    fn scroll(&mut self, rows: isize) {
        let prev_position = self.actual_position;
        self.actual_position += rows;
        self.cleanup_scrollback();

        if rows.abs() < self.actual_lines.len() as isize {
            if rows >  0 {
                // Check if we need to extend the scrollback buffer
                // If the scroll direction has changed it might have been shrunk by the cleanup_scrollback function instead.
                if self.scrollback_lines.iter().last().map_or(true, |v| v.0 < self.actual_position) {
                    let source = &self.actual_lines[0..rows as usize];
                    for (i, line) in source.iter().enumerate() {
                        if let Some(picture) = line {
                            self.scrollback_lines.push_back((prev_position + i as isize, picture.clone()));
                        }
                    }
                }
            } else {
                // Check if we need to extend the scrollback buffer
                // If the scroll direction has changed it might have been shrunk by the cleanup_scrollback function instead.
                if self.scrollback_lines.iter().next().map_or(true, |v| v.0 > self.actual_position) {
                    let source = self.actual_lines.iter().rev().take(-rows as usize);
                    for (i, line) in source.enumerate() {
                        if let Some(picture) = line {
                            self.scrollback_lines.push_front((prev_position + (rows - 1 - i as isize), picture.clone()));
                        }
                    }
                }
            };
        }
    }

    fn cleanup_scrollback(&mut self) {
        let (first_valid, last_valid) = if self.scroll_position <= self.actual_position as f64 {
            (self.scroll_position.floor() as isize, self.actual_position - 1)
        } else {
            (self.actual_position + self.actual_lines.len() as isize, self.scroll_position.ceil() as isize)
        };
        self.scrollback_lines.drain(0..self.scrollback_lines.partition_point(|line| line.0 < first_valid));
        self.scrollback_lines.drain(self.scrollback_lines.partition_point(|line| line.0 > last_valid)..);
    }

    fn get_visible_line(&self, index: usize) -> Option<&LineType> {
        let start_virtual_line = self.scroll_position.floor();
        let start_virtual_line = start_virtual_line as isize;

        let virtual_line = start_virtual_line + index as isize;
        let offset = virtual_line - self.actual_position;
        if offset >= 0 && offset < self.actual_lines.len() as isize {
            self.actual_lines[offset as usize].as_ref()
        } else if let Ok(index) = self.scrollback_lines.binary_search_by_key(&virtual_line, |line| line.0) {
            Some(&self.scrollback_lines[index].1)
        } else {
            None
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn assign_lines(buffer: &mut ScrollbackBuffer<i32>, lines: &[i32]) {
        buffer.actual_lines.iter_mut().zip(lines.iter()).for_each(|(line, new_value)| *line=Some(*new_value));
    }

    fn assign_lines_at(buffer: &mut ScrollbackBuffer<i32>, pos: usize, lines: &[i32]) {
        buffer.actual_lines.iter_mut().skip(pos).zip(lines.iter()).for_each(|(line, new_value)| *line=Some(*new_value));
    }

    fn lines(lines: &[i32]) -> Vec<Option<i32>> {
        lines.iter().map(|v| Some(*v)).collect()
    }

    fn get_visible_lines(buffer: &ScrollbackBuffer<i32>) -> Vec<Option<i32>> {
        // Always return one extra line, to simulate what's happening when scrolling
        (0..buffer.actual_lines.len() + 1).map(|i| buffer.get_visible_line(i).cloned()).collect()
    }

    #[test]
    fn create() {
        let buffer = ScrollbackBuffer::<i32>::new(3);
        assert_eq!(buffer.actual_lines, [None, None, None]);
        assert_eq!(buffer.scrollback_lines.len(), 0);
        assert_eq!(buffer.actual_position, 0);
        assert_eq!(buffer.scroll_position, 0.0);
    }

    #[test]
    fn scroll_internal_down() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, 3);
        assert_eq!(buffer.actual_lines[0..2], lines(&[4, 5]));
    }

    #[test]
    fn scroll_internal_down_one_less_than_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, 4);
        assert_eq!(buffer.actual_lines[0..1], lines(&[5]));
    }

    #[test]
    fn scroll_internal_down_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, 5);
        // Nothing should happen, since everything is invalidated
        assert_eq!(buffer.actual_lines[0..5], lines(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn scroll_internal_down_more_than_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, 5);
        // Nothing should happen, since everything is invalidated
        assert_eq!(buffer.actual_lines[0..5], lines(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn scroll_internal_up() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, -3);
        assert_eq!(buffer.actual_lines[3..5], lines(&[1, 2]));
    }

    #[test]
    fn scroll_internal_up_one_less_than_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, -4);
        assert_eq!(buffer.actual_lines[4..5], lines(&[1]));
    }

    #[test]
    fn scroll_internal_up_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, -5);
        // Nothing should happen, since everything is invalidated
        assert_eq!(buffer.actual_lines[0..5], lines(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn scroll_internal_up_more_than_full() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(0, 5, -5);
        // Nothing should happen, since everything is invalidated
        assert_eq!(buffer.actual_lines[0..5], lines(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn scroll_internal_middle_down() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(1, 4, 1);
        assert_eq!(buffer.actual_lines[0..3], lines(&[1, 3, 4]));
        assert_eq!(buffer.actual_lines[4..5], lines(&[5]));
    }

    #[test]
    fn scroll_internal_middle_up() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll_internal(1, 4, -1);
        assert_eq!(buffer.actual_lines[0..1], lines(&[1]));
        assert_eq!(buffer.actual_lines[2..5], lines(&[2, 3, 5]));
    }

    #[test]
    fn scroll_down() {
        let mut buffer = ScrollbackBuffer::<i32>::new(5);
        assign_lines(&mut buffer, &[1, 2, 3, 4, 5]);
        buffer.scroll(2);
        buffer.scroll_internal(0, 5, 2);
        assign_lines_at(&mut buffer, 3, &[6, 7]);
        assert_eq!(buffer.scroll_position, 0.0);
        assert_eq!(buffer.actual_position, 2);
        assert_eq!(get_visible_lines(&buffer), lines(&[1, 2, 3, 4, 5, 6]));
        buffer.scroll_position = 1.0;
        assert_eq!(get_visible_lines(&buffer), lines(&[2, 3, 4, 5, 6, 7]));
        buffer.scroll_position = 2.0;
        assert_eq!(get_visible_lines(&buffer), &[Some(3), Some(4), Some(5), Some(6), Some(7), None]);
    }
}
