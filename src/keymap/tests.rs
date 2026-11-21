use std::collections::HashSet;

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn resolve(app: &App, event: KeyEvent) -> Option<Action> {
    lookup(&active_contexts(app), event, app).map(|binding| binding.action)
}

fn app_in(pane: Pane) -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.nav.pane = pane;
    app
}

#[test]
fn no_duplicate_key_inside_a_context() {
    for context in ALL_CONTEXTS {
        let mut seen = HashSet::new();
        for binding in bindings(context) {
            let id = format!("{:?}{:?}{:?}", binding.prefix, binding.key, binding.mods);
            assert!(seen.insert(id), "{context:?} binds {} twice", binding.key_name());
        }
    }
}

#[test]
fn a_plain_row_never_shadows_a_chord_row_below_it() {
    for context in ALL_CONTEXTS.into_iter().filter(|context| !context.strict()) {
        let rows: Vec<_> = bindings(context).collect();
        for (at, early) in rows.iter().enumerate() {
            for late in &rows[at + 1..] {
                let shadowed = early.key == late.key && early.prefix == late.prefix && early.mods == Mods::Any;
                assert!(!shadowed, "{context:?}: {} hides {}", early.key_name(), late.key_name());
            }
        }
    }
}

/// Esc and `?` close or open something in every modal context. Text
/// layers type `?`, so they have no help key.
#[test]
fn modal_contexts_have_esc_and_help_except_where_documented() {
    const NO_HELP_KEY: [Context; 6] = [
        Context::Dialog(Dialog::Compose),
        Context::Dialog(Dialog::Finder),
        Context::Input(TextInput::Bar),
        Context::Input(TextInput::Query),
        Context::Input(TextInput::Name),
        Context::Input(TextInput::BoardQuery),
    ];
    let has = |context: Context, code: KeyCode| bindings(context).any(|b| b.key == code);
    let modal = ALL_CONTEXTS.into_iter()
        .filter(|context| matches!(context, Context::Global | Context::Dialog(_) | Context::Input(_)));
    for context in modal {
        assert!(has(context, KeyCode::Esc), "{context:?} has no Esc");
        let help = has(context, KeyCode::Char('?'));
        assert_eq!(!help, NO_HELP_KEY.contains(&context), "{context:?} `?` exception list is stale");
    }
}

#[test]
fn text_contexts_leave_printable_keys_to_typing() {
    for context in ALL_CONTEXTS.into_iter().filter(|context| context.types_text()) {
        let claimed: Vec<String> = bindings(context)
            .filter(|b| matches!(b.key, KeyCode::Char(_)) && b.mods == Mods::Plain)
            .map(Binding::key_name)
            .collect();
        let allowed: &[&str] = match context {
            // Choice letters are dynamic; board nav keys win over typing.
            Context::Dialog(Dialog::BoardFilter) => &["j", "l", "k", "h", "q", "?"],
            // `?` opens help; the typed-through choice letters are dynamic.
            Context::Dialog(Dialog::Confirm) => &["?"],
            _ => &[],
        };
        assert!(
            claimed.iter().all(|name| allowed.contains(&name.as_str()) || name == "space"),
            "{context:?} claims printable keys: {claimed:?}",
        );
    }
}

#[test]
fn every_action_has_a_binding_and_every_binding_a_mouse_answer() {
    let bound: HashSet<Action> = ALL_CONTEXTS.into_iter()
        .flat_map(|context| bindings(context).map(|b| b.action)).collect();
    for action in Action::ALL {
        assert!(bound.contains(action), "{action:?} is bound nowhere");
    }
    for context in ALL_CONTEXTS {
        for binding in bindings(context) {
            if let Gesture::NoMouse(reason) = binding.mouse {
                assert!(!reason.is_empty(), "{:?} needs a NoMouse reason", binding.action);
            }
            assert!(!binding.label.is_empty() && !binding.group.is_empty(), "{:?}", binding.action);
        }
    }
}

