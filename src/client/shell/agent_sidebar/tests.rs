use super::*;
use crate::ui::{ResolvedToken, ResolvedTokenKind};

fn row(workspace: &str, tab: &str, agent: &str) -> AgentRow {
    AgentRow {
        pane_id: agent.into(),
        endpoint_id: None,
        workspace_id: workspace.into(),
        workspace_label: workspace.into(),
        workspace_number: 1,
        tab_id: tab.into(),
        workspace_focused: false,
        status: crate::api::schema::AgentStatus::Idle,
        focused: false,
        rows: vec![vec![ResolvedToken {
            kind: ResolvedTokenKind::Agent(agent.into()),
            style: Default::default(),
        }]],
    }
}

fn config() -> ClientShellConfig {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.agents.group_boxes = true;
    config.agents.cards = true;
    config.agents.row_gap = 1;
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
    config
}

#[test]
fn custom_grouped_agents_render_space_and_untitled_split_boxes() {
    let rows = vec![
        row("one", "a", "codex"),
        row("one", "b", "pi"),
        row("one", "b", "claude"),
        row("one", "c", "codex"),
    ];
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 13));
    let mut hits = ShellHitMap::default();
    let mut scroll = 0;
    render_agent_list(
        &mut buffer,
        Rect::new(0, 0, 20, 13),
        &rows,
        None,
        &config(),
        &mut scroll,
        &mut hits,
        |row| row,
        |buffer, rect, row, boxed, cards, hits| {
            register_agent_hit(hits, row, rect);
            render_agent_row(buffer, rect, row, &config(), boxed, cards);
        },
    );
    let text = (3..12)
        .map(|y| {
            (0..20)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        text,
        [
            "╭─ one ───────────╮",
            "│  codex          │",
            "│╭───────────────╮│",
            "││ pi            ││",
            "││▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀││",
            "││ claude        ││",
            "│╰───────────────╯│",
            "│  codex          │",
            "╰─────────────────╯",
        ]
    );
    assert_eq!(hits.agents.len(), 5);
    for border in [3, 5, 9, 11] {
        assert!(!hits
            .agents
            .iter()
            .any(|(rect, _)| rect.y <= border && border < rect.bottom()));
    }
    assert_eq!(
        hits.agents.iter().find(|(rect, _)| rect.y == 7).unwrap().1,
        "pi"
    );
}

