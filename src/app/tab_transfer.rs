use super::state::{AppState, Mode};

impl AppState {
    /// Move the original tab tree and viewport states; terminals and PTYs stay in place.
    pub(crate) fn transfer_tab(
        &mut self,
        source: usize,
        tab_index: usize,
        destination: usize,
        focus: bool,
    ) -> Option<usize> {
        if source == destination || destination >= self.workspaces.len() {
            return None;
        }
        let source_ws = self.workspaces.get(source)?;
        let tab = source_ws.tabs.get(tab_index)?;
        let pane_ids = tab.layout.pane_ids();
        let previous_ids = pane_ids
            .iter()
            .map(|pane_id| {
                (
                    *pane_id,
                    crate::workspace::public_pane_id_for_number(
                        &source_ws.id,
                        source_ws.public_pane_numbers[pane_id],
                    ),
                )
            })
            .collect::<Vec<_>>();
        let destination_id = self.workspaces[destination].id.clone();
        let active_id = self
            .active
            .and_then(|index| self.workspaces.get(index))
            .map(|ws| ws.id.clone());
        let selected_id = self.workspaces.get(self.selected).map(|ws| ws.id.clone());
        let previous_focus = self.current_pane_focus_target();
        let tab = self.workspaces[source].take_tab_for_transfer(tab_index)?;
        let target_tab = self.workspaces[destination].insert_transferred_tab(tab);
        for (pane_id, previous_id) in previous_ids {
            self.public_pane_id_aliases.insert(previous_id, pane_id);
        }
        if self.workspaces[source].tabs.is_empty() {
            self.workspaces.remove(source);
        }
        let destination = self
            .workspaces
            .iter()
            .position(|ws| ws.id == destination_id)?;
        self.active = active_id
            .and_then(|id| self.workspaces.iter().position(|ws| ws.id == id))
            .or(Some(destination));
        self.selected = selected_id
            .and_then(|id| self.workspaces.iter().position(|ws| ws.id == id))
            .unwrap_or(destination);
        if focus {
            let pane_id = self.workspaces[destination].tabs[target_tab]
                .layout
                .focused();
            self.switch_workspace_tab(destination, target_tab);
            self.record_pane_focus_change(previous_focus, destination, pane_id);
            self.mode = Mode::Terminal;
        }
        // References to an internal pane follow it to its new workspace.
        if let Some(previous) = self
            .previous_pane_focus
            .as_mut()
            .filter(|previous| pane_ids.contains(&previous.pane_id))
        {
            previous.workspace_id = destination_id.clone();
        }
        if let Some(target) = self
            .toast
            .as_mut()
            .and_then(|toast| toast.target.as_mut())
            .filter(|target| pane_ids.contains(&target.pane_id))
        {
            target.workspace_id = destination_id.clone();
        }
        for pane_id in pane_ids {
            if let Some(notification) = self.pending_agent_notifications.get_mut(&pane_id) {
                notification.workspace_id = destination_id.clone();
            }
        }
        self.mark_session_dirty();
        Some(target_tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{
        PaneFocusTarget, PendingAgentNotification, ToastKind, ToastNotification, ToastTarget,
    };
    use crate::workspace::Workspace;

    fn tab_snapshot(state: &AppState, ws: usize, tab: usize) -> serde_json::Value {
        let snapshot = crate::persist::capture(
            &state.workspaces,
            &state.terminals,
            &Default::default(),
            state.active,
            state.selected,
        );
        serde_json::to_value(&snapshot.workspaces[ws].tabs[tab]).unwrap()
    }

    #[test]
    fn tab_transfer_preserves_complete_tree_terminals_and_workspace_references() {
        for reverse in [false, true] {
            for last_tab in [false, true] {
                for focus in [false, true] {
                    let mut state = AppState::test_with_adversarial_identity_state();
                    let source = usize::from(reverse);
                    if reverse {
                        state
                            .workspaces
                            .insert(0, Workspace::test_new("destination"));
                    } else {
                        state.workspaces.push(Workspace::test_new("destination"));
                    }
                    let destination = usize::from(!reverse);
                    if last_tab {
                        state.workspaces[source] = Workspace::test_new("single-tab");
                        state.workspaces[source].test_split(ratatui::layout::Direction::Horizontal);
                        state.workspaces[source].test_split(ratatui::layout::Direction::Vertical);
                        state.terminals.clear();
                    }
                    let source_id = state.workspaces[source].id.clone();
                    let destination_id = state.workspaces[destination].id.clone();
                    let tab_index = state.workspaces[source]
                        .tabs
                        .iter()
                        .position(|tab| tab.panes.len() > 1)
                        .unwrap();
                    state.workspaces[source].tabs[tab_index].zoomed = true;
                    state.workspaces[source].tabs[tab_index].custom_name = Some("分窗组".into());
                    state.active = Some(source);
                    state.selected = destination;
                    state.ensure_test_terminals();
                    let pane_ids = state.workspaces[source].tabs[tab_index].layout.pane_ids();
                    let pane = pane_ids[1];
                    state.previous_pane_focus = Some(PaneFocusTarget {
                        workspace_id: source_id.clone(),
                        pane_id: pane,
                    });
                    state.toast = Some(ToastNotification {
                        kind: ToastKind::Finished,
                        title: "done".into(),
                        context: "test".into(),
                        position: None,
                        target: Some(ToastTarget {
                            workspace_id: source_id.clone(),
                            pane_id: pane,
                        }),
                    });
                    state.pending_agent_notifications.insert(
                        pane,
                        PendingAgentNotification {
                            pane_id: pane,
                            workspace_id: source_id.clone(),
                            agent_label: "codex".into(),
                            known_agent: None,
                            kind: ToastKind::Finished,
                            state: crate::detect::AgentState::Idle,
                            deadline: std::time::Instant::now(),
                        },
                    );
                    let before = tab_snapshot(&state, source, tab_index);
                    let terminals = state.terminals.len();
                    let old_numbers = state.workspaces[source].public_pane_numbers.clone();
                    state.assert_invariants_for_test();
                    let moved_tab = state
                        .transfer_tab(source, tab_index, destination, focus)
                        .unwrap();
                    state.assert_invariants_for_test();
                    let destination = state
                        .workspaces
                        .iter()
                        .position(|ws| ws.id == destination_id)
                        .unwrap();
                    assert_eq!(before, tab_snapshot(&state, destination, moved_tab));
                    assert_eq!(state.terminals.len(), terminals);
                    assert!(state.terminal_runtime_shutdowns.is_empty());
                    for pane in &pane_ids {
                        let old_id = crate::workspace::public_pane_id_for_number(
                            &source_id,
                            old_numbers[pane],
                        );
                        assert_eq!(state.public_pane_id_aliases.get(&old_id), Some(pane));
                    }
                    assert_eq!(
                        state.pending_agent_notifications[&pane].workspace_id,
                        destination_id
                    );
                    assert_eq!(
                        state
                            .toast
                            .as_ref()
                            .unwrap()
                            .target
                            .as_ref()
                            .unwrap()
                            .workspace_id,
                        destination_id
                    );
                    if focus {
                        assert_eq!(state.active, Some(destination));
                        assert_eq!(state.workspaces[destination].active_tab, moved_tab);
                    }
                    assert_eq!(
                        state.workspaces.iter().any(|ws| ws.id == source_id),
                        !last_tab
                    );
                }
            }
        }
    }
}