/// Actions with no mouse gesture today. This is the work queue for mouse
/// parity: run with `--nocapture` to print it.
fn mouse_gaps() -> Vec<(Action, &'static str)> {
    let mut gaps: Vec<(Action, &'static str)> = vec![];
    for action in Action::ALL {
        let all: Vec<Gesture> = ALL_CONTEXTS.into_iter()
            .flat_map(|context| bindings(context).filter(|b| b.action == *action).map(|b| b.mouse))
            .collect();
        if all.iter().all(|mouse| matches!(mouse, Gesture::NoMouse(_))) {
            let reason = all.iter().find_map(|mouse| match mouse {
                Gesture::NoMouse(reason) if *reason != "alias of a sibling key" => Some(*reason),
                _ => None,
            });
            gaps.push((*action, reason.unwrap_or("alias of a sibling key")));
        }
    }
    gaps
}

#[test]
fn mouse_gap_list_is_exposed_for_mouse_parity() {
    let gaps = mouse_gaps();
    println!("actions without a mouse gesture ({}):", gaps.len());
    for (action, reason) in &gaps {
        println!("  {action:?}: {reason}");
    }
    assert!(!gaps.iter().any(|(action, _)| *action == Action::Pin), "Pin has the right-click menu");
    assert!(!gaps.iter().any(|(action, _)| *action == Action::TabVisual));
}

/// The NoMouse list holds only four kinds of key: editing keys, aliases of
/// a key that has a gesture, scroll keys the wheel covers, and dismiss or
/// pick keys another click covers. A new gap must add a reason on purpose.
#[test]
fn no_mouse_reasons_stay_in_the_allowed_categories() {
    for context in ALL_CONTEXTS {
        for binding in bindings(context) {
            if let Gesture::NoMouse(reason) = binding.mouse {
                assert!(
                    ALLOWED_NO_MOUSE.contains(&reason),
                    "{context:?} {}: `{reason}` is not an allowed NoMouse reason",
                    binding.key_name(),
                );
            }
        }
    }
    for (action, reason) in mouse_gaps() {
        assert!(ALLOWED_NO_MOUSE.contains(&reason), "{action:?}: `{reason}`");
    }
}

/// Text editing keys are the only ones allowed to have no pointer control
/// inside the comment editor and the text inputs.
#[test]
fn editor_and_text_input_keys_are_the_only_gaps_in_their_contexts() {
    let editors = ALL_CONTEXTS.into_iter()
        .filter(|context| matches!(context, Context::Input(_) | Context::Dialog(Dialog::Compose)));
    for context in editors {
        for binding in bindings(context) {
            if let Gesture::NoMouse(reason) = binding.mouse {
                assert!(
                    ALLOWED_NO_MOUSE[..2].contains(&reason),
                    "{context:?} {}: {reason}", binding.key_name(),
                );
            }
        }
    }
}

/// A binding that promises "click hint" must carry a hint rank, or the
/// click target would never draw.
#[test]
fn hint_gestures_rest_on_a_hint_rank() {
    for context in ALL_CONTEXTS {
        for binding in bindings(context) {
            if binding.mouse == Gesture::Hint {
                assert!(binding.hint.is_some(), "{context:?} {} has no hint", binding.key_name());
            }
        }
    }
}

#[test]
fn every_menu_entry_has_a_mouse_gesture() {
    let gaps: Vec<Action> = mouse_gaps().into_iter().map(|(action, _)| action).collect();
    let app = App::new(Some("o/r".to_string()));
    for kind in [
        crate::app::MenuKind::Issue, crate::app::MenuKind::Ref, crate::app::MenuKind::History,
        crate::app::MenuKind::Labels, crate::app::MenuKind::Body, crate::app::MenuKind::Map, crate::app::MenuKind::Board,
        crate::app::MenuKind::Global,
    ] {
        let menu = crate::app::MenuState::new(kind, &app, (0, 0));
        for entry in menu.entries {
            assert!(!gaps.contains(&entry.action), "{kind:?}: {:?}", entry.action);
        }
    }
}

#[test]
fn global_keys_resolve_to_their_actions() {
    let app = app_in(Pane::Main);
    for (event, action) in [
        (key(KeyCode::Char('y')), Action::Yank),
        (key(KeyCode::Char('p')), Action::Pin),
        (key(KeyCode::Char('r')), Action::Refresh),
        (key(KeyCode::Char('w')), Action::MergedWindow),
        (key(KeyCode::Char('o')), Action::OpenBrowser),
        (ctrl('s'), Action::SubmitReview),
        (key(KeyCode::Char('F')), Action::QuerySearch),
        (key(KeyCode::Char('G')), Action::JumpBottom),
        (ctrl('d'), Action::HalfPageDown),
        (ctrl('u'), Action::HalfPageUp),
        (key(KeyCode::PageDown), Action::PageDown),
        (key(KeyCode::Char('[')), Action::StepPrev),
        (key(KeyCode::Char(']')), Action::StepNext),
        (key(KeyCode::Char('[')), Action::StepPrev),
        (key(KeyCode::Down), Action::MoveDown),
        (key(KeyCode::Char('q')), Action::Quit),
        (ctrl('c'), Action::Quit),
        (ctrl('o'), Action::HistoryBack),
        (ctrl('i'), Action::HistoryForward),
        (KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL), Action::HistoryForward),
        (key(KeyCode::Tab), Action::NextPane),
        (key(KeyCode::Char('?')), Action::ShowHelp),
        (key(KeyCode::Esc), Action::Back),
    ] {
        assert_eq!(resolve(&app, event), Some(action), "{event:?}");
    }
}

