use std::{collections::BTreeSet, ops::Range, path::PathBuf};

#[cfg(test)]
use cleanr_core::ScanEntry;
use cleanr_core::{CleanupItem, Confidence, EntryKind};
use cleanr_i18n::LanguagePackSource;
use cleanr_tasks::restored_run_ids;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, HighlightSpacing, List, ListItem, ListState,
        Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
        TableState, Wrap,
    },
};
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{CandidateCategory, CategoryKey, ConfirmChoice, Mode, View, Workbench},
    effects::{ScanStage, ScanTaskProgress},
    theme::Theme,
};

// -------------------------------------------------------------------------

mod chrome;
mod cleanup;
mod context;
mod helpers;
mod home;
mod restore;
mod root;
mod scan;
mod usage;

use chrome::*;
use cleanup::*;
use context::*;
pub(crate) use helpers::*;
use home::*;
use restore::*;
pub(crate) use root::render;
#[cfg(test)]
pub(crate) use scan::scan_loading_indicator_sample;
use scan::*;
use usage::render_usage;
#[cfg(test)]
pub(crate) use usage::usage_descendant_count;
