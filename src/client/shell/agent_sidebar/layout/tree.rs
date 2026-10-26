use super::{AgentListGeometry, AgentListLayout, GroupBox, Rect, TreeLink};

impl AgentListGeometry {
    pub(super) fn tree_max_scroll(&self) -> usize {
        let mut suffix = 0u32;
        let mut start = self.rows.len();
        for (index, row) in self.rows.iter().enumerate().rev() {
            suffix += u32::from(row.height);
            if index + 1 < self.rows.len() {
                suffix += if row.ends_space {
                    2
                } else {
                    u32::from(self.gap)
                };
            }
            // Every scrolled space starts with its own visible heading.
            if suffix + 1 > u32::from(self.body.height) {
                break;
            }
            start = index;
        }
        start.min(self.rows.len().saturating_sub(1))
    }

    pub(super) fn tree_layout(&self, scroll: usize) -> AgentListLayout {
        let mut layout = AgentListLayout::default();
        let mut index = scroll;
        let mut y = self.body.y;
        while let Some(row) = self.rows.get(index) {
            if y.saturating_add(1 + row.height) > self.body.bottom() {
                break;
            }
            let heading_y = y;
            y += 1;
            let first_entry = layout.entries.len();
            loop {
                let row = self.rows[index];
                if y.saturating_add(row.height) > self.body.bottom() {
                    break;
                }
                let rect = Rect::new(self.body.x + 4, y, self.body.width - 5, row.height);
                layout.entries.push((index, rect));
                y += row.height;
                index += 1;
                if row.ends_space || index == self.rows.len() {
                    break;
                }
                y = y.saturating_add(self.gap);
            }

            // Group only visible entries. Clipped tabs get a complete short branch,
            // while every card retains the same x coordinate and hit rectangle.
            let end = layout.entries.len();
            let mut entry = first_entry;
            while entry < end {
                let (first, rect) = layout.entries[entry];
                let mut last = entry;
                if self.rows[first].split {
                    while last + 1 < end && !self.rows[layout.entries[last].0].ends_split {
                        last += 1;
                    }
                }
                let bottom = layout.entries[last].1.y;
                let anchor = rect.y + (bottom - rect.y) / 2;
                let split_join = self.rows[first].split.then_some(if bottom == rect.y {
                    '─'
                } else if anchor == rect.y {
                    '┬'
                } else {
                    '┤'
                });
                if split_join.is_some() {
                    layout.boxes.push(GroupBox {
                        index: first,
                        rect: Rect::new(self.body.x + 3, rect.y, 1, bottom - rect.y + 1),
                        split: true,
                    });
                }
                layout.links.push(TreeLink {
                    rect: Rect::new(self.body.x + 1, anchor, 3, 1),
                    last: last + 1 == end,
                    split_join,
                });
                entry = last + 1;
            }
            let last_link = layout.links.last().expect("a visible space has a branch");
            layout.boxes.push(GroupBox {
                index: layout.entries[first_entry].0,
                rect: Rect::new(
                    self.body.x,
                    heading_y,
                    self.body.width - 1,
                    last_link.rect.y - heading_y + 1,
                ),
                split: false,
            });
            if self.cards && self.gap > 0 {
                for pair in layout.entries[first_entry..end].windows(2) {
                    let (first, rect) = pair[0];
                    layout
                        .tails
                        .push((first, Rect::new(rect.x, rect.bottom(), rect.width, 1)));
                }
            }
            if !self.rows[layout.entries[end - 1].0].ends_space {
                break;
            }
            y = layout.entries[end - 1].1.bottom().saturating_add(1);
        }
        layout
    }
}
