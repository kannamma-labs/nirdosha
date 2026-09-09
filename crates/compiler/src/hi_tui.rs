//! The interactive front end for `nirdosha hi`: a `ratatui`/`crossterm`
//! full-screen console, styled after Claude Code's own CLI (a bordered
//! header carrying the product's wordmark in its top-left corner, a
//! scrolling transcript, a sticky input box) -- but a pure presentation
//! layer over the *exact same* `super::generate_and_build`/
//! `super::explain_diagnostic` calls `run_console_plain` makes. Nothing
//! in this file talks to the model or the compiler directly.
//!
//! The one behavior this layer adds that the plain loop didn't need:
//! `LlmClient::complete` is a blocking call that can take up to its own
//! 120s timeout, so it runs on a background thread (`std::thread::spawn`)
//! that reports back over an `mpsc` channel -- otherwise the whole UI
//! (including the quit key) would freeze for the duration of every
//! request.

use std::cell::Cell;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use rusqlite::Connection;

use super::{explain_diagnostic, failure_counters, format_ask_hits, format_impact_report, generate_and_build, parse_line, Activation, Command, FailureCounters, LlmClient, LogEvent, SessionLog, TokenUsage};

/// The one-time full-screen intro played by [`run`] before the console
/// proper starts -- see that fn's own comment for why it's a distinct
/// phase from the small, permanent header glyph `draw_header` renders.
#[path = "hi_logo_anim.rs"]
mod logo_anim;
#[path = "hi_logo_pixels.rs"]
mod logo_pixels;

const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// Sampled from `assets/brand/nirdosha-logo.png` itself (not guessed),
/// so the colored-text fallback and the "programming language" caption
/// next to the real image use the same navy/orange the logo does.
const BRAND_NAVY: Color = Color::Rgb(29, 24, 76);
const BRAND_ORANGE: Color = Color::Rgb(239, 159, 48);

/// Which inline-image escape sequence dialect (if any) the terminal
/// we're running in understands. Detected once at startup from env
/// vars alone -- no active terminal-capability query -- since that
/// covers every terminal that actually implements one of these today
/// (Kitty, WezTerm, Ghostty all advertise `KITTY_WINDOW_ID` or a
/// recognizable `TERM_PROGRAM`; iTerm2 sets `TERM_PROGRAM=iTerm.app`)
/// without the extra complexity of writing a query escape and parsing
/// a response back out of `crossterm`'s own input stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphicsProtocol {
    /// No known inline-image support -- e.g. Alacritty, plain xterm,
    /// most Linux terminal emulators, tmux. Falls back to the colored
    /// ASCII wordmark.
    None,
    /// The Kitty graphics protocol -- also implemented by WezTerm and
    /// Ghostty, which is why this variant covers all three rather than
    /// naming just Kitty.
    Kitty,
    /// iTerm2's own OSC 1337 inline-image extension.
    Iterm2,
}

fn detect_graphics_protocol() -> GraphicsProtocol {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if env("KITTY_WINDOW_ID").is_some() {
        return GraphicsProtocol::Kitty;
    }
    match env("TERM_PROGRAM").as_deref() {
        Some("WezTerm") | Some("ghostty") => return GraphicsProtocol::Kitty,
        Some("iTerm.app") => return GraphicsProtocol::Iterm2,
        _ => {}
    }
    if env("TERM").map(|t| t.contains("kitty")).unwrap_or(false) {
        return GraphicsProtocol::Kitty;
    }
    GraphicsProtocol::None
}

/// Just the glyph, cropped (with a transparent background painted in
/// over what was originally solid white -- see the crop script in this
/// change's own history) from the full lockup in
/// `assets/brand/nirdosha-logo.png`, which also carries the
/// "NIRDOSHA PROGRAMMING LANGUAGE" wordmark baked into the pixels --
/// far too much detail to read at the handful of terminal cells this
/// header has room for. The wordmark is re-set as real (so: legible,
/// theme-colored) text in `draw_header` instead.
const NIRDOSHA_ICON_PNG: &[u8] = include_bytes!("../../../assets/brand/nirdosha-icon.png");

/// Sized in terminal cells, not pixels -- both inline-image protocols
/// scale the source image to fit whatever `cols x rows` box they're
/// told to render into, and `block_art_lines` below renders exactly
/// `ICON_COLS x (ICON_ROWS * 2)` source pixels into the same box. 2:1
/// (cols:rows) because a terminal cell itself is roughly twice as tall
/// as it is wide, so a 2:1 box renders the (square) icon at roughly
/// its true aspect ratio instead of looking squashed.
const ICON_COLS: u16 = 24;
const ICON_ROWS: u16 = 12;

