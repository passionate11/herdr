use super::*;
use crate::api::schema::Method;
use crate::raw_input::RawInputEvent;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn setup() -> ClientShellState {
    let mut config = Config::default();
    config.ui.sidebar.agents.group_tree = true;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut snapshot = super::super::tests::snapshot();
    snapshot
        .workspaces
        .push(crate::protocol::ClientShellWorkspace {
            workspace_id: "ws_2".into(),
            number: 2,
            label: "destination".into(),
            focused: false,
            ..snapshot.workspaces[0].clone()
        });
    for index in 1..4 {
        let workspace_id = if index < 3 { "ws_1" } else { "ws_2" };
        let tab_id = if index < 3 { "tab_1" } else { "tab_2" };
        snapshot.panes.push(crate::protocol::ClientShellPane {
            pane_id: format!("pane_{}", index + 1),
            workspace_id: workspace_id.into(),
            tab_id: tab_id.into(),
            focused: false,
            ..snapshot.panes[0].clone()
        });
    }
    // The third source pane is a shell: it must travel with the two agents.
    snapshot.agents = [0, 1, 3]
        .into_iter()
        .map(|index| crate::protocol::ClientShellAgent {
            pane_id: snapshot.panes[index].pane_id.clone(),
            workspace_id: snapshot.panes[index].workspace_id.clone(),
            tab_id: snapshot.panes[index].tab_id.clone(),
            name: None,
            display_agent: None,
            agent: Some("codex".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Idle,
            state_change_seq: 0,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: index == 0,
        })
        .collect();
    snapshot.tabs.push(crate::protocol::ClientShellTab {
        tab_id: "tab_2".into(),
        workspace_id: "ws_2".into(),
        ..snapshot.tabs[0].clone()
    });
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(super::super::tests::surface());
    state.set_endpoint_methods(Some(vec!["pane.focus".into(), "tab.transfer".into()]));
    state.compose(106, 40).unwrap();
    state
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    point: (u16, u16),
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::NONE,
    })])
}
fn start(state: &mut ClientShellState) -> (u16, u16) {
    let rect = state.hits.agents[0].0;
    let point = (rect.x + 2, rect.y);
    let outcome = mouse(state, MouseEventKind::Down(MouseButton::Left), point);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(request.method, Method::PaneFocus(_)))
    );
    point
}

#[test]
fn agent_drag_transfers_whole_tab_including_shell_to_folded_heading_or_space() {
    for heading in [false, true] {
        let mut state = setup();
        state
            .config
            .collapsed_agent_groups
            .entry(ClientEndpointId::Local)
            .or_default()
            .insert("ws_2".into());
        state.compose(106, 40).unwrap();
        start(&mut state);
        let target = if heading {
            state
                .hits
                .agent_group_toggles
                .iter()
                .find(|(_, _, id)| id == "ws_2")
                .unwrap()
                .0
        } else {
            state
                .hits
                .workspaces
                .iter()
                .find(|hit| hit.workspace_id == "ws_2")
                .unwrap()
                .rect
        };
        let target = (target.x + 2, target.y);
        assert!(
            mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), target)
                .actions
                .is_empty()
        );
        let frame = state.compose(106, 40).unwrap();
        assert!(frame.cells.chunks(frame.width as usize).any(|row| row
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>()
            .contains("move → destination")));
        let outcome = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), target);
        assert!(outcome.requests.is_empty());
        assert!(
            matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }] if matches!(&request.method, Method::TabTransfer(params) if params.workspace_id=="ws_2" && params.tab_id=="tab_1" && params.expected_pane_ids==["pane_1","pane_2","pane_3"]))
        );
        assert!(state.agent_drag.is_none());
        assert!(state.config.collapsed_agent_groups[&ClientEndpointId::Local].contains("ws_2"));
    }
}

