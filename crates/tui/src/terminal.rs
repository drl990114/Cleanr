use std::{
    io,
    path::PathBuf,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use cleanr_config::{Config, default_config_path};
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

#[derive(Debug, Clone)]
pub struct UpdateNotice {
    pub version: String,
    pub release_url: String,
}

/// Optional background services. Existing TUI entry points remain usable without them.
#[derive(Default)]
pub struct TuiServices {
    pub update_notice_rx: Option<Receiver<UpdateNotice>>,
}

pub fn run(options: TuiOptions) -> Result<()> {
    run_with_inactivity_override(options, None)
}

pub fn run_with_inactivity_override(options: TuiOptions, inactive_days: Option<u16>) -> Result<()> {
    run_with_config_path_and_inactivity_override(options, None, inactive_days)
}

#[doc(hidden)]
pub fn run_with_config_path_and_inactivity_override(
    options: TuiOptions,
    config_path: Option<PathBuf>,
    inactive_days: Option<u16>,
) -> Result<()> {
    run_with_services(options, config_path, inactive_days, TuiServices::default())
}

pub fn run_with_services(
    options: TuiOptions,
    config_path: Option<PathBuf>,
    inactive_days: Option<u16>,
    services: TuiServices,
) -> Result<()> {
    const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
    const IDLE_WAKE_INTERVAL: Duration = Duration::from_secs(1);

    if let Some(inactive_days) = inactive_days {
        RecommendationPolicy::new(inactive_days)?;
    }
    let (registry, i18n) = load_runtime(&options.config)?;
    let theme = resolve_theme(options.config.ui.theme);
    let mut app = Workbench::new_with_config_path(
        options.roots,
        options.config,
        config_path.or_else(default_config_path),
        registry,
        i18n,
        theme,
    );
    app.session_inactive_days = inactive_days;
    app.update_notice = options.update_available;
    app.update_notice_rx = services.update_notice_rx;

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
    let mut pending_event = None;
    let mut input_times = Vec::with_capacity(32);
    loop {
        let commit_started = Instant::now();
        let committed = app.poll_tasks();
        if committed {
            app.task_commit_durations.record(commit_started.elapsed());
        }
        redraw |= committed;
        if app.has_background_task() && last_animation.elapsed() >= ANIMATION_INTERVAL {
            redraw |= app.advance_animation();
            last_animation = Instant::now();
        }

        if redraw {
            let draw_started_at = Instant::now();
            let area = terminal.draw(|frame| render(frame, &mut app))?.area;
            app.record_frame_duration(draw_started_at.elapsed());
            for started in input_times.drain(..) {
                app.input_to_frame_durations
                    .record(Instant::now().saturating_duration_since(started));
            }
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
        if pending_event.is_none() && !event::poll(timeout)? {
            continue;
        }
        let batch_started = Instant::now();
        for index in 0..32 {
            let (event, read_at) = match pending_event.take() {
                Some(event) => event,
                None => (event::read()?, Instant::now()),
            };
            let started = Instant::now();
            let navigation = matches!(&event, Event::Key(key) if app.can_batch_navigation(*key));
            let changed = match event {
                Event::Key(key) => app.handle_key_changed(key),
                Event::Paste(value) => {
                    let before = app.ui_stamp();
                    app.handle_paste(&value);
                    app.ui_stamp() != before
                }
                Event::Resize(_, _) => true,
                _ => false,
            };
            if changed {
                app.record_input_duration(started.elapsed());
                input_times.push(read_at);
                redraw = true;
            }
            if !navigation
                || index == 31
                || batch_started.elapsed() >= Duration::from_millis(4)
                || !event::poll(Duration::ZERO)?
            {
                break;
            }
            let next = event::read()?;
            let read_at = Instant::now();
            let next_is_navigation =
                matches!(&next, Event::Key(key) if app.can_batch_navigation(*key));
            pending_event = Some((next, read_at));
            if !next_is_navigation {
                break;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Explicit manual PTY fixture: all candidate data is generated in a temporary directory.
    #[test]
    #[ignore = "requires an interactive PTY; exits with q"]
    fn interactive_terminal_fixture() {
        let temp = tempfile::tempdir().expect("fixture");
        let mut config = Config::default();
        config.plugins.dirs.clear();
        config.i18n.dirs.clear();
        config.ui.theme = cleanr_config::UiTheme::Dark;
        for i in 0..8 {
            let project = temp.path().join(format!("project-{i}"));
            std::fs::create_dir_all(project.join("node_modules")).unwrap();
            std::fs::write(project.join("package.json"), b"{}").unwrap();
            std::fs::write(
                project.join("node_modules/cache.bin"),
                vec![0; 2 * 1024 * 1024],
            )
            .unwrap();
        }
        // An intentionally pending update receiver verifies that startup does not wait for it.
        let (_sender, receiver) = std::sync::mpsc::channel();
        run_with_services(
            TuiOptions {
                roots: vec![temp.path().into()],
                config,
                update_available: None,
            },
            Some(temp.path().join("config.toml")),
            Some(0),
            TuiServices {
                update_notice_rx: Some(receiver),
            },
        )
        .unwrap();
    }
}