#[test]
fn custom_tree_renders_untitled_split_branches_without_indenting_cards() {
    let mut rows = vec![
        row("one", "a", "codex"),
        row("one", "b", "pi"),
        row("one", "b", "claude"),
        row("one", "c", "codex"),
    ];
    rows[3].focused = true;
    let mut config = config();
    config.agents.group_tree = true;
    config.agents.group_boxes = false;
    let area = Rect::new(0, 0, 20, 15);
    let mut buffer = Buffer::empty(area);
    let mut hits = ShellHitMap::default();
    let mut scroll = 0;
    render_agent_list(
        &mut buffer,
        area,
        &rows,
        None,
        &config,
        &mut scroll,
        &mut hits,
        |row| row,
        |buffer, rect, row, grouped, cards, hits| {
            register_agent_hit(hits, row, rect);
            render_agent_row(buffer, rect, row, &config, grouped, cards);
        },
    );
    let text = (3..11)
        .map(|y| {
            (0..20)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        text,
        [
            "1 ▾ one",
            " ├─  codex",
            " │  ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀",
            " │ ╭ pi",
            " ├─┤▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀",
            " │ ╰ claude",
            " │  ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀",
            " └─ ▏codex",
        ]
    );
    assert_eq!(buffer[(5, 6)].bg, agent_card_bg(&config, false));
    assert_eq!(buffer[(5, 10)].bg, agent_card_bg(&config, true));
    assert!(hits.agents.iter().all(|(rect, _)| rect.x == 4));
    assert!(!hits.agents.iter().any(|(rect, _)| rect.y == 3));
    assert_eq!(
        hits.agents.iter().find(|(rect, _)| rect.y == 7).unwrap().1,
        "pi"
    );
}

#[test]
fn custom_tree_scroll_preserves_visible_groups_and_reveals_every_agent() {
    let mut rows = vec![
        row("one", "a", "a"),
        row("one", "a", "b"),
        row("one", "a", "c"),
        row("one", "b", "d"),
        row("two", "a", "e"),
        row("two", "a", "f"),
    ];
    rows[4].endpoint_id = Some(ClientEndpointId::Local);
    rows[5].endpoint_id = Some(ClientEndpointId::Local);
    for (gap, lines) in [(0, 1), (1, 1), (3, 2)] {
        let mut config = config();
        config.agents.group_tree = true;
        config.agents.row_gap = gap;
        for row in &mut rows {
            row.rows.resize(lines, row.rows[0].clone());
        }
        for width in [4, 5, 7, 8, 9, 10, 20] {
            for height in 1..=22 {
                let body = Rect::new(2, 3, width, height);
                let geometry = AgentListGeometry::new(&rows, |row| row, &config, body, true);
                for scroll in 0..rows.len() {
                    let layout = geometry.layout(scroll);
                    assert!(!layout.entries.is_empty(), "{body:?} scroll={scroll}");
                    if geometry.tree {
                        for (_, rect) in layout.entries.iter().chain(&layout.tails) {
                            assert_eq!(rect.x, body.x + 4);
                            assert!(rect.y > body.y && rect.bottom() <= body.bottom());
                            assert!(rect.right() < body.right());
                        }
                        for group in &layout.boxes {
                            assert!(group.rect.y >= body.y);
                            assert!(group.rect.bottom() <= body.bottom());
                        }
                        for link in &layout.links {
                            assert!(link.rect.y > body.y && link.rect.bottom() <= body.bottom());
                            assert!(link.rect.right() <= body.right());
                        }
                    }
                }
                for target in 0..rows.len() {
                    let start = geometry.reveal(0, target);
                    assert!(geometry
                        .layout(start)
                        .entries
                        .iter()
                        .any(|(index, _)| *index == target));
                }
                let last_start = geometry.metrics(usize::MAX).max_offset_from_bottom;
                assert_eq!(
                    geometry.layout(last_start).entries.last().unwrap().0,
                    rows.len() - 1
                );
            }
        }
    }
}

#[test]
fn custom_grouped_agents_keep_scrolled_boxes_closed_and_reveal_targets() {
    let mut rows = vec![
        row("one", "a", "a"),
        row("one", "a", "b"),
        row("one", "a", "c"),
        row("one", "b", "d"),
        row("two", "a", "e"),
        row("two", "a", "f"),
    ];
    for (gap, lines, cards) in [
        (0, 1, false),
        (0, 1, true),
        (1, 1, false),
        (1, 1, true),
        (3, 2, false),
        (3, 2, true),
    ] {
        let mut config = config();
        config.agents.row_gap = gap;
        config.agents.cards = cards;
        for row in &mut rows {
            row.rows.resize(lines, row.rows[0].clone());
        }
        for width in [4, 5, 9, 10, 12, 13, 20] {
            for height in 1..=22 {
                let body = Rect::new(2, 3, width, height);
                let geometry = AgentListGeometry::new(&rows, |row| row, &config, body, true);
                for scroll in 0..rows.len() {
                    let layout = geometry.layout(scroll);
                    assert!(!layout.entries.is_empty(), "{body:?} scroll={scroll}");
                    let first = layout.entries[0].1;
                    assert!(layout
                        .entries
                        .iter()
                        .all(|(_, rect)| rect.x == first.x && rect.width == first.width));
                    for group in &layout.boxes {
                        assert!(group.rect.y >= body.y && group.rect.bottom() <= body.bottom());
                        assert!(group.rect.x >= body.x && group.rect.right() <= body.right());
                        for (_, rect) in layout.entries.iter().chain(&layout.tails) {
                            assert!(
                                rect.bottom() <= group.rect.y
                                    || rect.y >= group.rect.bottom()
                                    || (rect.y > group.rect.y
                                        && rect.bottom() < group.rect.bottom())
                            );
                        }
                    }
                }
                for target in 0..rows.len() {
                    let start = geometry.reveal(0, target);
                    assert!(geometry
                        .layout(start)
                        .entries
                        .iter()
                        .any(|(index, _)| *index == target));
                }
                let last_start = geometry.metrics(usize::MAX).max_offset_from_bottom;
                assert_eq!(
                    geometry.layout(last_start).entries.last().unwrap().0,
                    rows.len() - 1
                );
            }
        }
    }
}

#[test]
fn custom_grouped_agents_qualify_groups_by_endpoint_and_keep_other_sort_orders() {
    let mut rows = vec![row("one", "a", "a"), row("one", "a", "b")];
    rows[1].endpoint_id = Some(ClientEndpointId::Local);
    let config = config();
    let body = Rect::new(0, 0, 20, 20);
    let layout = AgentListGeometry::new(&rows, |row| row, &config, body, true).layout(0);
    assert_eq!(layout.boxes.len(), 2);
    assert!(!layout.boxes.iter().any(|group| group.split));
    for view in [false, true] {
        let mut config = ClientShellConfig::from_config(&Config::default());
        config.agents.group_boxes = true;
        config.agents.group_tree = true;
        if view {
            config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
        }
        let geometry = AgentListGeometry::new(&rows, |row| row, &config, body, view);
        assert!(!geometry.boxed);
        assert_eq!(
            geometry
                .layout(0)
                .entries
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }
}

#[test]
fn custom_grouped_cards_follow_host_colors_and_drop_only_redundant_workspace_tokens() {
    let mut config = config();
    config.agent_card_background = Some(crate::terminal_theme::RgbColor { r: 0, g: 0, b: 0 });
    config.agent_card_foreground = Some(crate::terminal_theme::RgbColor {
        r: 200,
        g: 200,
        b: 200,
    });
    assert_eq!(
        agent_card_bg(&config, false),
        ratatui::style::Color::Rgb(7, 7, 7)
    );
    assert_eq!(
        agent_card_bg(&config, true),
        ratatui::style::Color::Rgb(18, 18, 18)
    );
    let mut row = row("one", "a", "pi");
    row.rows[0].insert(
        0,
        ResolvedToken {
            kind: ResolvedTokenKind::Workspace("one".into()),
            style: Default::default(),
        },
    );
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 2));
    render_agent_row(
        &mut buffer,
        Rect::new(0, 0, 20, 1),
        &row,
        &config,
        true,
        true,
    );
    let text = (0..20).map(|x| buffer[(x, 0)].symbol()).collect::<String>();
    assert!(text.contains("pi") && !text.contains("one"));
    row.rows[0].truncate(1);
    render_agent_row(
        &mut buffer,
        Rect::new(0, 1, 20, 1),
        &row,
        &config,
        true,
        true,
    );
    assert!((0..20)
        .map(|x| buffer[(x, 1)].symbol())
        .collect::<String>()
        .contains("one"));
}

