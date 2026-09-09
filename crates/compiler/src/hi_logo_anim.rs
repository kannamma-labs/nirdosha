//! Animated nirdosha logo, drawable into any ratatui `Buffer`/`Rect`.
//!
//! Three phases, driven purely by `elapsed` seconds since the animation
//! started (no internal timers -- the caller's render loop owns time):
//!
//!   1. Reveal  -- the glyph strokes wipe in on a diagonal, with a short
//!      bright "catch the light" flash riding the reveal edge.
//!   2. Type    -- "NIRDOSHA" types in letter by letter under the glyph,
//!      each letter flashing white-hot for an instant before settling
//!      into brand navy; the subtitle fades in after.
//!   3. Idle    -- everything is static except the orange glyph accent,
//!      which breathes (slow sine pulse) forever, like a status glow.
//!
//! Copied verbatim (only the `logo_pixels` import path changed to
//! `super::logo_pixels`, since this now lives as a sibling child module
//! of `hi_tui`'s `logo_pixels` rather than at its own crate root) from
//! `crates/tui-logo-splash/src/logo_anim.rs`, which stays the canonical,
//! independently-previewable copy (`cargo run --release -p
//! nirdosha-tui-logo-splash`) -- port any future change there over here
//! too, or vice versa.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::logo_pixels::{LOGO, LOGO_H, LOGO_W};

const REVEAL_SECS: f32 = 0.95;
const SHIMMER_TRAIL: f32 = 0.16;

const WORD: &str = "NIRDOSHA";
const SUBTITLE: &str = "PROGRAMMING LANGUAGE";
const TYPE_PER_CHAR: f32 = 0.045;
const TYPE_FLASH: f32 = 0.12;
const WORD_TO_SUB_GAP: f32 = 0.22;
const SUB_FADE_SECS: f32 = 0.55;

const IDLE_PERIOD: f32 = 2.6;
const IDLE_BOOST: f32 = 0.4;

const WORD_FG: (u8, u8, u8) = (86, 76, 190); // brand navy, lifted a couple stops for legibility as bold text on dark terminals
const SUB_FG: (u8, u8, u8) = (120, 118, 145); // muted navy-gray
const FLASH: (u8, u8, u8) = (255, 255, 255);
const ACCENT_HOT: (u8, u8, u8) = (255, 214, 140); // brightened orange for the idle breathe

/// Seconds after which the whole intro (reveal + type + subtitle fade)
/// has settled and only the idle breathe is left running.
pub fn intro_done_at() -> f32 {
    word_start() + WORD.len() as f32 * TYPE_PER_CHAR + WORD_TO_SUB_GAP + SUB_FADE_SECS
}

fn word_start() -> f32 {
    REVEAL_SECS
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8
}

fn lerp(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        lerp_u8(a.0, b.0, t),
        lerp_u8(a.1, b.1, t),
        lerp_u8(a.2, b.2, t),
    )
}

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// How "orange accent" a sampled logo pixel is, 0.0 (navy) .. 1.0 (orange).
/// The brand navy is blue-dominant (b > r); the accent orange is
/// red-dominant (r > b) -- cheap enough to not need the source hue.
fn accent_amount(c: (u8, u8, u8)) -> f32 {
    ((c.0 as f32 - c.2 as f32) / 150.0).clamp(0.0, 1.0)
}

/// Render the animated logo, centered in `area`, at `elapsed` seconds
/// since the animation began.
pub fn render(buf: &mut Buffer, area: Rect, elapsed: f32) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let icon_w = LOGO_W as u16;
    let icon_h = LOGO_H as u16;
    let block_h = icon_h + 2 /* gap */ + 2 /* word + subtitle rows */;

    let ox = area.x + area.width.saturating_sub(icon_w) / 2;
    let oy = area.y + area.height.saturating_sub(block_h) / 2;

    draw_icon(buf, area, ox, oy, elapsed);
    draw_word(buf, area, ox, oy + icon_h + 1, icon_w, elapsed);
}

fn idle_breathe(elapsed: f32) -> f32 {
    let done = intro_done_at();
    if elapsed <= done {
        return 0.0;
    }
    let t = elapsed - done;
    let phase = (t / IDLE_PERIOD) * std::f32::consts::TAU;
    (phase.sin() * 0.5 + 0.5) * IDLE_BOOST
}

