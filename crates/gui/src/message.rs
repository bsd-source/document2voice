//! Messages the window can send itself (clicks, keys, job finished, and so on).

use crate::focus::FocusSlot;
use crate::types::{BitrateUi, DocLoadOutcome, FormatUi, SampleRateUi, VoiceUi};
use document2voice_core::Language;
use iced::widget::text_editor;
use iced::window;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    LanguageSelected(Language),
    VoiceSelected(VoiceUi),
    SpeedChanged(f32),
    PitchChanged(f32),
    StyleChanged(f32),
    PauseChanged(f32),
    LoudnessChanged(f32),
    FormatSelected(FormatUi),
    BitrateSelected(BitrateUi),
    SampleRateSelected(SampleRateUi),
    OutPathChanged(String),
    SourcePathChanged(String),
    PageFromChanged(String),
    PageToChanged(String),
    PageSkipChanged(String),
    BrowseOut,
    OpenFile,
    DocLoaded(DocLoadOutcome),
    ResetDefaults,
    /// "Split by chapter" checkbox.
    SplitChaptersToggled(bool),
    Speak,
    /// Build about 10 seconds of speech and play it (from the book, or a fixed sample if Source is empty).
    Preview,
    CancelSave,
    TickProgress,
    /// Saved path + optional warning, or an error message.
    JobFinished(Result<(PathBuf, Option<String>), String>),
    PreviewFinished(Result<(), String>),
    /// Startup tool probe finished (one banner entry per tool).
    ToolsProbed(Vec<String>),
    /// Path chosen in the save dialog (`None` means the user cancelled).
    OutPathPicked(Option<String>),
    /// Replace-dialog answer: `true` = replace file, `false` = cancel Start.
    OverwriteConfirmed(bool),
    /// Unfinished-export dialog: `Some(true)` resume, `Some(false)` start over, `None` cancel.
    ResumeChosen(Option<bool>),
    /// Tab / Shift+Tab to move between controls.
    FocusMove(bool),
    /// Enter or Space on the focused control (activate it).
    FocusActivate,
    /// Arrow keys on a focused slider or dropdown. `+1` or `-1`.
    FocusNudge(i32),
    /// Mouse clicked a control — move keyboard focus there.
    FocusTake(FocusSlot),
    /// Left-click somewhere — re-check which text field iced focused.
    PointerPressed,
    /// Select / scroll / copy in the Log (edits are ignored).
    LogAction(text_editor::Action),
    /// Ctrl+C: copy the selected Log text.
    LogCopy,
    /// Right-click the Log: copy everything in it.
    LogCopyAll,
    /// After a click: which path/page text box iced says is focused (`None` = none of them).
    PageTextFocus(Option<FocusSlot>),
    WindowClosed(window::Id),
}