#[test]
fn custom_grouped_endpoint_reveal_and_tail_hits_share_rendered_geometry() {
    let mut state = ClientShellState::new(config());
    state.set_endpoint_catalog(&[crate::client::endpoint::SavedSshEndpoint {
        id: crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "remote".into(),
        target: "test@example.invalid".into(),
        session: "test".into(),
        enabled: true,
    }]);
    let mut snapshot = super::super::tests::snapshot();
    snapshot.agents = (0..6)
        .map(|index| crate::protocol::ClientShellAgent {
            pane_id: format!("pane_{index}"),
            workspace_id: "ws_1".into(),
            tab_id: if index < 3 { "tab_1" } else { "tab_2" }.into(),
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
        number: 2,
        focused: false,
        ..snapshot.tabs[0].clone()
    });
    snapshot.panes = snapshot
        .agents
        .iter()
        .map(|agent| crate::protocol::ClientShellPane {
            pane_id: agent.pane_id.clone(),
            tab_id: agent.tab_id.clone(),
            focused: agent.focused,
            ..snapshot.panes[0].clone()
        })
        .collect();
    state.set_snapshot(Box::new(snapshot));
    state.compose(100, 28).unwrap();
    let height = state.hits.agent_body.height;
    state.reveal_endpoint_agent(&ClientEndpointId::Local, "pane_5", height);
    state.compose(100, 28).unwrap();
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(_, endpoint, pane)| *endpoint == ClientEndpointId::Local && pane == "pane_5"));
    state.reveal_endpoint_agent(&ClientEndpointId::Local, "pane_0", height);
    state.compose(100, 28).unwrap();
    let first = state
        .hits
        .endpoint_agents
        .iter()
        .find(|(_, _, pane)| pane == "pane_0")
        .unwrap()
        .0;
    assert!(state
        .hits
        .endpoint_agents
        .iter()
        .any(|(rect, _, pane)| pane == "pane_0" && rect.y == first.y + first.height));
    for border in [first.y - 2, first.y - 1] {
        assert!(!state
            .hits
            .endpoint_agents
            .iter()
            .any(|(rect, _, _)| rect.y <= border && border < rect.bottom()));
    }
}

#[test]
#[ignore = "manual fixed-geometry profile; run with --ignored --nocapture"]
fn custom_grouped_layout_profile() {
    let body = Rect::new(0, 0, 30, 40);
    let iterations = 10_000;
    for count in [1, 15, 150] {
        let rows = (0..count)
            .map(|index| {
                row(
                    "space",
                    &format!("tab_{}", index / 3),
                    &format!("agent_{index}"),
                )
            })
            .collect::<Vec<_>>();
        for (grouped, tree) in [(false, false), (true, false), (true, true)] {
            let mut config = config();
            config.agents.group_boxes = grouped;
            config.agents.group_tree = tree;
            let started = std::time::Instant::now();
            for _ in 0..iterations {
                let geometry = AgentListGeometry::new(
                    std::hint::black_box(&rows),
                    |row| row,
                    &config,
                    body,
                    true,
                );
                let metrics = geometry.metrics(0);
                std::hint::black_box(geometry.layout(metrics.max_offset_from_bottom));
            }
            eprintln!(
                "custom_grouped_layout_profile rows={count} grouped={grouped} tree={tree} geometry_ns={}",
                started.elapsed().as_nanos() / iterations
            );

            let area = Rect::new(0, 0, 30, 43);
            let mut buffer = Buffer::empty(area);
            let mut hits = ShellHitMap::default();
            let mut scroll = 0;
            let started = std::time::Instant::now();
            for _ in 0..iterations {
                hits.agents.clear();
                render_agent_list(
                    &mut buffer,
                    area,
                    &rows,
                    None,
                    &config,
                    &mut scroll,
                    &mut hits,
                    |row| row,
                    |buffer, rect, row, boxed, cards, hits| {
                        register_agent_hit(hits, row, rect);
                        render_agent_row(buffer, rect, row, &config, boxed, cards);
                    },
                );
                std::hint::black_box(&buffer);
            }
            eprintln!(
                "custom_grouped_layout_profile rows={count} grouped={grouped} tree={tree} render_ns={}",
                started.elapsed().as_nanos() / iterations
            );
        }
    }
}