#[test]
fn pane_keys_resolve_only_in_their_pane() {
    for (pane, code, want) in [
        (Pane::Issues, 'x', Some(Action::Evict)),
        (Pane::PRs, 's', Some(Action::CycleSort)),
        (Pane::PRs, 'S', Some(Action::ReverseSort)),
        (Pane::PRs, 'a', None),
        (Pane::Main, 'x', None),
        (Pane::Refs, 's', None),
        (Pane::Labels, 'X', Some(Action::ClearLabels)),
        (Pane::Issues, 'X', None),
    ] {
        assert_eq!(resolve(&app_in(pane), key(KeyCode::Char(code))), want, "{pane:?} {code}");
    }
    assert_eq!(resolve(&app_in(Pane::Labels), key(KeyCode::Enter)), Some(Action::ToggleLabel));
    assert_eq!(resolve(&app_in(Pane::Refs), key(KeyCode::Enter)), Some(Action::OpenRef));
    assert_eq!(resolve(&app_in(Pane::History), key(KeyCode::Enter)), Some(Action::OpenHistoryRow));
    assert_eq!(resolve(&app_in(Pane::Issues), key(KeyCode::Enter)), Some(Action::OpenRow));
    assert_eq!(resolve(&app_in(Pane::PRs), key(KeyCode::Enter)), Some(Action::OpenRow));
    assert_eq!(resolve(&app_in(Pane::Main), key(KeyCode::Enter)), None);
}

#[test]
fn predicates_gate_their_bindings() {
    let mut app = app_in(Pane::Issues);
    assert_eq!(resolve(&app, key(KeyCode::Char('z'))), None, "tree keys need the tree");
    assert_eq!(resolve(&app, key(KeyCode::Char('u'))), None, "undo needs a live offer");
    app.sidebar.sub_issue_tree = true;
    assert_eq!(resolve(&app, key(KeyCode::Char('z'))), Some(Action::TreeToggle));
    assert_eq!(resolve(&app, key(KeyCode::Right)), Some(Action::TreeExpand));
    app.nav.pane = Pane::PRs;
    assert_eq!(resolve(&app, key(KeyCode::Right)), None, "tree keys are Issues only");
}

#[test]
fn layers_resolve_top_down_and_dialogs_are_modal() {
    let mut app = app_in(Pane::Issues);
    app.open(crate::app::Entry::Shortcuts);
    assert_eq!(resolve(&app, key(KeyCode::Char('j'))), Some(Action::HelpDown));
    assert_eq!(resolve(&app, key(KeyCode::Char('x'))), None, "pane keys do not reach through");
    assert_eq!(resolve(&app, key(KeyCode::Char('?'))), Some(Action::CloseHelp));
    app.close(crate::app::Slot::Shortcuts);
    app.nav.focus = Focus::Bar;
    assert_eq!(resolve(&app, key(KeyCode::Enter)), Some(Action::InputSubmit));
    assert_eq!(resolve(&app, ctrl('c')), Some(Action::Quit));
    assert_eq!(resolve(&app, key(KeyCode::Char('j'))), None, "the bar types j");
}

