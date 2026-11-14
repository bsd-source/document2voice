//! Which control the keyboard is on, and helpers for moving that focus.

use crate::Message;
use iced::widget::{self as widget};
use iced::Task;

/// Widget ids of the text inputs (shared by the view and focus operations).
pub(crate) const PAGE_FROM_ID: &str = "page_from";
pub(crate) const PAGE_TO_ID: &str = "page_to";
pub(crate) const PAGE_SKIP_ID: &str = "page_skip";
pub(crate) const SOURCE_PATH_ID: &str = "source_path";
pub(crate) const OUT_PATH_ID: &str = "out_path";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusSlot {
    /// Nothing on the form is focused yet — only the window (normal at startup).
    None,
    // Speech settings (Kokoro voice engine)
    Language,
    Voice,
    Speed,
    Style,
    Pause,
    PageFrom,
    PageTo,
    PageSkip,
    // Audio export settings
    Format,
    /// "Split by chapter" checkbox (next to Format).
    SplitChapters,
    Bitrate,
    Pitch,
    Loudness,
    SampleRate,
    /// Files card (on screen: Source row first, then Output row).
    SourcePath,
    OpenFile,
    OutPath,
    Browse,
    Defaults,
    /// Short ~10 second “try this voice” preview.
    Preview,
    /// Main action button (Start).
    Save,
}

impl FocusSlot {
    /// Tab order: top to bottom, left to right, matching the main form.
    /// (`None` is not in the cycle; first Tab from `None` jumps to the first control.)
    pub(crate) const ORDER: &'static [FocusSlot] = &[
        // Speech section
        Self::Language,
        Self::Voice,
        Self::Speed,
        Self::Style,
        Self::Pause,
        Self::PageFrom,
        Self::PageTo,
        Self::PageSkip,
        // Audio section (same order as the Audio card on screen)
        Self::Format,
        Self::SplitChapters,
        Self::Bitrate,
        Self::SampleRate,
        Self::Pitch,
        Self::Loudness,
        // Files section: Source … then Output …
        Self::SourcePath,
        Self::OpenFile,
        Self::OutPath,
        Self::Browse,
        // Bottom action buttons
        Self::Defaults,
        Self::Preview,
        Self::Save,
    ];

    /// Pages From / To / Skip text boxes.
    pub(crate) fn is_page(self) -> bool {
        matches!(self, Self::PageFrom | Self::PageTo | Self::PageSkip)
    }

    pub(crate) fn is_active(self, bitrate_enabled: bool, pages_enabled: bool) -> bool {
        match self {
            Self::None => false,
            Self::Bitrate => bitrate_enabled,
            // Pages / Skip only work after a Source file is chosen.
            s if s.is_page() => pages_enabled,
            // Defaults stays in the Tab order even when greyed out (nothing to reset).
            _ => true,
        }
    }

    pub(crate) fn next(self, forward: bool, bitrate_enabled: bool, pages_enabled: bool) -> Self {
        let list: Vec<FocusSlot> = Self::ORDER
            .iter()
            .copied()
            .filter(|s| s.is_active(bitrate_enabled, pages_enabled))
            .collect();
        if list.is_empty() {
            return Self::None;
        }
        // First Tab/Shift+Tab when nothing on the form is focused yet.
        if self == Self::None {
            return if forward {
                list[0]
            } else {
                list[list.len() - 1]
            };
        }
        let idx = list.iter().position(|&s| s == self).unwrap_or(0);
        if forward {
            list[(idx + 1) % list.len()]
        } else {
            list[(idx + list.len() - 1) % list.len()]
        }
    }

    pub(crate) fn text_id(self) -> Option<widget::Id> {
        let id = match self {
            Self::PageFrom => PAGE_FROM_ID,
            Self::PageTo => PAGE_TO_ID,
            Self::PageSkip => PAGE_SKIP_ID,
            Self::SourcePath => SOURCE_PATH_ID,
            Self::OutPath => OUT_PATH_ID,
            _ => return None,
        };
        Some(widget::Id::new(id))
    }
}

pub(crate) fn unfocus_text_inputs() -> Task<Message> {
    // Point focus at a fake id so every real text box loses focus.
    widget::operation::focus(widget::Id::new("__unfocus__"))
}

pub(crate) fn apply_focus_slot(slot: FocusSlot) -> Task<Message> {
    if let Some(id) = slot.text_id() {
        widget::operation::focus(id)
    } else {
        unfocus_text_inputs()
    }
}