fn draw_icon(buf: &mut Buffer, clip: Rect, ox: u16, oy: u16, elapsed: f32) {
    let breathe = idle_breathe(elapsed);

    for (row, cells) in LOGO.iter().enumerate() {
        let y = oy + row as u16;
        if y < clip.y || y >= clip.y + clip.height {
            continue;
        }
        for (col, &(top, bot)) in cells.iter().enumerate() {
            if top.is_none() && bot.is_none() {
                continue;
            }
            let x = ox + col as u16;
            if x < clip.x || x >= clip.x + clip.width {
                continue;
            }

            // Diagonal reveal front: cells further along the
            // top-left -> bottom-right diagonal appear later.
            let diag = (col as f32 / LOGO_W as f32 + row as f32 / LOGO_H as f32) / 2.0;
            let reveal_at = diag * REVEAL_SECS;
            if elapsed < reveal_at {
                continue;
            }
            let age = elapsed - reveal_at;
            let shimmer = if age < SHIMMER_TRAIL {
                1.0 - age / SHIMMER_TRAIL
            } else {
                0.0
            };

            let paint = |c: (u8, u8, u8)| -> Color {
                let mut c = lerp(c, FLASH, shimmer * 0.85);
                let a = accent_amount(c);
                if a > 0.0 && breathe > 0.0 {
                    c = lerp(c, ACCENT_HOT, breathe * a);
                }
                rgb(c)
            };

            let cell = buf.cell_mut((x, y));
            let Some(cell) = cell else { continue };
            match (top, bot) {
                (Some(t), Some(b)) => {
                    cell.set_char('▀');
                    cell.set_fg(paint(t));
                    cell.set_bg(paint(b));
                }
                (Some(t), None) => {
                    cell.set_char('▀');
                    cell.set_fg(paint(t));
                }
                (None, Some(b)) => {
                    cell.set_char('▄');
                    cell.set_fg(paint(b));
                }
                (None, None) => unreachable!(),
            }
        }
    }
}

fn draw_word(buf: &mut Buffer, clip: Rect, ox: u16, oy: u16, width: u16, elapsed: f32) {
    let word_chars: Vec<char> = WORD.chars().collect();
    let word_w = word_chars.len() as u16;
    let wx0 = ox + width.saturating_sub(word_w) / 2;
    let wy = oy;

    if wy >= clip.y && wy < clip.y + clip.height {
        for (i, ch) in word_chars.iter().enumerate() {
            let t0 = word_start() + i as f32 * TYPE_PER_CHAR;
            if elapsed < t0 {
                continue;
            }
            let age = elapsed - t0;
            let flash = if age < TYPE_FLASH {
                1.0 - age / TYPE_FLASH
            } else {
                0.0
            };
            let color = rgb(lerp(WORD_FG, FLASH, flash * 0.9));
            let x = wx0 + i as u16;
            if x < clip.x || x >= clip.x + clip.width {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, wy)) {
                cell.set_char(*ch);
                cell.set_style(Style::default().fg(color).add_modifier(Modifier::BOLD));
            }
        }
    }

    let sub_chars: Vec<char> = SUBTITLE.chars().collect();
    let sub_w = sub_chars.len() as u16;
    let sx0 = ox + width.saturating_sub(sub_w) / 2;
    let sy = oy + 1;
    if sy < clip.y || sy >= clip.y + clip.height {
        return;
    }

    let sub_start = word_start() + word_w as f32 * TYPE_PER_CHAR + WORD_TO_SUB_GAP;
    if elapsed < sub_start {
        return;
    }
    let t = ((elapsed - sub_start) / SUB_FADE_SECS).clamp(0.0, 1.0);
    // Fades up from a faint ghost of itself rather than popping in.
    let color = rgb(lerp((SUB_FG.0 / 3, SUB_FG.1 / 3, SUB_FG.2 / 3), SUB_FG, t));
    for (i, ch) in sub_chars.iter().enumerate() {
        if *ch == ' ' {
            continue;
        }
        let x = sx0 + i as u16;
        if x < clip.x || x >= clip.x + clip.width {
            continue;
        }
        if let Some(cell) = buf.cell_mut((x, sy)) {
            cell.set_char(*ch);
            cell.set_fg(color);
        }
    }
}
