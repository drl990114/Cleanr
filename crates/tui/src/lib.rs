#![forbid(unsafe_code)]

mod app;
mod commands;
mod effects;
mod terminal;
mod theme;
mod views;

pub use app::Workbench;
pub use terminal::{TuiOptions, UpdateNotice, run, run_with_inactivity_override};
pub use theme::Theme;

#[cfg(test)]
mod tests;