/// A flat `width*height*4` (RGBA, row-major) pixel dump -- not a PNG --
/// pre-downsampled from `nirdosha-icon.png` to exactly the resolution
/// `block_art_lines` renders (`ICON_COLS` wide, `ICON_ROWS * 2` tall,
/// one source pixel per terminal half-cell). Baked in at this exact
/// size rather than decoded from the PNG at runtime so this file's own
/// fallback path -- the one every terminal without Kitty/iTerm2 support
/// actually uses, this codebase's own Alacritty included -- needs no
/// PNG-decoding dependency of its own.
const ICON_PIXELS: &[u8] = include_bytes!("../../../assets/brand/nirdosha-icon-24x24.rgba");
const ICON_PIXEL_DIM: usize = 24;

/// Below this, a sampled pixel is treated as background (the logo's
/// transparent surround), not ink -- there's no terminal-background
/// color to alpha-blend against in general, so this is a hard cutoff
/// rather than a blend.
const ICON_ALPHA_THRESHOLD: u8 = 90;

fn icon_pixel(x: usize, y: usize) -> Option<Color> {
    let i = (y * ICON_PIXEL_DIM + x) * 4;
    (ICON_PIXELS[i + 3] >= ICON_ALPHA_THRESHOLD).then(|| Color::Rgb(ICON_PIXELS[i], ICON_PIXELS[i + 1], ICON_PIXELS[i + 2]))
}