#[test]
fn compose_chords_need_exact_modifiers() {
    let mut app = app_in(Pane::Main);
    app.close(crate::app::Slot::Format);
    app.open_draft(crate::app::ComposeState {
        number: 7,
        target: crate::app::ComposeTarget::Thread,
        body: String::new(),
        cursor: 0,
        sending: false,
        kill: String::new(), view: Default::default(),
    });
    assert_eq!(resolve(&app, ctrl('S')), Some(Action::ComposeSend), "chord letters fold case");
    assert_eq!(resolve(&app, key(KeyCode::Char('s'))), None, "plain s types");
    let both = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert_eq!(resolve(&app, both), None, "ctrl-alt chords are swallowed");
    assert_eq!(
        resolve(&app, KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL)),
        Some(Action::ComposeTab),
        "Tab indents whatever the modifiers say",
    );
}

#[test]
fn help_merges_aliases_and_leads_with_the_current_context() {
    let app = app_in(Pane::Labels);
    let sections = help_sections(&app);
    assert_eq!(sections[0], (Context::Pane(Pane::Labels), true));
    assert_eq!(sections[1], (Context::Global, true));
    assert!(sections[2..].iter().all(|(_, current)| !current));
    assert!(sections.iter().all(|(context, _)| bindings(*context).next().is_some()));
    let rows = help_rows(Context::Dialog(Dialog::Board));
    let down = rows.iter().find(|row| row.label == "Next card").expect("next card row");
    assert_eq!(down.keys, "j / ↓");
    assert_eq!(down.mouse, Some("wheel"));
}

#[test]
fn help_from_a_diff_tab_lists_the_diff_context_before_global() {
    let mut app = app_in(Pane::Main);
    app.issues.data.insert(9, crate::model::IssueData {
        number: 9,
        title: String::new(),
        state: "OPEN".to_string(),
        author: crate::model::Author { login: "bo".to_string() },
        labels: vec![],
        assignees: vec![],
        created_at: String::new(),
        updated_at: String::new(),
        url: String::new(),
        body: String::new(),
        comments: vec![],
        review_threads: vec![],
        paired: vec![],
        checks: None,
        milestone: None,
        parent: None,
        sub_summary: None,
            links: Default::default(),
        is_pr: true,
    });
    app.nav.current = Some(9);
    app.nav.tab = crate::app::Tab::Visual;
    let order: Vec<Context> = help_sections(&app).into_iter()
        .filter(|(_, current)| *current).map(|(context, _)| context).collect();
    assert_eq!(order, vec![Context::Pane(Pane::Main), Context::Main(MainMode::Diff), Context::Global]);
}

#[test]
fn plain_bindings_do_not_fire_with_ctrl_or_alt() {
    let app = app_in(Pane::Issues);
    assert_eq!(resolve(&app, ctrl('b')), None, "ctrl-b is not the board key");
    assert_eq!(resolve(&app, ctrl('x')), None);
    let alt = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT);
    assert_eq!(resolve(&app, alt), None);
    assert_eq!(resolve(&app, key(KeyCode::Char('b'))), Some(Action::OpenBoard));
    let shift = KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT);
    assert_eq!(resolve(&app, shift), Some(Action::ReverseSort), "shift still counts as the capital");
    let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
    assert_eq!(resolve(&app, ctrl_enter), None);
}

#[test]
fn a_prefix_waits_for_its_second_key() {
    let app = app_in(Pane::Issues);
    let chain = active_contexts(&app);
    let g = key(KeyCode::Char('g'));
    assert!(lookup(&chain, g, &app).is_none(), "g alone binds nothing outside dialogs");
    assert_eq!(sequence_start(&chain, g, &app), Some('g'));
    assert_eq!(lookup_after(&chain, 'g', g, &app).map(|b| b.action), Some(Action::JumpTop));
    assert!(lookup_after(&chain, 'g', key(KeyCode::Char('j')), &app).is_none(), "any other key cancels");
    assert_eq!(sequence_start(&chain, key(KeyCode::Char('j')), &app), None);
    assert_eq!(sequence_start(&chain, ctrl('g'), &app), None, "a chord never starts a sequence");
    assert!(lookup_after(&chain, 'x', g, &app).is_none(), "the prefix must match");
}

