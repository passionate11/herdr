use super::*;

fn projected_agents() -> ClientShellSnapshot {
    let mut snapshot = super::super::super::tests::snapshot();
    snapshot.agents = (0..8)
        .map(|index| crate::protocol::ClientShellAgent {
            pane_id: format!("pane_{index}"),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
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
    snapshot
}

fn click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: crossterm::event::KeyModifiers::empty(),
        },
    )])
}

#[test]
fn custom_agent_group_click_round_trip_preserves_session_and_preferences() {
    let path = std::env::temp_dir().join(format!("herdr-agent-fold-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut settings = config().with_preferences_path(path.clone());
    settings.agents.group_tree = true;
    let mut state = ClientShellState::new(settings);
    state.set_snapshot(Box::new(projected_agents()));
    state.compose(100, 28).unwrap();
    state.agent_scroll = usize::MAX;
    state.compose(100, 28).unwrap();
    assert!(state.agent_scroll > 0);
    let snapshot = state.snapshot.clone();
    let heading = state.hits.agent_group_toggles[0].0;
    let outcome = click(&mut state, heading);
    assert!(outcome.repaint && !outcome.resize);
    assert!(outcome.requests.is_empty() && outcome.actions.is_empty());
    let frame = state.compose(100, 28).unwrap();
    assert!(frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| row
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>())
        .any(|row| row.contains("1 ▸ client-shell")));
    assert!(state.hits.agents.is_empty() && state.hits.endpoint_agents.is_empty());
    assert_eq!(state.hits.agent_group_toggles.len(), 1);
    assert_eq!(state.agent_scroll, 0);
    assert_eq!(state.hits.agent_max_scroll, 0);
    assert_eq!(state.snapshot.as_deref(), snapshot.as_deref());
    assert!(state.collapsed_groups.is_empty());

    let settings = config().with_preferences_path(path.clone());
    let mut restored = ClientShellState::new(settings);
    restored.config.agents.group_tree = true;
    restored.set_snapshot(Box::new(projected_agents()));
    restored.compose(100, 28).unwrap();
    assert!(restored.hits.agents.is_empty() && restored.hits.endpoint_agents.is_empty());
    let heading = restored.hits.agent_group_toggles[0].0;
    click(&mut restored, heading);
    restored.compose(100, 28).unwrap();
    assert!(!restored.hits.agents.is_empty());
    let agent = restored.hits.agents[0].0;
    let focused = click(&mut restored, agent);
    assert!(
        matches!(&focused.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(request.method, crate::api::schema::Method::PaneFocus(_)))
    );
    let reloaded = ClientShellState::new(config().with_preferences_path(path.clone()));
    assert!(reloaded
        .config
        .collapsed_agent_groups
        .values()
        .all(HashSet::is_empty));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn custom_agent_group_scroll_and_reveal_keep_collapsed_headings_reachable() {
    let mut rows = vec![
        row("one", "a", "a"),
        row("one", "a", "b"),
        row("two", "a", "c"),
        row("two", "a", "d"),
        row("three", "a", "e"),
        row("three", "a", "f"),
    ];
    for mask in 0..8 {
        for (index, row) in rows.iter_mut().enumerate() {
            row.group_collapsed = mask & (1 << (index / 2)) != 0;
        }
        let mut config = config();
        config.agents.group_tree = true;
        for width in [4, 5, 7, 8, 12, 30] {
            for height in 1..=20 {
                let body = Rect::new(2, 3, width, height);
                let geometry = AgentListGeometry::new(&rows, |row| row, &config, body, true);
                let max_scroll = geometry.metrics(usize::MAX).max_offset_from_bottom;
                for scroll in 0..=max_scroll {
                    let layout = geometry.layout(scroll);
                    assert!(geometry.metrics(scroll).viewport_rows > 0);
                    for (index, rect) in &layout.entries {
                        assert!(!geometry.tree || !rows[*index].group_collapsed);
                        assert!(rect.y >= body.y && rect.bottom() <= body.bottom());
                    }
                    if geometry.tree {
                        for group in &layout.boxes {
                            assert!(group.rect.y >= body.y && group.rect.bottom() <= body.bottom());
                        }
                    }
                }
                for target in 0..rows.len() {
                    let start = geometry.reveal(usize::MAX, target);
                    let layout = geometry.layout(start);
                    if geometry.tree && rows[target].group_collapsed {
                        assert!(layout.boxes.iter().any(|group| !group.split
                            && rows[group.index].workspace_id == rows[target].workspace_id));
                    } else {
                        assert!(layout.entries.iter().any(|(index, _)| *index == target));
                    }
                }
            }
        }
    }
}

#[test]
fn custom_agent_group_collapse_is_endpoint_local_and_ignored_in_other_views() {
    let mut config = config();
    config.agents.group_tree = true;
    let mut state = ClientShellState::new(config);
    let profile = SavedSshEndpoint::new("remote", "test@example.invalid", "test").unwrap();
    let remote_id = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    let snapshot = projected_agents();
    state.set_snapshot(Box::new(snapshot.clone()));
    state.set_endpoint_snapshot(&remote_id, Box::new(snapshot));
    state
        .config
        .collapsed_agent_groups
        .entry(ClientEndpointId::Local)
        .or_default()
        .insert("ws_1".into());
    state.compose(100, 45).unwrap();
    assert_eq!(state.hits.agent_group_toggles.len(), 2);
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .all(|(_, endpoint, _)| endpoint == &remote_id));
    assert!(!state.hits.endpoint_agents.is_empty());
    state.reveal_endpoint_agent(
        &ClientEndpointId::Local,
        "pane_7",
        state.hits.agent_body.height,
    );
    state.compose(100, 45).unwrap();
    assert!(state
        .hits
        .agent_group_toggles
        .iter()
        .any(|(_, endpoint, _)| endpoint.is_local()));
    assert!(state.config.collapsed_agent_groups[&ClientEndpointId::Local].contains("ws_1"));
    state.config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    state.compose(100, 45).unwrap();
    assert!(state.hits.agent_group_toggles.is_empty());
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, endpoint, _)| endpoint.is_local()));
    state.config.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    state.config.mouse_capture = false;
    state.compose(100, 45).unwrap();
    assert!(state.hits.agent_group_toggles.is_empty());
}
