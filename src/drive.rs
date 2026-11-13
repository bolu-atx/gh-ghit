//! Drive mode: headless UI over a Unix socket for CI.
//!
//! `ghit --drive /tmp/x.sock` boots without a TTY on a cell-buffer
//! backend at a fixed geometry and serves versioned JSON-lines frames
//! to one local client. The socket is the event source in place of
//! crossterm `event::poll`/`read`: key and mouse messages land in the
//! same `input::handle_key`/`handle_mouse` calls as the terminal loop,
//! and every `poll_*` lander in `app` runs unchanged each pass. Frame
//! capture reads the post-draw `TestBackend` buffer the way the
//! `render_smoke` tests read it today.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Wire protocol version. The server sends it in the first `hello`
/// frame; clients must check it before driving.
pub const PROTOCOL_VERSION: u32 = 1;

/// Client to server: one JSON object per line.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Key press: `key` is one printable char (`"j"`, `" "`, `":"`) or
    /// a name (`"Enter"`, `"Esc"`, `"Tab"`, `"Backspace"`, `"Delete"`,
    /// `"Up"`, `"Down"`, `"Left"`, `"Right"`, `"PageUp"`, `"PageDown"`,
    /// `"Home"`, `"End"`), plus modifier flags.
    Key { key: String, #[serde(default)] ctrl: bool, #[serde(default)] alt: bool, #[serde(default)] shift: bool },
    /// Mouse event at 1-based cell coordinates, like SGR mouse reports.
    /// `kind` is one of `down-left`, `up-left`, `drag-left`,
    /// `scroll-down`, `scroll-up`.
    Mouse { kind: String, column: u16, row: u16 },
    /// New cell geometry.
    Resize { width: u16, height: u16 },
    /// End the session. The server answers with a final `quit` frame.
    Quit,
}

/// Server to client: a `hello` first, then one `frame` per draw.
/// `rows` are the post-draw cell rows with trailing blanks trimmed,
/// `current` is the viewed issue/PR number, `cursor` the cell cursor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Hello { protocol: u32 },
    Frame {
        tick: u64,
        current: Option<u64>,
        rows: Vec<String>,
        width: u16,
        height: u16,
        cursor: Option<[u16; 2]>,
        quit: bool,
    },
}

/// Parse one client line. Unknown lines are a client bug: the caller
/// drops them so one bad line never stalls the loop.
pub fn parse_client_line(line: &str) -> Option<ClientMsg> {
    serde_json::from_str(line.trim()).ok()
}

/// Render one client line back out. Round-trips with `parse_client_line`.
/// Test-only: the shipped client is the Python bridge in `scripts/e2e`.
#[cfg(test)]
pub fn encode_client_msg(msg: &ClientMsg) -> String {
    serde_json::to_string(msg).expect("client message serializes")
}

/// Map a drive-mode key to the crossterm event the terminal loop would
/// have produced. `None` means no such key: the caller ignores it.
pub fn key_event(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::empty();
    if ctrl {
        modifiers |= KeyModifiers::CONTROL;
    }
    if alt {
        modifiers |= KeyModifiers::ALT;
    }
    if shift {
        modifiers |= KeyModifiers::SHIFT;
    }
    let code = if key.chars().count() == 1 {
        KeyCode::Char(key.chars().next().expect("one char"))
    } else {
        match key {
            "Enter" => KeyCode::Enter,
            "Esc" | "Escape" => KeyCode::Esc,
            "Tab" => KeyCode::Tab,
            "Backspace" => KeyCode::Backspace,
            "Delete" => KeyCode::Delete,
            "Up" => KeyCode::Up,
            "Down" => KeyCode::Down,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "PageUp" => KeyCode::PageUp,
            "PageDown" => KeyCode::PageDown,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            _ => return None,
        }
    };
    Some(KeyEvent::new(code, modifiers))
}

/// Map a drive-mode mouse report to the crossterm event the terminal
/// loop would have produced. Coordinates stay 1-based, as SGR sends.
pub fn mouse_event(kind: &str, column: u16, row: u16) -> Option<MouseEvent> {
    let kind = match kind {
        "down-left" => MouseEventKind::Down(MouseButton::Left),
        "up-left" => MouseEventKind::Up(MouseButton::Left),
        "drag-left" => MouseEventKind::Drag(MouseButton::Left),
        "scroll-down" => MouseEventKind::ScrollDown,
        "scroll-up" => MouseEventKind::ScrollUp,
        _ => return None,
    };
    Some(MouseEvent { kind, column, row, modifiers: KeyModifiers::empty() })
}

