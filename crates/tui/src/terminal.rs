use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use cleanr_config::Config;
use cleanr_core::RecommendationPolicy;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{
        Clear as TerminalClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::{Mode, Workbench},
    effects::load_runtime,
    theme::resolve_theme,
    views::{ime_guard_position, render},
};

pub struct TuiOptions {
    pub roots: Vec<PathBuf>,
    pub config: Config,
    pub update_available: Option<UpdateNotice>,
}

pub struct UpdateNotice {
    pub version: String,
    pub release_url: String,
}

pub fn run(options: TuiOptions) -> Result<()> {
    run_with_inactivity_override(options, None)
}

pub fn run_with_inactivity_override(options: TuiOptions, inactive_days: Option<u16>) -> Result<()> {
    const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
    const IDLE_WAKE_INTERVAL: Duration = Duration::from_secs(1);

    if let Some(inactive_days) = inactive_days {
        RecommendationPolicy::new(inactive_days)?;
    }
    let (registry, i18n) = load_runtime(&options.config)?;
    let theme = resolve_theme(options.config.ui.theme);
    let mut app = Workbench::new(options.roots, options.config, registry, i18n, theme);
    app.session_inactive_days = inactive_days;
    if let Some(update) = options.update_available {
        app.status = app.i18n.format(
            "status_update_available",
            &[("version", update.version), ("url", update.release_url)],
        );
    }

    enable_raw_mode().context("failed to enable raw mode")?;
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        TerminalClear(ClearType::All),
        MoveTo(0, 0),
        EnableBracketedPaste,
        Hide
    )
    .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;
    terminal
        .clear()
        .context("failed to clear terminal before rendering")?;

    let mut redraw = true;
    let mut last_animation = Instant::now();
    loop {
        redraw |= app.poll_tasks();
        if app.has_background_task() && last_animation.elapsed() >= ANIMATION_INTERVAL {
            redraw |= app.advance_animation();
            last_animation = Instant::now();
        }

        if redraw {
            let draw_started_at = Instant::now();
            let area = terminal.draw(|frame| render(frame, &mut app))?.area;
            app.record_frame_duration(draw_started_at.elapsed());
            if matches!(app.mode, Mode::Normal) {
                terminal.set_cursor_position(ime_guard_position(area))?;
                terminal.hide_cursor()?;
            }
            redraw = false;
        }

        if app.should_quit {
            break;
        }

        let timeout = if app.has_background_task() {
            ANIMATION_INTERVAL.saturating_sub(last_animation.elapsed())
        } else {
            IDLE_WAKE_INTERVAL
        };
        if !event::poll(timeout)? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => {
                let input_started_at = Instant::now();
                app.handle_key(key);
                app.record_input_duration(input_started_at.elapsed());
                redraw = true;
            }
            Event::Paste(value) => {
                let input_started_at = Instant::now();
                app.handle_paste(&value);
                app.record_input_duration(input_started_at.elapsed());
                redraw = true;
            }
            Event::Resize(_, _) => redraw = true,
            _ => {}
        }
    }

    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen
        );
    }
}