/// The universal fallback for terminals with no inline-image support
/// at all (this codebase's own Alacritty, most Linux terminal
/// emulators, tmux, ...): the classic "half-block" trick, where each
/// terminal cell encodes *two* vertically-stacked source pixels -- one
/// painted as `\u{2580}`/`\u{2584}`'s foreground, one as its background
/// -- which happens to roughly correct for a terminal cell itself
/// being about twice as tall as it is wide. A transparent top+bottom
/// pair renders as a plain, unstyled space rather than a solid block,
/// so the rendered shape follows the logo's actual silhouette instead
/// of filling a rectangle.
fn block_art_lines() -> Vec<Line<'static>> {
    (0..ICON_ROWS as usize)
        .map(|row| {
            Line::from(
                (0..ICON_COLS as usize)
                    .map(|col| match (icon_pixel(col, row * 2), icon_pixel(col, row * 2 + 1)) {
                        (Some(top), Some(bottom)) => Span::styled("\u{2580}", Style::default().fg(top).bg(bottom)),
                        (Some(top), None) => Span::styled("\u{2580}", Style::default().fg(top)),
                        (None, Some(bottom)) => Span::styled("\u{2584}", Style::default().fg(bottom)),
                        (None, None) => Span::raw(" "),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// Writes the one escape sequence that actually draws the logo, in
/// whichever dialect `protocol` calls for, positioned at `area`'s
/// top-left cell. Bypasses `ratatui`/ the `Terminal` entirely -- ANSI
/// image escapes are their own side channel, not something `ratatui`'s
/// widget tree knows how to express -- so this must run *after* the
/// `Terminal::draw` call it's paired with, never interleaved with one,
/// or the two writers' bytes could interleave on the wire.
fn draw_icon(protocol: GraphicsProtocol, area: Rect) -> io::Result<()> {
    if protocol == GraphicsProtocol::None || area.width == 0 || area.height == 0 {
        return Ok(());
    }
    let mut out = io::stdout();
    execute!(out, MoveTo(area.x, area.y))?;
    match protocol {
        GraphicsProtocol::Kitty => write_kitty_image(&mut out, area)?,
        GraphicsProtocol::Iterm2 => write_iterm2_image(&mut out, area)?,
        GraphicsProtocol::None => unreachable!(),
    }
    out.flush()
}

/// <https://sw.kovidgoyal.net/kitty/graphics-protocol/>'s "direct"
/// transmission path: `f=100` hands the terminal the original PNG
/// bytes and lets its own PNG decoder do the work, so this needs no
/// image-decoding dependency of its own. Payloads over 4096 base64
/// bytes must be split across multiple escape sequences (`m=1` on
/// every chunk but the last) -- the protocol's own limit, not a
/// choice made here. `q=2` suppresses the terminal's success/failure
/// response, which would otherwise land in `crossterm`'s input stream
/// as unrecognized bytes for `handle_key` to trip over.
fn write_kitty_image(out: &mut impl Write, area: Rect) -> io::Result<()> {
    let payload = BASE64_STANDARD.encode(NIRDOSHA_ICON_PNG);
    let chunks: Vec<&[u8]> = payload.as_bytes().chunks(4096).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i != last);
        let chunk = std::str::from_utf8(chunk).expect("chunking a base64 string on byte boundaries stays valid UTF-8");
        if i == 0 {
            write!(out, "\x1b_Ga=T,f=100,q=2,c={},r={},m={more};{chunk}\x1b\\", area.width, area.height)?;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    Ok(())
}

/// iTerm2's own (simpler, unchunked) inline-image extension --
/// <https://iterm2.com/documentation-images.html>.
fn write_iterm2_image(out: &mut impl Write, area: Rect) -> io::Result<()> {
    let payload = BASE64_STANDARD.encode(NIRDOSHA_ICON_PNG);
    write!(out, "\x1b]1337;File=inline=1;width={};height={};preserveAspectRatio=1:{payload}\x07", area.width, area.height)
}

/// One rendered line of the transcript. Kept as plain data (not
/// pre-styled `ratatui` types) so the draw function is the only place
/// that decides how each kind looks.
enum EntryKind {
    UserInput,
    Info,
    Error,
    Success,
    Explanation,
    Notice,
}

struct Entry {
    kind: EntryKind,
    text: String,
}

/// What a background worker thread reports back to the render loop.
enum WorkerEvent {
    Log(LogEvent),
    GenerateDone(Option<String>),
    ExplainDone(Result<String, String>),
}

struct App {
    model: String,
    key_redacted: String,
    input: String,
    entries: Vec<Entry>,
    last_diagnostic: Option<String>,
    busy: bool,
    spinner_tick: usize,
    scroll_from_bottom: u16,
    should_quit: bool,
    usage: TokenUsage,
    /// Durable, cross-process count from `failure_counters()` -- refreshed
    /// every tick just like `usage`, so it stays live even though nothing
    /// in this process is the sole writer of it (a compiler panic that
    /// kills a *different* `nirdosha hi` process still shows up here).
    failures: FailureCounters,
    /// `.nir/realm.db` (rfcs/0013), opened once by `super::
    /// open_realm_or_warn` before either front end starts -- `None`
    /// when Realm is disabled or couldn't be opened, in which case
    /// `:ask`/`:impact` say so instead of panicking.
    realm: Option<Connection>,
}

impl App {
    fn push(&mut self, kind: EntryKind, text: impl Into<String>) {
        self.entries.push(Entry { kind, text: text.into() });
        self.scroll_from_bottom = 0; // new content always snaps the view back to the bottom
    }
}

pub(super) fn run(activation: Activation, log: SessionLog, realm: Option<Connection>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Deliberately *not* `EnableMouseCapture`: this app has no mouse
    // handling to give those events to, and capturing them anyway
    // would only take click-drag text selection away from the
    // terminal itself -- the transcript pane needs to stay selectable
    // and copyable the normal way.
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    terminal.hide_cursor()?;
    let splash_result = show_splash(&mut terminal);
    terminal.show_cursor()?;

    let result = splash_result.and_then(|()| run_app(&mut terminal, activation, log, realm));

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

/// Plays `logo_anim`'s reveal -> type -> idle-breathe animation
/// full-screen, once, before the console proper starts -- a distinct
/// phase from `draw_header`'s small, permanent glyph (which stays
/// exactly as it was: this doesn't touch that path at all). Ends the
/// moment [`logo_anim::intro_done_at`] elapses, or immediately on any
/// keypress so it never gets in a returning user's way; Ctrl+C during
/// the splash quits the same as it does everywhere else in `hi`.
fn show_splash(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let start = Instant::now();
    let hold = logo_anim::intro_done_at();
    loop {
        let elapsed = start.elapsed().as_secs_f32();
        terminal.draw(|f| {
            let area = f.area();
            logo_anim::render(f.buffer_mut(), area, elapsed);
        })?;
        if elapsed >= hold {
            return Ok(());
        }
        let remaining = Duration::from_secs_f32((hold - elapsed).max(0.0));
        let frame_budget = Duration::from_millis(16);
        if event::poll(remaining.min(frame_budget))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    return Ok(());
                }
            }
        }
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, activation: Activation, log: SessionLog, realm: Option<Connection>) -> io::Result<()> {
    let model = activation.model.clone();
    let key_redacted = activation.redacted_key();
    let client = Arc::new(LlmClient::new(activation));

    let mut app = App {
        model,
        key_redacted,
        input: String::new(),
        entries: Vec::new(),
        last_diagnostic: None,
        busy: false,
        spinner_tick: 0,
        scroll_from_bottom: 0,
        should_quit: false,
        usage: TokenUsage::default(),
        failures: failure_counters(),
        realm,
    };
    app.push(EntryKind::Notice, "Type a description of the program you want (or `:build <description>`), `:explain` to explain the last build error, `:ask <question>`/`:impact <target>` to query the project's realm graph, or `:quit` to exit.");
    if let Some(path) = log.path() {
        app.push(EntryKind::Notice, format!("session log: {}", path.display()));
    }
    app.push(EntryKind::Notice, format!("compile failures so far (all-time, this machine): {} self-repair retries, {} compiler panics", app.failures.self_repair_retries, app.failures.compiler_panics));

    let (tx, rx): (Sender<WorkerEvent>, Receiver<WorkerEvent>) = mpsc::channel();
    let protocol = detect_graphics_protocol();
    // The header is always the frame's topmost chunk, so its (and the
    // icon's) position is invariant across resizes -- only surrounding
    // widths reflow -- which is why the image only ever needs sending
    // once, not re-sent on every `Event::Resize`. `icon_rect` only ever
    // comes back `Some` when `protocol` calls for a raw escape-sequence
    // image; the `block_art_lines` fallback is ordinary widget content
    // `draw_header` already rendered by the time `terminal.draw`
    // returns, so there's nothing left for `draw_icon` to send for it.
    let mut icon_sent = false;
    let icon_rect: Cell<Option<Rect>> = Cell::new(None);

    while !app.should_quit {
        terminal.draw(|f| icon_rect.set(draw(f, &app, protocol)))?;
        if !icon_sent {
            if let Some(area) = icon_rect.get() {
                draw_icon(protocol, area)?;
            }
            icon_sent = true;
        }

        // A short poll timeout, not a blocking read, so the spinner
        // animates and worker events (which arrive with no keyboard
        // input at all) get picked up promptly.
        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(&mut app, &client, &tx, &log, key.code, key.modifiers);
                }
            }
        }

        while let Ok(event) = rx.try_recv() {
            handle_worker_event(&mut app, event);
        }
        // Cheap enough (a mutex lock and a `Copy`) to refresh every
        // tick rather than only on `WorkerEvent` arrival -- keeps the
        // header's token count live without `LlmClient` needing to
        // push updates of its own.
        app.usage = client.usage_totals();
        app.failures = failure_counters();

        if app.busy {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, client: &Arc<LlmClient>, tx: &Sender<WorkerEvent>, log: &SessionLog, code: KeyCode, modifiers: KeyModifiers) {
    // Ctrl+C always exits, busy or not -- the in-flight request's
    // worker thread is left to finish and is simply ignored, the same
    // "abandon, don't try to cancel a blocking HTTP call" choice a
    // plain Ctrl+C at a shell prompt makes.
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    if app.busy {
        return; // input is frozen while a request is in flight, mirroring the plain loop's one-request-at-a-time console
    }
    match code {
        KeyCode::Enter => submit(app, client, tx, log),
        KeyCode::Char(c) => app.input.push(c),
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Up => app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(1),
        KeyCode::Down => app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(1),
        KeyCode::PageUp => app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10),
        KeyCode::PageDown => app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(10),
        _ => {}
    }
}

fn submit(app: &mut App, client: &Arc<LlmClient>, tx: &Sender<WorkerEvent>, log: &SessionLog) {
    let line = app.input.trim().to_string();
    app.input.clear();
    if line.is_empty() {
        return;
    }
    log.log(format!("> {line}"));
    // `parse_line` borrows `line`; resolve it to owned data up front so
    // the borrow doesn't outlive the `move` closures spawned below.
    match parse_line(&line) {
        Command::Quit => {
            app.should_quit = true;
        }
        Command::Explain => {
            app.push(EntryKind::UserInput, ":explain");
            match app.last_diagnostic.clone() {
                Some(diagnostic) => {
                    app.busy = true;
                    let client = Arc::clone(client);
                    let tx = tx.clone();
                    let log = log.clone();
                    thread::spawn(move || {
                        let result = explain_diagnostic(&client, &diagnostic);
                        log.log(match &result {
                            Ok(explanation) => format!("explanation: {explanation}"),
                            Err(e) => format!("explain error: {e}"),
                        });
                        let _ = tx.send(WorkerEvent::ExplainDone(result));
                    });
                }
                None => app.push(EntryKind::Notice, "no build has failed yet in this session -- nothing to explain."),
            }
        }
        Command::Ask(question) => {
            app.push(EntryKind::UserInput, format!(":ask {question}"));
            match &app.realm {
                Some(conn) => match crate::realm::ask(conn, question) {
                    Ok(hits) => app.push(EntryKind::Notice, format_ask_hits(question, &hits)),
                    Err(e) => app.push(EntryKind::Notice, e),
                },
                None => app.push(EntryKind::Notice, "the realm graph isn't available this session -- try `nirdosha realm ingest`/`sync` from a shell instead."),
            }
        }
        Command::Impact(target) => {
            app.push(EntryKind::UserInput, format!(":impact {target}"));
            match &app.realm {
                Some(conn) => match crate::realm::impact(conn, target) {
                    Ok(report) => app.push(EntryKind::Notice, format_impact_report(target, &report)),
                    Err(e) => app.push(EntryKind::Notice, e),
                },
                None => app.push(EntryKind::Notice, "the realm graph isn't available this session -- try `nirdosha realm impact` from a shell instead."),
            }
        }
        Command::Unknown(other) => {
            app.push(EntryKind::UserInput, other.to_string());
            app.push(EntryKind::Notice, format!("unrecognized command `{other}` -- try `:explain`, `:ask`, `:impact`, or `:quit`."));
        }
        Command::Request { description, serve } => {
            app.push(EntryKind::UserInput, description.to_string());
            app.busy = true;
            let request = description.to_string();
            let client = Arc::clone(client);
            let tx = tx.clone();
            let log = log.clone();
            thread::spawn(move || {
                let tx_log = tx.clone();
                let diagnostic = generate_and_build(&client, &request, serve, &|event| {
                    log.log(match &event {
                        LogEvent::Info(s) => format!("info: {s}"),
                        LogEvent::Error(s) => format!("error: {s}"),
                        LogEvent::Success(s) => format!("success: {s}"),
                    });
                    let _ = tx_log.send(WorkerEvent::Log(event));
                });
                let _ = tx.send(WorkerEvent::GenerateDone(diagnostic));
            });
        }
    }
}

fn handle_worker_event(app: &mut App, event: WorkerEvent) {
    match event {
        WorkerEvent::Log(LogEvent::Info(s)) => app.push(EntryKind::Info, s),
        WorkerEvent::Log(LogEvent::Error(s)) => app.push(EntryKind::Error, s),
        WorkerEvent::Log(LogEvent::Success(s)) => app.push(EntryKind::Success, s),
        WorkerEvent::GenerateDone(diagnostic) => {
            app.last_diagnostic = diagnostic;
            app.busy = false;
        }
        WorkerEvent::ExplainDone(Ok(explanation)) => {
            app.push(EntryKind::Explanation, explanation);
            app.busy = false;
        }
        WorkerEvent::ExplainDone(Err(e)) => {
            app.push(EntryKind::Error, format!("couldn't reach the model to explain that: {e}"));
            app.busy = false;
        }
    }
}

/// Returns the absolute screen `Rect` a raw escape-sequence image
/// still needs to be drawn into after this frame, or `None` when
/// either this terminal has no inline-image support (the glyph was
/// already rendered as ordinary `block_art_lines` widget content) or
/// there's no room for the header's icon column at all.
fn draw(f: &mut Frame, app: &App, protocol: GraphicsProtocol) -> Option<Rect> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(ICON_ROWS + 2), Constraint::Min(3), Constraint::Length(3)])
        .split(f.area());

    let icon_rect = draw_header(f, chunks[0], protocol, app);
    draw_transcript(f, chunks[1], app);
    draw_input(f, chunks[2], app);
    icon_rect
}

