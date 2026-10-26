use ratatui::layout::Rect;

use super::{AgentRow, ClientShellConfig};

mod tree;

#[derive(Clone, Copy)]
struct RowGeometry {
    height: u16,
    starts_space: bool,
    ends_space: bool,
    split: bool,
    starts_split: bool,
    ends_split: bool,
}

pub(in crate::client::shell) struct AgentListGeometry {
    body: Rect,
    rows: Vec<RowGeometry>,
    gap: u16,
    has_split_rows: bool,
    pub(super) tree: bool,
    pub(super) boxed: bool,
    pub(super) cards: bool,
}

#[derive(Default)]
pub(super) struct AgentListLayout {
    pub(super) entries: Vec<(usize, Rect)>,
    pub(super) tails: Vec<(usize, Rect)>,
    pub(super) boxes: Vec<GroupBox>,
    pub(super) links: Vec<TreeLink>,
}

pub(super) struct TreeLink {
    pub(super) rect: Rect,
    pub(super) last: bool,
    pub(super) split_join: Option<char>,
}

pub(super) struct GroupBox {
    pub(super) index: usize,
    pub(super) rect: Rect,
    pub(super) split: bool,
}

impl AgentListGeometry {
    pub(in crate::client::shell) fn new<T>(
        rows: &[T],
        agent_row: impl Fn(&T) -> &AgentRow,
        config: &ClientShellConfig,
        body: Rect,
        grouped_view: bool,
    ) -> Self {
        // Boxed lists already leave the last column free for the scrollbar.
        let width = body.width;
        let boxed = grouped_view
            && (config.agents.group_boxes || config.agents.group_tree)
            && config.agent_panel_sort == crate::config::AgentPanelSortConfig::Spaces
            && width >= 5
            && body.height >= 3;
        let tree = boxed && config.agents.group_tree && width >= 8;
        let split_boxes = boxed && width >= 10 && (tree || body.height >= 5);
        let same_space = |a: &AgentRow, b: &AgentRow| {
            a.endpoint_id == b.endpoint_id && a.workspace_id == b.workspace_id
        };
        let same_tab = |a: &AgentRow, b: &AgentRow| same_space(a, b) && a.tab_id == b.tab_id;
        let mut has_split_rows = false;
        let geometry = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let row = agent_row(row);
                let previous = index
                    .checked_sub(1)
                    .and_then(|i| rows.get(i))
                    .map(&agent_row);
                let next = rows.get(index + 1).map(&agent_row);
                let split = split_boxes
                    && (previous.is_some_and(|p| same_tab(p, row))
                        || next.is_some_and(|n| same_tab(row, n)));
                has_split_rows |= split;
                let reserved = if tree {
                    1
                } else {
                    2 * u16::from(boxed) + 2 * u16::from(split)
                };
                let max_height = body.height.saturating_sub(reserved);
                RowGeometry {
                    height: (row.rows.len().max(1).min(u16::MAX as usize) as u16).min(max_height),
                    starts_space: boxed && previous.is_none_or(|p| !same_space(p, row)),
                    ends_space: boxed && next.is_none_or(|n| !same_space(row, n)),
                    split,
                    starts_split: split && previous.is_none_or(|p| !same_tab(p, row)),
                    ends_split: split && next.is_none_or(|n| !same_tab(row, n)),
                }
            })
            .collect();
        Self {
            body,
            rows: geometry,
            gap: config.agents.row_gap,
            has_split_rows,
            tree,
            boxed,
            cards: boxed && config.agents.cards,
        }
    }

    pub(super) fn with_body(mut self, body: Rect) -> Self {
        self.body = body;
        self
    }

    fn gap_after(&self, index: usize) -> u16 {
        let row = self.rows[index];
        if index + 1 == self.rows.len()
            || row.ends_space
            || row.ends_split
            || (!row.split && self.rows[index + 1].starts_split)
        {
            0
        } else {
            self.gap
        }
    }

    fn max_scroll(&self) -> usize {
        if self.tree {
            return self.tree_max_scroll();
        }
        let mut suffix = 0u32;
        let mut start = self.rows.len();
        for (index, row) in self.rows.iter().enumerate().rev() {
            suffix += u32::from(row.height) + u32::from(row.ends_split) + u32::from(row.ends_space);
            if index + 1 < self.rows.len() {
                let next = self.rows[index + 1];
                suffix += u32::from(self.gap_after(index))
                    + u32::from(next.starts_space)
                    + u32::from(next.starts_split);
            }
            let needed = suffix + u32::from(self.boxed) + u32::from(row.split);
            if needed > u32::from(self.body.height) {
                break;
            }
            start = index;
        }
        start.min(self.rows.len().saturating_sub(1))
    }

    pub(super) fn metrics(&self, requested_start: usize) -> crate::pane::ScrollMetrics {
        let max_scroll = self.max_scroll();
        let start = requested_start.min(max_scroll);
        crate::pane::ScrollMetrics {
            offset_from_bottom: max_scroll.saturating_sub(start),
            max_offset_from_bottom: max_scroll,
            viewport_rows: self.layout(start).entries.len(),
        }
    }

    pub(in crate::client::shell) fn reveal(&self, requested_start: usize, target: usize) -> usize {
        let max_scroll = self.max_scroll();
        let mut start = requested_start.min(max_scroll);
        if target < start {
            return target.min(max_scroll);
        }
        while start < max_scroll {
            let visible = self.layout(start).entries.len();
            if target < start.saturating_add(visible) {
                break;
            }
            start += 1;
        }
        start
    }

    pub(super) fn layout(&self, scroll: usize) -> AgentListLayout {
        if self.tree {
            return self.tree_layout(scroll);
        }
        let mut layout = AgentListLayout::default();
        if self.body.is_empty() {
            return layout;
        }
        let inset = u16::from(self.boxed);
        let box_width = self.body.width.saturating_sub(inset);
        let inner_width = box_width.saturating_sub(2 * inset);
        // Reserve the subgroup border in the shared gutter so all cards align.
        let margin = u16::from(self.has_split_rows || (self.cards && inner_width >= 10));
        let mut y = self.body.y;
        let mut content_bottom = y;
        let mut space = None;
        let mut split = None;
        let mut pending_tail = None;
        for (index, row) in self.rows.iter().enumerate().skip(scroll) {
            let starts_space = self.boxed && (index == scroll || row.starts_space);
            let starts_split = row.split && (index == scroll || row.starts_split);
            let split_top = y + u16::from(starts_space);
            let content_y = split_top + u16::from(starts_split);
            if content_y
                .saturating_add(row.height)
                .saturating_add(inset + u16::from(row.split))
                > self.body.bottom()
            {
                break;
            }
            layout.tails.extend(pending_tail.take());
            if starts_space {
                space = Some((index, y));
            }
            if starts_split {
                split = Some((index, split_top));
            }
            let rect = Rect::new(
                self.body.x + inset + margin,
                content_y,
                inner_width - 2 * margin,
                row.height,
            );
            layout.entries.push((index, rect));
            y = content_y + row.height;
            if row.ends_split {
                if let Some((first, top)) = split.take() {
                    layout.boxes.push(GroupBox {
                        index: first,
                        rect: Rect::new(self.body.x + 1, top, inner_width, y - top + 1),
                        split: true,
                    });
                }
                y += 1;
            }
            content_bottom = y;
            if row.ends_space {
                if let Some((first, top)) = space.take() {
                    layout.boxes.push(GroupBox {
                        index: first,
                        rect: Rect::new(self.body.x, top, box_width, y - top + 1),
                        split: false,
                    });
                }
                y += 1;
            } else {
                let gap = self.gap_after(index);
                if self.cards && gap > 0 {
                    pending_tail = Some((index, Rect::new(rect.x, y, rect.width, 1)));
                }
                y = y.saturating_add(gap).min(self.body.bottom());
            }
        }
        if let Some((index, top)) = split {
            layout.boxes.push(GroupBox {
                index,
                rect: Rect::new(self.body.x + 1, top, inner_width, content_bottom - top + 1),
                split: true,
            });
            content_bottom += 1;
        }
        if let Some((index, top)) = space {
            layout.boxes.push(GroupBox {
                index,
                rect: Rect::new(self.body.x, top, box_width, content_bottom - top + 1),
                split: false,
            });
        }
        layout
    }
}
