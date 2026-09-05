#![forbid(unsafe_code)]

mod app;
mod commands;
mod effects;
mod projection;
mod terminal;
mod theme;
mod views;

pub use app::Workbench;
pub use terminal::{
    TuiOptions, TuiServices, UpdateNotice, run, run_with_config_path_and_inactivity_override,
    run_with_inactivity_override, run_with_services,
};
pub use theme::Theme;

#[cfg(test)]
mod tests;
