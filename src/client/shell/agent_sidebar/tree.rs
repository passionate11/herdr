use super::layout::{GroupBox, TreeLink};
use super::{put_text, AgentRow, Buffer, Line, Palette, Paragraph, Rect, Style, Widget};

pub(super) fn status_indicator(
    status: crate::api::schema::AgentStatus,
    palette: &Palette,
) -> (&'static str, ratatui::style::Color) {
    use crate::api::schema::AgentStatus;

    // Keep agent-tree's compact symbols while retaining Herdr theme colors.
    let symbol = match status {
        AgentStatus::Working => "◐",
        AgentStatus::Blocked => "●",
        AgentStatus::Done => "✓",
        AgentStatus::Idle => "○",
        AgentStatus::Unknown => "·",
    };
    (symbol, super::status_color(status, palette))
}

pub(super) fn render_group(
    buffer: &mut Buffer,
    group: &GroupBox,
    row: &AgentRow,
    palette: &Palette,
) {
    let rect = group.rect;
    let branch = Style::default().fg(palette.overlay0);
    if group.split {
        for y in rect.y..rect.bottom() {
            let symbol = if rect.height == 1 {
                "─"
            } else if y == rect.y {
                "╭"
            } else if y + 1 == rect.bottom() {
                "╰"
            } else {
                "│"
            };
            put_text(buffer, rect.x, y, 1, symbol, branch);
        }
    } else {
        let title = Style::default().fg(if row.workspace_focused {
            palette.accent
        } else {
            palette.subtext0
        });
        Paragraph::new(Line::from(format!(
            "{} ▾ {}",
            row.workspace_number, row.workspace_label
        )))
        .style(title)
        .render(Rect::new(rect.x, rect.y, rect.width, 1), buffer);
        for y in rect.y + 1..rect.bottom() {
            put_text(buffer, rect.x + 1, y, 1, "│", branch);
        }
    }
}

pub(super) fn render_links(buffer: &mut Buffer, links: &[TreeLink], palette: &Palette) {
    let style = Style::default().fg(palette.overlay0);
    for link in links {
        let rect = link.rect;
        put_text(
            buffer,
            rect.x,
            rect.y,
            2,
            if link.last { "└─" } else { "├─" },
            style,
        );
        if let Some(join) = link.split_join {
            if let Some(cell) = buffer.cell_mut((rect.x + 2, rect.y)) {
                cell.set_char(join).set_style(style);
            }
        }
    }
}
