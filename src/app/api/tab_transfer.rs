use super::responses::{encode_error, encode_success};
use crate::api::schema::{EventData, EventEnvelope, EventKind, ResponseResult, TabTransferParams};
use crate::app::App;

impl App {
    pub(super) fn handle_tab_transfer(&mut self, id: String, params: TabTransferParams) -> String {
        let Some((source, tab_index)) = self.parse_tab_id(&params.tab_id) else {
            return encode_error(id, "tab_not_found", "source tab no longer exists");
        };
        let Some(destination) = self.parse_workspace_id(&params.workspace_id) else {
            return encode_error(
                id,
                "workspace_not_found",
                "destination workspace no longer exists",
            );
        };
        let source_id = self.public_workspace_id(source);
        let destination_id = self.public_workspace_id(destination);
        let Some(source_tab) = self.tab_info(source, tab_index) else {
            return encode_error(id, "tab_not_found", "source tab no longer exists");
        };
        let source_tab_id = source_tab.tab_id.clone();
        let panes = self.state.workspaces[source].tabs[tab_index]
            .layout
            .pane_ids();
        let previous_ids = panes
            .iter()
            .map(|pane_id| {
                crate::workspace::public_pane_id_for_number(
                    &source_id,
                    self.state.workspaces[source].public_pane_numbers[pane_id],
                )
            })
            .collect::<Vec<_>>();
        let mut expected = params.expected_pane_ids;
        expected.sort();
        let mut current = previous_ids.clone();
        current.sort();
        if source_id != params.source_workspace_id || current != expected {
            return encode_error(
                id,
                "stale_target",
                "The split group changed while dragging. Drag it again.",
            );
        }
        if source == destination {
            return encode_success(id, ResponseResult::TabInfo { tab: source_tab });
        }
        let Some(target_tab) =
            self.state
                .transfer_tab(source, tab_index, destination, params.focus)
        else {
            return encode_error(id, "stale_target", "split group is no longer available");
        };
        let Some(destination) = self
            .state
            .workspaces
            .iter()
            .position(|ws| ws.id == destination_id)
        else {
            return encode_error(
                id,
                "workspace_not_found",
                "destination workspace no longer exists",
            );
        };
        let Some(tab) = self.tab_info(destination, target_tab) else {
            return encode_error(id, "tab_not_found", "transferred tab no longer exists");
        };
        let closed_workspace_id =
            (!self.state.workspaces.iter().any(|ws| ws.id == source_id)).then(|| source_id.clone());
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id: source_tab_id.clone(),
                workspace_id: source_id.clone(),
            },
        });
        if let Some(workspace_id) = &closed_workspace_id {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id: workspace_id.clone(),
                    workspace: None,
                },
            });
        }
        self.emit_event(EventEnvelope {
            event: EventKind::TabCreated,
            data: EventData::TabCreated { tab: tab.clone() },
        });
        for (pane_id, previous_pane_id) in panes.into_iter().zip(previous_ids) {
            let Some(pane) = self.pane_info(destination, pane_id) else {
                continue;
            };
            self.emit_event(EventEnvelope {
                event: EventKind::PaneMoved,
                data: EventData::PaneMoved {
                    previous_pane_id,
                    previous_workspace_id: source_id.clone(),
                    previous_tab_id: source_tab_id.clone(),
                    pane: Box::new(pane),
                    created_workspace: None,
                    created_tab: Some(tab.clone()),
                    closed_workspace_id: closed_workspace_id.clone(),
                    closed_tab_id: Some(source_tab_id.clone()),
                },
            });
        }
        if let Some(layout) = self.pane_layout_snapshot(destination, target_tab) {
            self.emit_layout_updated_snapshot(layout);
        }
        encode_success(id, ResponseResult::TabInfo { tab })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{ErrorResponse, SuccessResponse};
    use crate::app::{AppPolicy, AppState};

    #[test]
    fn tab_transfer_api_rejects_changed_members_without_mutation_and_emits_moves() {
        let (_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let hub = crate::api::EventHub::default();
        let mut app = App::new(
            &crate::config::Config::default(),
            AppPolicy::TEST,
            None,
            receiver,
            hub.clone(),
        );
        app.state = AppState::test_with_adversarial_identity_state();
        app.state
            .workspaces
            .push(crate::workspace::Workspace::test_new("target"));
        app.state.ensure_test_terminals();
        let index = app.state.workspaces[0]
            .tabs
            .iter()
            .position(|tab| tab.panes.len() > 1)
            .unwrap();
        let pane_ids = app.state.workspaces[0].tabs[index].layout.pane_ids();
        let source_id = app.public_workspace_id(0);
        let target_id = app.public_workspace_id(1);
        let tab_id = app.public_tab_id(0, index).unwrap();
        let expected = pane_ids
            .iter()
            .map(|pane| {
                crate::workspace::public_pane_id_for_number(
                    &source_id,
                    app.state.workspaces[0].public_pane_numbers[pane],
                )
            })
            .collect::<Vec<_>>();
        let mut params = TabTransferParams {
            tab_id,
            source_workspace_id: source_id,
            workspace_id: target_id.clone(),
            expected_pane_ids: expected.clone(),
            focus: true,
        };
        params.expected_pane_ids.pop();
        let rejected: ErrorResponse =
            serde_json::from_str(&app.handle_tab_transfer("stale".into(), params.clone())).unwrap();
        assert_eq!(rejected.error.code, "stale_target");
        assert!(hub.events_after(0).is_empty());
        assert_eq!(
            app.state.workspaces[0].tabs[index].layout.pane_ids(),
            pane_ids
        );
        params.expected_pane_ids = expected;
        let moved: SuccessResponse =
            serde_json::from_str(&app.handle_tab_transfer("move".into(), params)).unwrap();
        assert!(
            matches!(moved.result, ResponseResult::TabInfo { tab } if tab.workspace_id == target_id && tab.pane_count == pane_ids.len())
        );
        app.state.assert_invariants_for_test();
        assert_eq!(
            hub.events_after(0)
                .iter()
                .filter(|(_, event)| event.event == EventKind::PaneMoved)
                .count(),
            pane_ids.len()
        );
    }
}
