use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

#[cfg(test)]
mod tests;

pub(super) struct AgentDrag {
    endpoint_id: ClientEndpointId,
    boot_id: String,
    workspace_id: String,
    tab_id: String,
    pane_ids: Vec<String>,
    start: (u16, u16),
    dragging: bool,
    target: Option<String>,
}

impl ClientShellState {
    pub(super) fn start_agent_press(
        &mut self,
        endpoint: &ClientEndpointId,
        pane_id: &str,
        point: (u16, u16),
    ) {
        if endpoint != &self.active_endpoint_id
            || self.sidebar_collapsed
            || !self.config.mouse_capture
        {
            return;
        }
        let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|candidate| &candidate.endpoint_id == endpoint)
        else {
            return;
        };
        if endpoint.status != ClientEndpointStatus::Online
            || !endpoint
                .methods
                .as_ref()
                .is_some_and(|methods| methods.contains("tab.transfer"))
        {
            return;
        }
        let Some(snapshot) = endpoint.snapshot.as_deref() else {
            return;
        };
        let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return;
        };
        let mut pane_ids = snapshot
            .panes
            .iter()
            .filter(|other| other.tab_id == pane.tab_id && other.workspace_id == pane.workspace_id)
            .map(|pane| pane.pane_id.clone())
            .collect::<Vec<_>>();
        pane_ids.sort();
        self.agent_drag = Some(AgentDrag {
            endpoint_id: endpoint.endpoint_id.clone(),
            boot_id: snapshot.boot_id.clone(),
            workspace_id: pane.workspace_id.clone(),
            tab_id: pane.tab_id.clone(),
            pane_ids,
            start: point,
            dragging: false,
            target: None,
        });
    }

    fn agent_drag_is_current(&self, drag: &AgentDrag) -> bool {
        if drag.endpoint_id != self.active_endpoint_id
            || !self.config.mouse_capture
            || self.overlay.is_some()
            || self.popup_pending
            || self.popup_terminal_id.is_some()
        {
            return false;
        }
        let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == drag.endpoint_id)
        else {
            return false;
        };
        if endpoint.status != ClientEndpointStatus::Online
            || !endpoint
                .methods
                .as_ref()
                .is_some_and(|methods| methods.contains("tab.transfer"))
        {
            return false;
        }
        let Some(snapshot) = endpoint
            .snapshot
            .as_deref()
            .filter(|snapshot| snapshot.boot_id == drag.boot_id)
        else {
            return false;
        };
        let panes = || {
            snapshot
                .panes
                .iter()
                .filter(|pane| pane.tab_id == drag.tab_id && pane.workspace_id == drag.workspace_id)
        };
        panes().count() == drag.pane_ids.len()
            && panes().all(|pane| drag.pane_ids.binary_search(&pane.pane_id).is_ok())
    }

    pub(super) fn cancel_stale_agent_drag(&mut self) {
        if self
            .agent_drag
            .as_ref()
            .is_some_and(|drag| !self.agent_drag_is_current(drag))
        {
            self.agent_drag = None;
        }
    }

    pub(super) fn reveal_transferred_agent(&mut self) {
        let Some(tab_id) = self.pending_transfer_reveal.as_deref() else {
            return;
        };
        let Some(pane_id) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.tab_id == tab_id)
                .map(|agent| agent.pane_id.clone())
        }) else {
            return;
        };
        self.pending_transfer_reveal = None;
        let endpoint_id = self.active_endpoint_id.clone();
        self.reveal_endpoint_agent(&endpoint_id, &pane_id, self.hits.agent_body.height);
    }

    fn agent_drop_target(&self, point: (u16, u16), drag: &AgentDrag) -> Option<String> {
        let snapshot = self.snapshot.as_deref()?;
        let workspace = self
            .hits
            .workspaces
            .iter()
            .find(|hit| hit.endpoint_id == drag.endpoint_id && super::contains(hit.rect, point))
            .map(|hit| hit.workspace_id.as_str())
            .or_else(|| {
                self.hits
                    .agent_group_toggles
                    .iter()
                    .find(|(rect, endpoint, _)| {
                        endpoint == &drag.endpoint_id && super::contains(*rect, point)
                    })
                    .map(|(_, _, workspace)| workspace.as_str())
            })
            .or_else(|| {
                self.hits
                    .endpoint_agents
                    .iter()
                    .find(|(rect, endpoint, _)| {
                        endpoint == &drag.endpoint_id && super::contains(*rect, point)
                    })
                    .and_then(|(_, _, pane)| {
                        snapshot
                            .panes
                            .iter()
                            .find(|candidate| &candidate.pane_id == pane)
                    })
                    .map(|pane| pane.workspace_id.as_str())
            })
            .or_else(|| {
                self.hits
                    .agents
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                    .and_then(|(_, pane)| {
                        snapshot
                            .panes
                            .iter()
                            .find(|candidate| &candidate.pane_id == pane)
                    })
                    .map(|pane| pane.workspace_id.as_str())
            })?;
        (workspace != drag.workspace_id
            && snapshot
                .workspaces
                .iter()
                .any(|ws| ws.workspace_id == workspace))
        .then(|| workspace.to_owned())
    }

    pub(super) fn handle_agent_drag(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(mut drag) = self.agent_drag.take() else {
            return false;
        };
        let point = (mouse.column, mouse.row);
        let owned = matches!(
            mouse.kind,
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        );
        if !self.agent_drag_is_current(&drag) {
            outcome.repaint = true;
            return owned;
        }
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                drag.dragging |= point.0.abs_diff(drag.start.0) >= 2 || point.1 != drag.start.1;
                if drag.dragging {
                    drag.target = self.agent_drop_target(point, &drag);
                    outcome.repaint = true;
                }
                self.agent_drag = Some(drag);
                true
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if drag.dragging {
                    if let Some(workspace_id) = self.agent_drop_target(point, &drag) {
                        self.push_endpoint_method_with_kind(
                            crate::api::schema::Method::TabTransfer(
                                crate::api::schema::TabTransferParams {
                                    tab_id: drag.tab_id,
                                    source_workspace_id: drag.workspace_id,
                                    workspace_id: workspace_id.clone(),
                                    expected_pane_ids: drag.pane_ids,
                                    focus: true,
                                },
                            ),
                            PendingEndpointKind::AgentTransfer { workspace_id },
                            outcome,
                        );
                    }
                    outcome.repaint = true;
                }
                true
            }
            MouseEventKind::Down(_) => false,
            _ => {
                self.agent_drag = Some(drag);
                false
            }
        }
    }

    pub(super) fn render_agent_drag(&self, buffer: &mut Buffer) {
        let Some(drag) = self
            .agent_drag
            .as_ref()
            .filter(|drag| drag.dragging && self.agent_drag_is_current(drag))
        else {
            return;
        };
        let palette = &self.config.palette;
        for hit in &self.hits.workspaces {
            if hit.endpoint_id == drag.endpoint_id
                && Some(&hit.workspace_id) == drag.target.as_ref()
            {
                buffer.set_style(
                    hit.rect,
                    Style::default()
                        .bg(palette.surface1)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        for (rect, endpoint, workspace) in &self.hits.agent_group_toggles {
            if endpoint == &drag.endpoint_id && Some(workspace) == drag.target.as_ref() {
                buffer.set_style(
                    *rect,
                    Style::default()
                        .bg(palette.surface1)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        let label = drag
            .target
            .as_ref()
            .and_then(|workspace| {
                self.snapshot
                    .as_deref()?
                    .workspaces
                    .iter()
                    .find(|ws| &ws.workspace_id == workspace)
            })
            .map(|ws| ws.label.as_str());
        let text = label.map_or_else(
            || " drop on a space · Esc cancel".into(),
            |label| format!(" move → {label}"),
        );
        let body = self.hits.agent_body;
        if body.y > 0 && body.width > 0 {
            super::render::put_text(
                buffer,
                body.x,
                body.y - 1,
                body.width,
                &text,
                Style::default().fg(palette.accent),
            );
        }
    }
}