#[test]
fn agent_drag_cancels_on_invalid_drop_escape_focus_loss_and_changed_members() {
    for cancellation in 0..9 {
        let mut state = setup();
        let source = start(&mut state);
        let target = state.hits.workspaces[1].rect;
        let target = (target.x + 2, target.y);
        mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), target);
        let release_at = match cancellation {
            0 => source,
            1 => (100, 35),
            2 => {
                let input = state.handle_raw_events(vec![RawInputEvent::Key(
                    crate::input::TerminalKey::new(KeyCode::Esc, KeyModifiers::NONE),
                )]);
                assert!(input.actions.is_empty() && input.requests.is_empty());
                target
            }
            3 => {
                state.handle_raw_events(vec![RawInputEvent::OuterFocusLost]);
                target
            }
            4 => {
                let mut snapshot = state.snapshot.clone().unwrap();
                snapshot.panes.remove(2);
                state.set_snapshot(snapshot);
                target
            }
            5 => {
                state.mark_endpoint_disconnected(&ClientEndpointId::Local);
                target
            }
            6 => {
                state.mark_endpoint_disconnected(&ClientEndpointId::Local);
                state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Online);
                target
            }
            7 => {
                let mut snapshot = state.snapshot.clone().unwrap();
                snapshot.boot_id = "new-server".into();
                state.set_snapshot(snapshot);
                target
            }
            _ => {
                state.reset_endpoint_projection();
                target
            }
        };
        let input = mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            release_at,
        );
        assert!(!input.actions.iter().any(|action| matches!(action, ClientShellAction::Endpoint { request, .. } if matches!(request.method, Method::TabTransfer(_)))));
        assert!(state.agent_drag.is_none());
    }
}

#[test]
fn agent_drag_unfolds_only_after_success_and_reports_stale_rejection() {
    use crate::api::schema::{AgentStatus, ResponseResult, TabInfo};
    for success in [false, true] {
        let mut state = setup();
        state
            .config
            .collapsed_agent_groups
            .entry(ClientEndpointId::Local)
            .or_default()
            .insert("ws_2".into());
        start(&mut state);
        let target = state.hits.workspaces[1].rect;
        let point = (target.x + 2, target.y);
        mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), point);
        let input = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
        let [ClientShellAction::Endpoint { request, .. }] = input.actions.as_slice() else {
            panic!("transfer action");
        };
        let result = if success {
            Ok(ResponseResult::TabInfo {
                tab: TabInfo {
                    tab_id: "new-tab".into(),
                    workspace_id: "ws_2".into(),
                    number: 2,
                    label: "split".into(),
                    focused: true,
                    pane_count: 3,
                    agent_status: AgentStatus::Idle,
                },
            })
        } else {
            Err(ClientShellEndpointError {
                code: Some("stale_target".into()),
                message: "Group changed; drag again".into(),
            })
        };
        state.handle_endpoint_result("boot-1", &request.id, result);
        assert_eq!(
            state.config.collapsed_agent_groups[&ClientEndpointId::Local].contains("ws_2"),
            !success
        );
        assert_eq!(
            state.pending_transfer_reveal.as_deref(),
            success.then_some("new-tab")
        );
        if !success {
            assert!(state.visible_endpoint_notice.is_some());
        }
    }
}

#[test]
fn agent_drag_does_not_drop_on_another_endpoint_with_the_same_workspace_id() {
    let mut state = setup();
    start(&mut state);
    let hit = state
        .hits
        .workspaces
        .iter_mut()
        .find(|hit| hit.workspace_id == "ws_2")
        .unwrap();
    hit.endpoint_id = ClientEndpointId::Ssh(
        crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
    );
    let point = (hit.rect.x + 2, hit.rect.y);
    mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), point);
    let result = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
    assert!(result.actions.is_empty());
}

#[test]
fn agent_drag_requires_advertised_method_and_preserves_clicks() {
    for methods in [None, Some(vec!["pane.focus".into()])] {
        let mut state = setup();
        state.set_endpoint_methods(methods);
        let point = start(&mut state);
        assert!(state.agent_drag.is_none());
        mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
    }
    let mut state = setup();
    let point = start(&mut state);
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        (point.0 + 1, point.1),
    );
    assert!(!state.agent_drag.as_ref().unwrap().dragging);
    assert!(
        mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point)
            .actions
            .is_empty()
    );
}