#[test]
fn dialog_keys_beat_the_jump_prefix() {
    let mut app = app_in(Pane::Issues);
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let chain = active_contexts(&app);
    let g = key(KeyCode::Char('g'));
    assert_eq!(lookup(&chain, g, &app).map(|b| b.action), Some(Action::CycleGroup));
    assert_eq!(sequence_start(&chain, g, &app), None);
}

#[test]
fn help_opens_from_every_non_text_dialog() {
    let mut app = app_in(Pane::Issues);
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    assert_eq!(resolve(&app, key(KeyCode::Char('?'))), Some(Action::ShowHelp));
    assert_eq!(resolve(&app, key(KeyCode::Char('q'))), Some(Action::CloseFormat));
}

#[test]
fn close_item_has_key_mouse_and_hint() {
    let binding = describe(Action::CloseItem).expect("bound");
    assert_eq!(binding.key, KeyCode::Char('C'));
    assert_eq!(binding.mouse, Gesture::Title);
    assert!(binding.hint.is_some());
}

#[test]
fn diff_expand_all_has_key_toolbar_button_and_hint() {
    let b = describe(Action::DiffExpandAll).expect("bound");
    assert_eq!(b.key_name(), "a");
    assert!(matches!(b.mouse, Gesture::Title), "toolbar button is its mouse path");
    assert!(b.hint.is_some(), "bar hint ranked");
}

#[test]
fn finder_has_key_mouse_and_hint() {
    let open = describe(Action::OpenFinder).expect("bound");
    assert_eq!((open.key, open.mods, open.mouse), (KeyCode::Char('p'), Mods::Ctrl, Gesture::Menu));
    let app = app_in(Pane::Main);
    assert_eq!(resolve(&app, ctrl('p')), Some(Action::OpenFinder));
    let menu = crate::app::MenuState::new(crate::app::MenuKind::Global, &app, (0, 0));
    assert!(menu.entries.iter().any(|e| e.action == Action::OpenFinder), "the global menu lists it");
    let finder = Context::Dialog(Dialog::Finder);
    let hinted: Vec<Action> = bindings(finder).filter(|b| b.hint.is_some()).map(|b| b.action).collect();
    assert_eq!(hinted, vec![Action::Back, Action::FinderRun, Action::FinderUp, Action::FinderDown]);
    assert!(bindings(finder).any(|b| b.action == Action::FinderRun && b.mouse == Gesture::Click));
    assert!(bindings(finder).any(|b| b.action == Action::FinderDown && b.mouse == Gesture::Wheel));
    assert!(bindings(finder).any(|b| b.action == Action::Back && b.mouse == Gesture::Outside));
}

/// The map keeps every global and Main key. Only the keys the old graph
/// already took (h j k l and the arrows) pan the map instead. Esc keeps
/// its global meaning: it closes the nearest layer, here an open card.
#[test]
fn map_keys_never_shadow_global_or_main_keys() {
    let kept = [ch_code('h'), ch_code('j'), ch_code('k'), ch_code('l'), KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down];
    let below: Vec<&Binding> = bindings(Context::Global).chain(bindings(Context::Pane(Pane::Main))).collect();
    for map in bindings(Context::Main(MainMode::Graph)) {
        if kept.contains(&map.key) || (map.key == KeyCode::Esc && map.when.is_some()) {
            continue;
        }
        let clash = below.iter().find(|b| b.key == map.key && b.prefix == map.prefix && b.mods == map.mods);
        assert!(clash.is_none(), "map key {} shadows {:?}", map.key_name(), clash.map(|b| b.action));
    }
}

fn ch_code(c: char) -> KeyCode {
    KeyCode::Char(c)
}