/// Route one client message through the same handlers the terminal
/// loop uses, so drive mode and the TTY never diverge.
pub(crate) fn dispatch(
    app: &mut crate::app::App,
    msg: &ClientMsg,
    terminal: &mut Terminal<TestBackend>,
    size: &mut (u16, u16),
) {
    match msg {
        ClientMsg::Key { key, ctrl, alt, shift } => {
            if let Some(ev) = key_event(key, *ctrl, *alt, *shift) {
                crate::input::handle_key(app, ev);
            }
        }
        ClientMsg::Mouse { kind, column, row } => {
            if let Some(m) = mouse_event(kind, *column, *row) {
                crate::input::handle_mouse(app, m);
            }
        }
        ClientMsg::Resize { width, height } => {
            if *width > 0 && *height > 0 {
                terminal.backend_mut().resize(*width, *height);
                *size = (*width, *height);
            }
        }
        ClientMsg::Quit => app.quit = true,
    }
}

/// Capture the post-draw cell buffer as frame rows, the way
/// `render_smoke` reads `TestBackend` today.
pub(crate) fn capture_frame(
    terminal: &mut Terminal<TestBackend>,
    app: &crate::app::App,
    size: (u16, u16),
) -> ServerMsg {
    let buf = terminal.backend().buffer().clone();
    let (width, height) = size;
    let mut rows = Vec::with_capacity(height as usize);
    for y in 0..height {
        let line: String = (0..width).map(|x| buf[(x, y)].symbol()).collect();
        rows.push(line.trim_end().to_string());
    }
    let cursor = terminal.get_cursor_position().ok().map(|p| [p.x, p.y]);
    ServerMsg::Frame { tick: app.tick, current: app.nav.current, rows, width, height, cursor, quit: app.quit }
}

fn send_msg(stream: &mut std::os::unix::net::UnixStream, msg: &ServerMsg) -> std::io::Result<()> {
    let mut line = serde_json::to_string(msg).expect("server message serializes");
    line.push('\n');
    let bytes = line.as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "client gone")),
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Drain every complete client line waiting on the socket. Returns
/// `Ok(false)` when the client hung up.
fn drain_lines(
    stream: &mut std::os::unix::net::UnixStream,
    pending: &mut Vec<u8>,
) -> std::io::Result<(bool, Vec<String>)> {
    let mut tmp = [0u8; 4096];
    let connected = loop {
        match stream.read(&mut tmp) {
            Ok(0) => break false,
            Ok(n) => pending.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break true,
            Err(e) => return Err(e),
        }
    };
    if !connected {
        return Ok((false, vec![]));
    }
    let mut lines = vec![];
    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
        let raw: Vec<u8> = pending.drain(..=pos).collect();
        let text = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
        if !text.trim().is_empty() {
            lines.push(text);
        }
    }
    Ok((true, lines))
}