/// Renders the header's wordmark/model/key text, plus the logo glyph
/// itself -- either right now, as `block_art_lines` widget content
/// (the `GraphicsProtocol::None` fallback every terminal can show), or
/// left as blank cells for the caller to hand off to `draw_icon`
/// (`Kitty`/`Iterm2`): no widget ever renders into that `Rect` in that
/// case, so `ratatui`'s diffing backend never emits anything for those
/// cells, leaving the raw-escape-sequence image `draw_icon` draws
/// there (entirely outside `ratatui`'s own rendering) undisturbed for
/// the rest of the session.
fn draw_header(f: &mut Frame, area: Rect, protocol: GraphicsProtocol, app: &App) -> Option<Rect> {
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(BRAND_NAVY));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let icon_rect = Rect { x: inner.x, y: inner.y, width: ICON_COLS.min(inner.width), height: ICON_ROWS.min(inner.height) };
    let text_x_offset = icon_rect.width + 2;
    let text_area = Rect { x: inner.x + text_x_offset, y: inner.y, width: inner.width.saturating_sub(text_x_offset), height: inner.height };

    let lines = vec![
        Line::from(Span::styled("NIRDOSHA", Style::default().fg(BRAND_NAVY).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("programming language", Style::default().fg(BRAND_ORANGE))),
        Line::raw(""),
        Line::from(vec![Span::styled("model ", Style::default().fg(Color::DarkGray)), Span::styled(app.model.clone(), Style::default().fg(Color::White))]),
        Line::from(vec![Span::styled("key ", Style::default().fg(Color::DarkGray)), Span::styled(app.key_redacted.clone(), Style::default().fg(Color::White))]),
        Line::from(vec![
            Span::styled("tokens ", Style::default().fg(Color::DarkGray)),
            Span::styled("in ", Style::default().fg(Color::DarkGray)),
            Span::styled(app.usage.prompt_tokens.to_string(), Style::default().fg(Color::White)),
            Span::styled("  out ", Style::default().fg(Color::DarkGray)),
            Span::styled(app.usage.completion_tokens.to_string(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("failures ", Style::default().fg(Color::DarkGray)),
            Span::styled("retries ", Style::default().fg(Color::DarkGray)),
            Span::styled(app.failures.self_repair_retries.to_string(), Style::default().fg(Color::White)),
            Span::styled("  panics ", Style::default().fg(Color::DarkGray)),
            // Red the moment there's ever been even one -- a compiler
            // panic (unlike a self-repair retry) is never expected, so
            // this count staying visually alarming past zero is the
            // point, not a bug.
            Span::styled(app.failures.compiler_panics.to_string(), if app.failures.compiler_panics > 0 { Style::default().fg(Color::Red).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) }),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), text_area);

    if protocol == GraphicsProtocol::None {
        f.render_widget(Paragraph::new(block_art_lines()), icon_rect);
        None
    } else {
        Some(icon_rect)
    }
}

fn draw_transcript(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)).title(" conversation ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for entry in &app.entries {
        let (prefix, style) = match entry.kind {
            EntryKind::UserInput => ("\u{203a} ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            EntryKind::Info => ("  ", Style::default().fg(Color::DarkGray)),
            EntryKind::Error => ("\u{2717} ", Style::default().fg(Color::Red)),
            EntryKind::Success => ("\u{2713} ", Style::default().fg(Color::Green)),
            EntryKind::Explanation => ("\u{2733} ", Style::default().fg(Color::White)),
            EntryKind::Notice => ("  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        };
        for (i, raw_line) in entry.text.lines().enumerate() {
            let indent = if i == 0 { prefix } else { "  " };
            lines.push(Line::from(Span::styled(format!("{indent}{raw_line}"), style)));
        }
    }
    if app.busy {
        let spinner = SPINNER[app.spinner_tick / 2 % SPINNER.len()];
        lines.push(Line::from(Span::styled(format!("{spinner} waiting for the model..."), Style::default().fg(Color::Yellow))));
    }

    let total = lines.len() as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom.min(max_scroll));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((scroll, 0)), inner);
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let border_color = if app.busy { Color::DarkGray } else { Color::Magenta };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(border_color)).title(" nirdosha hi ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = if app.busy { Line::from(Span::styled("(waiting for the model -- Ctrl+C to quit)", Style::default().fg(Color::DarkGray))) } else { Line::from(vec![Span::styled("\u{276f} ", Style::default().fg(Color::Magenta)), Span::raw(app.input.clone())]) };
    f.render_widget(Paragraph::new(text), inner);

    if !app.busy {
        f.set_cursor_position(Position::new(inner.x + 2 + app.input.chars().count() as u16, inner.y));
    }
}
