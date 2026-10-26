use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) mod layout;
use layout::AgentListGeometry;
#[cfg(test)]
mod tests;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) endpoint_id: Option<ClientEndpointId>,
    pub(super) workspace_id: String,
    pub(super) workspace_label: String,
    pub(super) tab_id: String,
    pub(super) workspace_focused: bool,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(status_priority(agent.agent_status)),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    render_agent_list(
        buffer,
        area,
        &rows,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row,
        |buffer, rect, row, boxed, cards, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            render_agent_row(buffer, rect, row, config, boxed, cards);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        " agents",
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    agent_row: impl Fn(&T) -> &AgentRow,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, bool, bool, &mut ShellHitMap),
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let geometry = AgentListGeometry::new(rows, &agent_row, config, body, empty_message.is_none());
    let metrics = geometry.metrics(*agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_body = Rect::new(
        body.x,
        body.y,
        body.width
            .saturating_sub(u16::from(show_scrollbar && !geometry.boxed)),
        body.height,
    );
    let geometry = geometry.with_body(content_body);
    let layout = geometry.layout(*agent_scroll);
    for group in &layout.boxes {
        let row = agent_row(&rows[group.index]);
        render_group_box(
            buffer,
            group.rect,
            if group.split {
                ""
            } else {
                &row.workspace_label
            },
            !group.split && row.workspace_focused,
            &config.palette,
        );
    }
    for &(index, rect) in &layout.tails {
        let row = agent_row(&rows[index]);
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &"▀".repeat(rect.width as usize),
            Style::default().fg(agent_card_bg(config, row.focused)),
        );
    }
    for (index, rect) in layout.entries {
        render_row(
            buffer,
            rect,
            &rows[index],
            geometry.boxed,
            geometry.cards,
            hits,
        );
    }
    // Tails belong to the card above; border rows deliberately have no hit target.
    for (index, rect) in layout.tails {
        register_agent_hit(hits, agent_row(&rows[index]), rect);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine))
        .collect()
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
        .count();
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| tab.label.as_str());
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let labels = agent
        .state_labels
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let state_text = labels
        .get(status_text(agent.agent_status))
        .map(String::as_str)
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let canonical_agent = agent
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let rows = crate::ui::sidebar_agent_rows(
        &config.agents,
        crate::ui::AgentTokenContext {
            machine,
            workspace: &workspace.label,
            tab: tab_label,
            pane: agent
                .title
                .as_deref()
                .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
            agent_label,
            terminal_title: agent.terminal_title.as_deref(),
            terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
            canonical_agent,
            tokens: &tokens,
        },
        state_text,
    );
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        endpoint_id: None,
        workspace_id: agent.workspace_id.clone(),
        workspace_label: workspace.label.clone(),
        tab_id: agent.tab_id.clone(),
        workspace_focused: workspace.focused,
        status: agent.agent_status,
        focused: agent.focused,
        rows,
    })
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
    boxed: bool,
    cards: bool,
) {
    let palette = &config.palette;
    let row_style = if cards {
        Style::default().bg(agent_card_bg(config, row.focused))
    } else if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color(row.status, palette));
    let secondary = Style::default().fg(palette.overlay0);
    let icon = (
        status_icon(row.status, config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows
            .iter()
            .map(|tokens| {
                let other_text = tokens.iter().any(|token| {
                    !matches!(
                        token.kind,
                        crate::ui::ResolvedTokenKind::StateIcon
                            | crate::ui::ResolvedTokenKind::Workspace(_)
                    )
                });
                tokens
                    .iter()
                    .filter(|token| {
                        !(boxed
                            && other_text
                            && matches!(token.kind, crate::ui::ResolvedTokenKind::Workspace(_)))
                    })
                    .cloned()
                    .collect()
            })
            .collect()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn register_agent_hit(hits: &mut ShellHitMap, row: &AgentRow, rect: Rect) {
    if let Some(endpoint_id) = &row.endpoint_id {
        hits.endpoint_agents
            .push((rect, endpoint_id.clone(), row.pane_id.clone()));
    } else {
        hits.agents.push((rect, row.pane_id.clone()));
    }
}

fn agent_card_bg(config: &ClientShellConfig, focused: bool) -> ratatui::style::Color {
    match (config.agent_card_background, config.agent_card_foreground) {
        (Some(background), Some(foreground)) => {
            let amount = if focused { 0.09 } else { 0.035 };
            let mix = |base: u8, target: u8| {
                (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
            };
            ratatui::style::Color::Rgb(
                mix(background.r, foreground.r),
                mix(background.g, foreground.g),
                mix(background.b, foreground.b),
            )
        }
        _ if focused => config.palette.surface1,
        _ => config.palette.surface0,
    }
}

fn render_group_box(buffer: &mut Buffer, rect: Rect, label: &str, active: bool, palette: &Palette) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let border = Style::default().fg(if active {
        palette.accent
    } else {
        palette.overlay0
    });
    let title = Style::default()
        .fg(if active {
            palette.accent
        } else {
            palette.subtext0
        })
        .add_modifier(Modifier::BOLD);
    let inner = usize::from(rect.width - 2);
    let mut top = vec![ratatui::text::Span::styled("╭", border)];
    let mut used = 0;
    if !label.is_empty() && inner > 3 {
        let label = crate::ui::truncate_end(label, inner - 3);
        used = display_width(&label) + 3;
        top.push(ratatui::text::Span::styled("─ ", border));
        top.push(ratatui::text::Span::styled(label, title));
        top.push(ratatui::text::Span::styled(" ", border));
    }
    top.push(ratatui::text::Span::styled(
        "─".repeat(inner.saturating_sub(used)),
        border,
    ));
    top.push(ratatui::text::Span::styled("╮", border));
    Paragraph::new(Line::from(top)).render(Rect::new(rect.x, rect.y, rect.width, 1), buffer);
    for y in rect.y + 1..rect.bottom() - 1 {
        put_text(buffer, rect.x, y, 1, "│", border);
        put_text(buffer, rect.right() - 1, y, 1, "│", border);
    }
    put_text(
        buffer,
        rect.x,
        rect.bottom() - 1,
        rect.width,
        &format!("╰{}╯", "─".repeat(inner)),
        border,
    );
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}