/// Headless runner: cell-buffer backend at a fixed geometry, the Unix
/// socket as the event source, the same draw plus `poll_all` shape as
/// the terminal loop. Serves exactly one client; ends when the client
/// quits, hangs up, or the app quits.
pub(crate) fn run_drive(
    app: &mut crate::app::App,
    numbers: &[u64],
    sock_path: &Path,
    width: u16,
    height: u16,
) -> Result<(), String> {
    crate::app::bootstrap(app, numbers);
    if sock_path.exists() {
        std::fs::remove_file(sock_path).map_err(|e| e.to_string())?;
    }
    let listener =
        std::os::unix::net::UnixListener::bind(sock_path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let (stream, _) = listener.accept().map_err(|e| e.to_string())?;
    serve(app, stream, width, height)
}

/// Drive-mode landing for an armed editor run: no terminal exists to
/// suspend for `$EDITOR`, so the run degrades to naming the limitation
/// and the draft stays open. True when it fired.
pub(crate) fn land_editor_request_drive(app: &mut crate::app::App) -> bool {
    if !app.compose.editor_requested {
        return false;
    }
    app.compose.editor_requested = false;
    app.notify("external editor needs the terminal UI, not drive mode".to_string());
    true
}

/// Serve one connected client: hello first, then a frame per draw.
/// Split from `run_drive` so tests can drive the wire protocol over
/// an already-connected stream with no filesystem bind.
pub(crate) fn serve(
    app: &mut crate::app::App,
    stream: std::os::unix::net::UnixStream,
    width: u16,
    height: u16,
) -> Result<(), String> {
    let mut stream = stream;
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).map_err(|e| format!("{e:?}"))?;
    if send_msg(&mut stream, &ServerMsg::Hello { protocol: PROTOCOL_VERSION }).is_err() {
        return Ok(());
    }
    let mut size = (width, height);
    let mut pending: Vec<u8> = Vec::new();
    loop {
        // Same shape as the terminal loop: tick, draw, events, landers.
        app.tick = app.tick.wrapping_add(1);
        terminal.draw(|f| crate::ui::render(f, app)).map_err(|e| format!("{e:?}"))?;
        let frame = capture_frame(&mut terminal, app, size);
        if send_msg(&mut stream, &frame).is_err() {
            return Ok(());
        }
        if app.quit {
            return Ok(());
        }
        let (connected, lines) = drain_lines(&mut stream, &mut pending).map_err(|e| e.to_string())?;
        if !connected {
            return Ok(());
        }
        for line in lines {
            if let Some(msg) = parse_client_line(&line) {
                dispatch(app, &msg, &mut terminal, &mut size);
            }
        }
        crate::app::poll_all(app);
        // Drive mode has no terminal to suspend for `gh auth refresh`,
        // so an accepted prompt degrades to naming the command.
        if app.auth.reauth_requested {
            app.auth.reauth_requested = false;
            if matches!(app.confirm.as_ref().map(|d| &d.kind), Some(crate::app::ConfirmKind::Reauth)) {
                app.confirm = None;
            }
            app.notify(format!("re-auth by hand: {}", crate::github::reauth_command()));
        }
        land_editor_request_drive(app);
        if app.quit {
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| crate::ui::render(f, app)).map_err(|e| format!("{e:?}"))?;
            let frame = capture_frame(&mut terminal, app, size);
            let _ = send_msg(&mut stream, &frame);
            return Ok(());
        }
        // No crossterm poll to pace us; 30ms keeps drive traffic near
        // the terminal loop's 100ms cadence without busy-spinning.
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: &str) -> ClientMsg {
        ClientMsg::Key { key: code.into(), ctrl: false, alt: false, shift: false }
    }

    fn next_line(reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>) -> String {
        use std::io::BufRead;
        let mut line = String::new();
        reader.read_line(&mut line).expect("server speaks");
        line
    }

    fn send_msg_client(writer: &mut std::os::unix::net::UnixStream, msg: &ClientMsg) {
        use std::io::Write;
        let line = encode_client_msg(msg);
        writer.write_all(line.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
    }

    fn read_frame(reader: &mut std::io::BufReader<std::os::unix::net::UnixStream>) -> ServerMsg {
        use std::io::BufRead;
        for _ in 0..500 {
            let mut line = String::new();
            reader.read_line(&mut line).expect("frame arrives");
            let msg: ServerMsg = serde_json::from_str(line.trim()).unwrap();
            if let ServerMsg::Frame { .. } = msg {
                return msg;
            }
        }
        panic!("no frame arrived");
    }

    fn frame_text(msg: &ServerMsg) -> String {
        match msg {
            ServerMsg::Frame { rows, .. } => rows.join("\n"),
            ServerMsg::Hello { .. } => panic!("expected frame"),
        }
    }

    #[test]
    fn protocol_key_round_trip() {
        let cases = vec![
            key("j"),
            key(" "),
            key(":"),
            key("Enter"),
            key("Esc"),
            key("Tab"),
            key("Up"),
            key("PageDown"),
            ClientMsg::Key { key: "o".into(), ctrl: true, alt: false, shift: false },
            ClientMsg::Mouse { kind: "down-left".into(), column: 4, row: 25 },
            ClientMsg::Mouse { kind: "scroll-up".into(), column: 40, row: 5 },
            ClientMsg::Resize { width: 120, height: 40 },
            ClientMsg::Quit,
        ];
        for msg in cases {
            let line = encode_client_msg(&msg);
            assert_eq!(parse_client_line(&line), Some(msg), "round trip {line}");
        }
    }

    #[test]
    fn protocol_rejects_garbage_quietly() {
        assert_eq!(parse_client_line(""), None);
        assert_eq!(parse_client_line("not json"), None);
        assert_eq!(parse_client_line(r#"{"type":"key"}"#), None, "key needs a code");
        assert_eq!(parse_client_line(r#"{"type":"nope"}"#), None);
    }

    #[test]
    fn server_frame_round_trip() {
        let frame = ServerMsg::Frame {
            tick: 7,
            current: Some(9),
            rows: vec!["ghit  #9".to_string(), String::new()],
            width: 120,
            height: 40,
            cursor: Some([3, 2]),
            quit: false,
        };
        let line = serde_json::to_string(&frame).unwrap();
        let back: ServerMsg = serde_json::from_str(&line).unwrap();
        assert_eq!(back, frame);
        let hello = ServerMsg::Hello { protocol: PROTOCOL_VERSION };
        let line = serde_json::to_string(&hello).unwrap();
        assert!(line.contains("\"hello\""), "{line}");
        assert!(line.contains(&PROTOCOL_VERSION.to_string()), "{line}");
    }

    #[test]
    fn key_event_maps_codes_and_modifiers() {
        let ev = key_event("j", false, false, false).unwrap();
        assert_eq!(ev.code, KeyCode::Char('j'));
        assert!(ev.modifiers.is_empty());
        assert_eq!(key_event("Enter", false, false, false).unwrap().code, KeyCode::Enter);
        assert_eq!(key_event("Esc", false, false, false).unwrap().code, KeyCode::Esc);
        assert_eq!(key_event("Escape", false, false, false).unwrap().code, KeyCode::Esc);
        let ev = key_event("o", true, false, false).unwrap();
        assert!(ev.modifiers.contains(KeyModifiers::CONTROL));
        assert!(key_event("nope-key", false, false, false).is_none());
    }

    #[test]
    fn mouse_event_maps_kinds_and_stays_1_based() {
        let m = mouse_event("down-left", 4, 25).unwrap();
        assert_eq!(m.kind, MouseEventKind::Down(MouseButton::Left));
        assert_eq!((m.column, m.row), (4, 25));
        assert!(matches!(mouse_event("scroll-up", 1, 1).unwrap().kind, MouseEventKind::ScrollUp));
        assert!(mouse_event("down-right", 1, 1).is_none());
    }

    fn driven_app() -> crate::app::App {
        use crate::model::{Author, IssueData};
        let issue = |number: u64| IssueData {
            number,
            title: format!("issue {number}"),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![],
            assignees: vec![],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-05T00:00:00Z".to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            body: "hello".to_string(),
            comments: vec![],
            review_threads: vec![],
            paired: vec![],
            checks: None,
            milestone: None,
            parent: None,
            sub_summary: None,
            is_pr: false,
        };
        let mut app = crate::app::App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7, 9] {
            app.issues.data.insert(n, issue(n));
            app.issues.statuses.insert(n, vec![]);
            app.nav.visited.push(n);
        }
        app.show_issue(7);
        app
    }

    fn drive_on(app: &mut crate::app::App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| crate::ui::render(f, app)).unwrap();
        // Warm the capture the way the loop does: one draw first so
        // hit areas and body rows exist before any key lands.
        let _ = capture_frame(&mut terminal, app, (width, height));
        terminal
    }

    #[test]
    fn drive_keys_move_and_quit_in_process() {
        let mut app = driven_app();
        let mut terminal = drive_on(&mut app, 120, 40);
        let mut size = (120, 40);
        assert_eq!(app.nav.current, Some(7));
        dispatch(&mut app, &key("j"), &mut terminal, &mut size);
        assert_eq!(app.nav.current, Some(9), "j walks the Issues pane");
        dispatch(&mut app, &key("k"), &mut terminal, &mut size);
        assert_eq!(app.nav.current, Some(7), "k walks back");
        assert!(!app.quit);
        dispatch(&mut app, &key("q"), &mut terminal, &mut size);
        assert!(app.quit, "q quits drive mode too");
    }

    #[test]
    fn drive_compose_edits_with_bash_chords() {
        let mut app = driven_app();
        let mut terminal = drive_on(&mut app, 120, 40);
        let mut size = (120, 40);
        let mut send = |app: &mut crate::app::App, key: &str, ctrl: bool, alt: bool| {
            dispatch(
                app,
                &ClientMsg::Key { key: key.into(), ctrl, alt, shift: false },
                &mut terminal,
                &mut size,
            );
        };
        send(&mut app, "c", false, false);
        assert!(app.compose_open());
        send(&mut app, "h", false, false);
        send(&mut app, "i", false, false);
        send(&mut app, "a", true, false);
        assert_eq!(app.compose.draft.as_ref().unwrap().cursor, 0, "ctrl-a over the socket");
        send(&mut app, "f", false, true);
        assert_eq!(app.compose.draft.as_ref().unwrap().cursor, 2, "alt-f over the socket");
        send(&mut app, "a", true, false);
        send(&mut app, "k", true, false);
        assert!(app.compose.draft.as_ref().unwrap().body.is_empty(), "ctrl-k over the socket");
        // No terminal exists to suspend here, so the armed run degrades.
        app.compose.editor_requested = true;
        assert!(super::land_editor_request_drive(&mut app));
        assert!(!app.compose.editor_requested);
        assert!(app.message.contains("terminal UI"));
        assert!(app.compose_open(), "draft survives");
        assert!(!super::land_editor_request_drive(&mut app), "nothing armed, nothing fired");
    }

    #[test]
    fn drive_ctrl_c_quits_like_the_terminal() {
        let mut app = driven_app();
        let mut terminal = drive_on(&mut app, 120, 40);
        let mut size = (120, 40);
        let ctrl_c = ClientMsg::Key { key: "c".into(), ctrl: true, alt: false, shift: false };
        dispatch(&mut app, &ctrl_c, &mut terminal, &mut size);
        assert!(app.quit);
    }

    #[test]
    fn frame_capture_carries_rows_current_and_tick() {
        let mut app = driven_app();
        let mut terminal = drive_on(&mut app, 200, 50);
        app.tick = 41;
        let frame = capture_frame(&mut terminal, &app, (200, 50));
        match frame {
            ServerMsg::Frame { tick, current, rows, width, height, quit, .. } => {
                assert_eq!(tick, 41);
                assert_eq!(current, Some(7));
                assert_eq!((width, height), (200, 50));
                assert_eq!(rows.len(), 50);
                assert!(!quit);
                let text = rows.join("\n");
                assert!(text.contains("ghit"), "masthead renders headless");
                assert!(text.contains("issue 7"), "seeded issue renders");
            }
            ServerMsg::Hello { .. } => panic!("expected a frame, got hello"),
        }
    }

    #[test]
    fn socket_boot_drive_keys_capture_frames() {
        use std::io::BufReader;
        use std::os::unix::net::{UnixListener, UnixStream};

        // Prefer the real path: bind a socket file, check its mode,
        // accept one client. Sandboxes that forbid `bind` fall back
        // to a socketpair, which still runs the hello/drive-frames/
        // quit wire protocol end to end.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("drive.sock");
        let bind_ok = UnixListener::bind(&sock).is_ok();
        if bind_ok {
            std::fs::remove_file(&sock).unwrap();
        }
        let (server, stream) = if bind_ok {
            let sock_path = sock.clone();
            let server = std::thread::spawn(move || {
                let mut app = driven_app();
                run_drive(&mut app, &[], &sock_path, 200, 50)
            });
            // The server binds before accepting, so wait for the path.
            for _ in 0..200 {
                if sock.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let mode = std::fs::metadata(&sock).unwrap().mode() & 0o777;
                assert_eq!(mode, 0o700, "drive socket is owner-only");
            }
            let stream = UnixStream::connect(&sock).expect("connect to drive socket");
            (server, stream)
        } else {
            eprintln!("drive test: bind forbidden here; covering the wire over socketpair");
            let (client, server_side) = UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                let mut app = driven_app();
                serve(&mut app, server_side, 200, 50)
            });
            (server, client)
        };
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        // Hello comes first and names the protocol version.
        let hello: ServerMsg = serde_json::from_str(next_line(&mut reader).trim()).unwrap();
        assert_eq!(hello, ServerMsg::Hello { protocol: PROTOCOL_VERSION });

        // The seeded issue renders without any fetch.
        let mut seen = false;
        for _ in 0..200 {
            let frame = read_frame(&mut reader);
            if frame_text(&frame).contains("issue 7") {
                seen = true;
                break;
            }
        }
        assert!(seen, "seeded issue renders over the socket");

        // `j` moves the selection on a fresh boot: startup stays in
        // Main so keys act at once, with no Escape preamble.
        send_msg_client(&mut writer, &key("j"));
        let mut moved = false;
        for _ in 0..200 {
            let frame = read_frame(&mut reader);
            if matches!(frame, ServerMsg::Frame { current: Some(9), .. }) {
                moved = true;
                break;
            }
        }
        assert!(moved, "`j` moves current to 9 over the socket");

        // `q` ends the session with a final quit frame, no tmux needed.
        send_msg_client(&mut writer, &key("q"));
        let mut quit = false;
        for _ in 0..200 {
            let line = next_line(&mut reader);
            if line.trim().is_empty() {
                continue;
            }
            let msg: ServerMsg = serde_json::from_str(line.trim()).unwrap();
            if matches!(msg, ServerMsg::Frame { quit: true, .. }) {
                quit = true;
                break;
            }
        }
        assert!(quit, "server ends on `q`");
        server.join().expect("server thread").expect("clean shutdown");
    }
}
