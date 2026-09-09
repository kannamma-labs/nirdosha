//! Standalone preview for the animated nirdosha splash.
//!
//! `cargo run --release` -- runs the intro once, then breathes idly
//! until you press any key.
//!
//! To use this in your own TUI app: copy `logo_pixels.rs` and
//! `logo_anim.rs` into it and call `logo_anim::render(buf, area, elapsed)`
//! from your draw function each frame, where `elapsed` is
//! `Instant::now().duration_since(app_start).as_secs_f32()`. That's the
//! entire integration surface -- everything else below is just this
//! demo's own event loop / terminal setup.

mod logo_anim;
mod logo_pixels;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

const FRAME: Duration = Duration::from_millis(16); // ~60fps

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let result = run(&mut terminal);

    terminal.show_cursor()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let start = Instant::now();

    loop {
        let elapsed = start.elapsed().as_secs_f32();

        terminal.draw(|frame| {
            let area = frame.area();
            logo_anim::render(frame.buffer_mut(), area, elapsed);
        })?;

        let deadline = Instant::now() + FRAME;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                break;
            }
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        return Ok(());
                    }
                }
            } else {
                break;
            }
        }
    }
}
