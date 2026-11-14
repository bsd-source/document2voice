//! Main window layout (what you see on screen).

use crate::focus::{
    FocusSlot, OUT_PATH_ID, PAGE_FROM_ID, PAGE_SKIP_ID, PAGE_TO_ID, SOURCE_PATH_ID,
};
use crate::log_hl::format_count;
use crate::log_hl::{log_editor_style, log_hl_format, LogHighlighter};
use crate::style::{
    card_style, dialog_cancel_button, dialog_card_style, dialog_danger_button,
    dialog_path_chip_style, dialog_scrim_style, disabled_control_bg, disabled_control_border,
    ellipsis_btn, field_btn, focus_accent, muted_text, page_style, pick_list_style,
    pick_menu_style, primary_button, primary_button_style, progress_bar_style, slider_style,
    text_field_style, tip_style, unavailable_button, MAIN_W,
};
use crate::types::{BitrateUi, FormatUi, SampleRateUi, StartPrompt};
use crate::{
    App, Message, LOUD_MAX, LOUD_OFF, PAUSE_MAX, PAUSE_MIN, PITCH_MAX, PITCH_MIN, SPEED_MAX,
    SPEED_MIN, STYLE_MAX, STYLE_MIN, TOOLTIP_DELAY,
};
use document2voice_core::Language;
use iced::border::Border;
use iced::keyboard::Key;
use iced::widget::tooltip::Position as TipPos;
use iced::widget::{
    self as widget, button, checkbox, column, container, mouse_area, pick_list, progress_bar, row,
    slider, stack, text, text_editor, text_input, tooltip, Space,
};
use iced::window;
use iced::{Alignment, Background, Color, Element, Fill, Font, Length, Shadow, Theme};

/// Compact dropdowns / edit fields (shorter box + slightly smaller type).
const FIELD_TEXT: f32 = 12.0;
const VALUE_W: u32 = 48;

const BITRATE_TIP: &str = "Quality for Opus, MP3 and M4B. Higher kbps sounds better but makes a larger file. Not used for WAV or FLAC.";

fn field_pad() -> iced::Padding {
    iced::Padding {
        top: 3.0,
        right: 7.0,
        bottom: 3.0,
        left: 7.0,
    }
}

/// Last path component for dialog text (falls back to the whole string).
fn file_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Hover tip on a setting label (tip on the name, not the control).
/// Only the outer bubble uses `tip_style`; styling the content too draws a double border.
fn tip_label(s: &'static str, tip: &'static str) -> Element<'static, Message> {
    tooltip(
        text(s).size(11).color(muted_text()).width(84),
        container(text(tip).size(11).color(Color::from_rgb8(0xF6, 0xF5, 0xF4)))
            .padding([6, 10])
            .max_width(300),
        TipPos::Bottom,
    )
    .gap(4)
    .delay(TOOLTIP_DELAY)
    .style(tip_style)
    .into()
}

fn section_title(s: &'static str) -> text::Text<'static> {
    text(s).size(12).color(muted_text())
}

impl App {
    pub(crate) fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let f = self.focus;
        let body = column![
            self.view_speech_card(f),
            self.view_audio_card(f),
            self.view_files_card(f),
            self.view_log_card(),
            self.view_progress_row(),
            self.view_action_bar(f),
        ]
        .spacing(8)
        .padding(iced::Padding {
            top: 10.0,
            right: 12.0,
            bottom: 10.0,
            left: 12.0,
        })
        .width(Fill)
        .height(Fill);

        let page = container(body).width(Fill).height(Fill).style(page_style);

        // In-app replace / resume dialog drawn over the page.
        if let Some(prompt) = self.start_prompt.as_ref() {
            stack![page, self.view_start_prompt(prompt)].into()
        } else {
            page.into()
        }
    }

    /// Dimmed full-window overlay asking how Start should proceed (Adwaita-style).
    fn view_start_prompt(&self, prompt: &StartPrompt) -> Element<'_, Message> {
        let (path, title, subtitle, note) = match prompt {
            StartPrompt::ReplaceFile(p) => (
                p.clone(),
                "Replace existing file?".to_string(),
                format!("“{}” is already on disk.", file_label(p)),
                "Replacing permanently overwrites that file.".to_string(),
            ),
            StartPrompt::ReplaceFolder(p) => (
                p.clone(),
                "Replace chapter files?".to_string(),
                format!("The folder “{}” already has files.", file_label(p)),
                "Numbered chapter files (“01 - …”) of this format are replaced; \
                 other files in the folder are left alone."
                    .to_string(),
            ),
            StartPrompt::Resume { path, done, total } => (
                path.clone(),
                "Continue the unfinished export?".to_string(),
                format!(
                    "{} of {} paragraphs were already made for “{}”.",
                    format_count(*done),
                    format_count(*total),
                    file_label(path)
                ),
                "Resume continues where it stopped (if the text and voice settings are \
                 unchanged). Start over discards that progress."
                    .to_string(),
            ),
        };

        // Dialog width: sit comfortably inside the 443px window with side margin.
        let dialog_w = (MAIN_W - 48.0).clamp(300.0, 380.0);

        let path_chip = container(
            text(path)
                .size(11)
                .font(Font::MONOSPACE)
                .color(Color::from_rgb8(0x3D, 0x38, 0x40)),
        )
        .width(Fill)
        .padding([10, 12])
        .style(dialog_path_chip_style);

        let heading = column![
            text(title).size(15),
            text(subtitle).size(12).color(muted_text()),
        ]
        .spacing(4);

        // Kind: 0 = secondary, 1 = destructive (red), 2 = safe primary (blue).
        let btn = |label: &'static str, msg: Message, kind: u8| {
            button(text(label).size(12))
                .padding([8, 18])
                .on_press(msg)
                .style(match kind {
                    1 => dialog_danger_button,
                    2 => primary_button,
                    _ => dialog_cancel_button,
                })
        };
        // GNOME-style: secondary left, primary right, flush to the end.
        let actions = match prompt {
            StartPrompt::Resume { .. } => row![
                Space::new().width(Fill),
                btn("Cancel", Message::ResumeChosen(None), 0),
                Space::new().width(8),
                btn("Start over", Message::ResumeChosen(Some(false)), 1),
                Space::new().width(8),
                btn("Resume", Message::ResumeChosen(Some(true)), 2),
            ],
            _ => row![
                Space::new().width(Fill),
                btn("Cancel", Message::OverwriteConfirmed(false), 0),
                Space::new().width(8),
                btn("Replace", Message::OverwriteConfirmed(true), 1),
            ],
        }
        .align_y(Alignment::Center)
        .width(Fill);

        let card = container(
            column![
                heading,
                path_chip,
                text(note).size(11).color(muted_text()),
                actions,
            ]
            .spacing(14)
            .padding(iced::Padding {
                top: 18.0,
                right: 18.0,
                bottom: 16.0,
                left: 18.0,
            })
            .width(Length::Fixed(dialog_w)),
        )
        .style(dialog_card_style);

        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: 16.0,
                bottom: 0.0,
                left: 16.0,
            })
            .style(dialog_scrim_style)
            .into()
    }

    /// Speech: Kokoro voice + document range (how it is spoken / what text).
    fn view_speech_card(&self, f: FocusSlot) -> Element<'_, Message> {
        container(
            column![
                section_title("Speech"),
                self.speech_language_row(f),
                self.speech_voice_row(f),
                self.speech_speed_row(f),
                self.speech_style_row(f),
                self.speech_pause_row(f),
                self.speech_pages_row(f),
                self.speech_skip_row(f),
            ]
            .spacing(6),
        )
        .padding(10)
        .width(Fill)
        .style(card_style)
        .into()
    }

    fn speech_language_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let language_pick = mouse_area(
            pick_list(
                Language::ALL.to_vec(),
                Some(self.language),
                Message::LanguageSelected,
            )
            .padding(field_pad())
            .text_size(FIELD_TEXT)
            .width(Fill)
            .style(pick_list_style(f == FocusSlot::Language))
            .menu_style(pick_menu_style),
        )
        .on_press(Message::FocusTake(FocusSlot::Language));
        row![
            tip_label("Language", "Which language the voice should speak."),
            language_pick
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_voice_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let voice_pick = mouse_area(
            pick_list(
                self.voices.clone(),
                self.selected_voice(),
                Message::VoiceSelected,
            )
            .padding(field_pad())
            .text_size(FIELD_TEXT)
            .width(Fill)
            .style(pick_list_style(f == FocusSlot::Voice))
            .menu_style(pick_menu_style),
        )
        .on_press(Message::FocusTake(FocusSlot::Voice));
        row![
            tip_label("Voice", "Which speaker reads the book."),
            voice_pick
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_speed_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let speed_slider = mouse_area(
            slider(SPEED_MIN..=SPEED_MAX, self.speed, Message::SpeedChanged)
                .step(0.01_f32)
                .width(Fill)
                .style(slider_style(f == FocusSlot::Speed)),
        )
        .on_press(Message::FocusTake(FocusSlot::Speed));
        row![
            tip_label("Speed", "How fast the voice speaks. 1.00× is normal."),
            text(format!("{:.2}×", self.speed))
                .size(11)
                .width(VALUE_W)
                .color(muted_text()),
            speed_slider,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_style_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let style_label = if self.style < 0.5 {
            "auto".to_string()
        } else {
            format!("{}", self.style.round() as i32)
        };
        let style_slider = mouse_area(
            slider(STYLE_MIN..=STYLE_MAX, self.style, Message::StyleChanged)
                .step(1.0_f32)
                .width(Fill)
                .style(slider_style(f == FocusSlot::Style)),
        )
        .on_press(Message::FocusTake(FocusSlot::Style));
        row![
            tip_label(
                "Style",
                "Slightly changes voice tone and rhythm (how expressive it sounds)."
            ),
            text(style_label)
                .size(11)
                .width(VALUE_W)
                .color(muted_text()),
            style_slider,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_pause_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let pause_slider = mouse_area(
            slider(
                PAUSE_MIN..=PAUSE_MAX,
                self.paragraph_pause,
                Message::PauseChanged,
            )
            .step(0.05_f32)
            .width(Fill)
            .style(slider_style(f == FocusSlot::Pause)),
        )
        .on_press(Message::FocusTake(FocusSlot::Pause));
        row![
            tip_label(
                "Pause",
                "How long to wait in silence between paragraphs (in seconds)."
            ),
            text(format!("{:.2}s", self.paragraph_pause))
                .size(11)
                .width(VALUE_W)
                .color(muted_text()),
            pause_slider,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_pages_row(&self, f: FocusSlot) -> Element<'_, Message> {
        row![
            tip_label(
                "Pages",
                "Page range to include (blank=all). Accurate for PDF files. Other formats are unpredictable and use ~250-word chunks, not real pages. For accurate page range, convert to PDF first and use that file instead. Otherwise it's trial and error."
            ),
            self.page_from_input(f),
            text("–").size(13).color(muted_text()),
            self.page_to_input(f),
            text("optional").size(11).color(muted_text()),
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn speech_skip_row(&self, f: FocusSlot) -> Element<'_, Message> {
        row![
            tip_label(
                "Skip pages",
                "Pages to leave out, e.g. 5, 8, 90-103. Must be inside the Pages range. Accurate for PDF; other formats are approximate — convert to PDF first for reliable skips."
            ),
            self.page_skip_input(f),
            text("optional").size(11).color(muted_text()),
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    /// Pages / Skip stay greyed until Source has a path.
    fn page_from_input(&self, f: FocusSlot) -> Element<'_, Message> {
        let pages_on = self.pages_fields_enabled();
        let pages_err = self.pages_error.as_ref();
        let mut input = text_input("", &self.page_from)
            .id(widget::Id::new(PAGE_FROM_ID))
            .padding(field_pad())
            .size(FIELD_TEXT)
            .width(64)
            .style(text_field_style(
                pages_on && f == FocusSlot::PageFrom,
                pages_on && pages_err.is_some_and(|e| e.from),
                pages_on,
            ));
        if pages_on {
            input = input.on_input(Message::PageFromChanged);
            mouse_area(input)
                .on_press(Message::FocusTake(FocusSlot::PageFrom))
                .into()
        } else {
            input.into()
        }
    }

    fn page_to_input(&self, f: FocusSlot) -> Element<'_, Message> {
        let pages_on = self.pages_fields_enabled();
        let pages_err = self.pages_error.as_ref();
        let mut input = text_input("", &self.page_to)
            .id(widget::Id::new(PAGE_TO_ID))
            .padding(field_pad())
            .size(FIELD_TEXT)
            .width(64)
            .style(text_field_style(
                pages_on && f == FocusSlot::PageTo,
                pages_on && pages_err.is_some_and(|e| e.to),
                pages_on,
            ));
        if pages_on {
            input = input.on_input(Message::PageToChanged);
            mouse_area(input)
                .on_press(Message::FocusTake(FocusSlot::PageTo))
                .into()
        } else {
            input.into()
        }
    }

    // Same overall width as Pages From–To (64 + gap + dash + gap + 64).
    fn page_skip_input(&self, f: FocusSlot) -> Element<'_, Message> {
        let pages_on = self.pages_fields_enabled();
        let pages_err = self.pages_error.as_ref();
        let mut input = text_input("", &self.page_skip)
            .id(widget::Id::new(PAGE_SKIP_ID))
            .padding(field_pad())
            .size(FIELD_TEXT)
            .width(148)
            .style(text_field_style(
                pages_on && f == FocusSlot::PageSkip,
                pages_on && pages_err.is_some_and(|e| e.skip),
                pages_on,
            ));
        if pages_on {
            input = input.on_input(Message::PageSkipChanged);
            mouse_area(input)
                .on_press(Message::FocusTake(FocusSlot::PageSkip))
                .into()
        } else {
            input.into()
        }
    }

    /// Audio: format + bitrate (compressed) + sample rate + loudness/pitch.
    /// Export is always mono (Kokoro makes mono speech; matches other Kokoro apps).
    fn view_audio_card(&self, f: FocusSlot) -> Element<'_, Message> {
        container(
            column![
                section_title("Audio"),
                self.audio_format_row(f),
                self.audio_bitrate_row(f),
                self.audio_sample_rate_row(f),
                self.audio_pitch_row(f),
                self.audio_loudnorm_row(f),
            ]
            .spacing(6),
        )
        .padding(10)
        .width(Fill)
        .style(card_style)
        .into()
    }

    fn audio_format_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let format_pick = mouse_area(
            pick_list(
                FormatUi::ALL.to_vec(),
                Some(self.format),
                Message::FormatSelected,
            )
            .padding(field_pad())
            .text_size(FIELD_TEXT)
            .width(Fill)
            .style(pick_list_style(f == FocusSlot::Format))
            .menu_style(pick_menu_style),
        )
        .on_press(Message::FocusTake(FocusSlot::Format));
        let split_focused = f == FocusSlot::SplitChapters;
        let split = mouse_area(
            checkbox(self.split_chapters)
                .label("Split by chapter")
                .on_toggle(Message::SplitChaptersToggled)
                .size(14)
                .text_size(11)
                .spacing(5)
                .style(move |theme: &Theme, status| {
                    let mut st = checkbox::primary(theme, status);
                    if split_focused {
                        st.border.color = focus_accent();
                        st.border.width = 2.0;
                    }
                    st
                }),
        )
        .on_press(Message::FocusTake(FocusSlot::SplitChapters));
        let split = tooltip(
            split,
            container(
                text(
                    "One audio file per chapter, in a folder named after Output. \
                     Off: one file with chapter markers (Opus, MP3, M4B, FLAC).",
                )
                .size(11),
            )
            .padding(6)
            .max_width(260)
            .style(tip_style),
            TipPos::Bottom,
        )
        .delay(TOOLTIP_DELAY);
        row![
            tip_label(
                "Format",
                "Opus, MP3 and M4B are smaller, lossy formats; M4B is the audiobook format most players show chapters for. WAV and FLAC are larger, lossless formats. Chapters found in the book are added to every format except WAV."
            ),
            format_pick,
            split,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    /// Bitrate: active for Opus/MP3/M4B; greyed out for lossless WAV/FLAC.
    fn audio_bitrate_row(&self, f: FocusSlot) -> Element<'_, Message> {
        if self.format.supports_bitrate() {
            let bitrate_pick = mouse_area(
                pick_list(
                    BitrateUi::ALL.to_vec(),
                    Some(self.bitrate),
                    Message::BitrateSelected,
                )
                .padding(field_pad())
                .text_size(FIELD_TEXT)
                .width(Fill)
                .style(pick_list_style(f == FocusSlot::Bitrate))
                .menu_style(pick_menu_style),
            )
            .on_press(Message::FocusTake(FocusSlot::Bitrate));
            row![tip_label("Bitrate", BITRATE_TIP), bitrate_pick]
                .align_y(Alignment::Center)
                .spacing(6)
                .into()
        } else {
            // Empty greyed field — lossless formats have no bitrate.
            let disabled = container(Space::new().height(Length::Fixed(FIELD_TEXT + 6.0)))
                .padding(field_pad())
                .width(Fill)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Color(disabled_control_bg())),
                    text_color: None,
                    border: Border {
                        color: disabled_control_border(),
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    shadow: Shadow::default(),
                    snap: false,
                });
            row![tip_label("Bitrate", BITRATE_TIP), disabled]
                .align_y(Alignment::Center)
                .spacing(6)
                .into()
        }
    }

    fn audio_sample_rate_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let sample_rate_pick = mouse_area(
            pick_list(
                SampleRateUi::ALL.to_vec(),
                Some(self.sample_rate),
                Message::SampleRateSelected,
            )
            .padding(field_pad())
            .text_size(FIELD_TEXT)
            .width(Fill)
            .style(pick_list_style(f == FocusSlot::SampleRate))
            .menu_style(pick_menu_style),
        )
        .on_press(Message::FocusTake(FocusSlot::SampleRate));
        row![
            tip_label(
                "Sample rate",
                "24 kHz matches Kokoro. Raising kHz does not improve sound quality and will increase WAV/FLAC file sizes."
            ),
            sample_rate_pick
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn audio_pitch_row(&self, f: FocusSlot) -> Element<'_, Message> {
        let pitch_slider = mouse_area(
            slider(PITCH_MIN..=PITCH_MAX, self.pitch, Message::PitchChanged)
                .step(0.1_f32)
                .width(Fill)
                .style(slider_style(f == FocusSlot::Pitch)),
        )
        .on_press(Message::FocusTake(FocusSlot::Pitch));
        row![
            tip_label(
                "Pitch",
                "How high or low the voice sounds. 0 is normal; lower is deeper, higher is brighter."
            ),
            text(format!("{:+.1} st", self.pitch))
                .size(11)
                .width(VALUE_W)
                .color(muted_text()),
            pitch_slider,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    fn audio_loudnorm_row(&self, f: FocusSlot) -> Element<'_, Message> {
        // Leftmost stop = Off; move right for loudness target (−24 … −12 LUFS).
        let loud_label = if self.loudnorm {
            format!("{:.0} LUFS", self.loudness)
        } else {
            "off".into()
        };
        let loud_slider = mouse_area(
            slider(
                LOUD_OFF..=LOUD_MAX,
                self.loud_slider_pos(),
                Message::LoudnessChanged,
            )
            .step(0.5_f32)
            .width(Fill)
            .style(slider_style(f == FocusSlot::Loudness)),
        )
        .on_press(Message::FocusTake(FocusSlot::Loudness));
        row![
            tip_label(
                "Loudnorm",
                "Keeps volume even. Far left turns it off; −18 is a common spoken-word level."
            ),
            text(loud_label).size(11).width(VALUE_W).color(muted_text()),
            loud_slider,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .into()
    }

    /// Files: Source above Output.
    fn view_files_card(&self, f: FocusSlot) -> Element<'_, Message> {
        let out_invalid = self.out_error.is_some();
        let out_path = mouse_area(
            text_input("", &self.out_path)
                .id(widget::Id::new(OUT_PATH_ID))
                .on_input(Message::OutPathChanged)
                .padding(field_pad())
                .size(FIELD_TEXT)
                .width(Fill)
                .style(text_field_style(f == FocusSlot::OutPath, out_invalid, true)),
        )
        .on_press(Message::FocusTake(FocusSlot::OutPath));
        let browse_btn = ellipsis_btn(f == FocusSlot::Browse, Some(Message::BrowseOut));

        let source_invalid = self.source_error.is_some();
        let source_path_field = mouse_area(
            text_input("", &self.source_path)
                .id(widget::Id::new(SOURCE_PATH_ID))
                .on_input(Message::SourcePathChanged)
                .padding(field_pad())
                .size(FIELD_TEXT)
                .width(Fill)
                .style(text_field_style(
                    f == FocusSlot::SourcePath,
                    source_invalid,
                    true,
                )),
        )
        .on_press(Message::FocusTake(FocusSlot::SourcePath));
        let open_btn = ellipsis_btn(
            f == FocusSlot::OpenFile,
            if self.is_busy() {
                None
            } else {
                Some(Message::OpenFile)
            },
        );

        container(
            column![
                row![
                    tip_label(
                        "Source",
                        "Document to turn into speech: PDF, EPUB, TXT, MD, DOCX, ODT, MOBI, PRC, AZW, AZW3, KFX."
                    ),
                    source_path_field,
                    open_btn
                ]
                .align_y(Alignment::Center)
                .spacing(6),
                row![
                    tip_label(
                        "Output",
                        "Where to save the audio file. Leave blank to use the launch folder when you press Start.",
                    ),
                    out_path,
                    browse_btn
                ]
                .align_y(Alignment::Center)
                .spacing(6),
            ]
            .spacing(6),
        )
        .padding(10)
        .width(Fill)
        .style(card_style)
        .into()
    }

    /// Log: selectable monospace console; colors by message kind via LogHighlighter.
    fn view_log_card(&self) -> Element<'_, Message> {
        // Ctrl+C copies the selection; typing/paste are blocked in key_binding + LogAction.
        let log_editor = text_editor(&self.log_content)
            .height(Fill)
            .padding([8, 10])
            .size(11)
            .font(Font::MONOSPACE)
            .style(log_editor_style)
            .highlight_with::<LogHighlighter>(self.log_hl_settings(), log_hl_format)
            .key_binding(|key_press| {
                // Explicit Ctrl/Cmd+C / +A (default binding only works when the editor is focused).
                if key_press.modifiers.command() {
                    if let Key::Character(c) = &key_press.key {
                        if c.eq_ignore_ascii_case("c") {
                            return Some(text_editor::Binding::Copy);
                        }
                        if c.eq_ignore_ascii_case("a") {
                            return Some(text_editor::Binding::SelectAll);
                        }
                    }
                }
                let binding = text_editor::Binding::from_key_press(key_press)?;
                match binding {
                    text_editor::Binding::Copy
                    | text_editor::Binding::Select(_)
                    | text_editor::Binding::SelectWord
                    | text_editor::Binding::SelectLine
                    | text_editor::Binding::SelectAll
                    | text_editor::Binding::Move(_)
                    | text_editor::Binding::Unfocus => Some(binding),
                    // Block Cut / Paste / Insert / Enter / Backspace / Delete.
                    _ => None,
                }
            })
            .on_action(Message::LogAction);

        // Right-click copies the full log (left-click still selects in the editor).
        let log_editor = mouse_area(log_editor).on_right_press(Message::LogCopyAll);

        // Version stays visible even after the startup log line scrolls away.
        let version_label = text(format!("v{}", env!("CARGO_PKG_VERSION")))
            .size(11)
            .color(muted_text());
        let log_header = row![
            tooltip(
                section_title("Log"),
                container(
                    text(
                        "Status console. Colored ● is the main entry; gray · marks indented details. Text is black. Select + Ctrl+C copies; right-click copies all.",
                    )
                    .size(11)
                    .color(Color::from_rgb8(0xF6, 0xF5, 0xF4)),
                )
                .padding([6, 10])
                .max_width(320),
                TipPos::Bottom,
            )
            .gap(4)
            .delay(TOOLTIP_DELAY)
            .style(tip_style),
            Space::new().width(Fill),
            version_label,
        ]
        .width(Fill)
        .align_y(Alignment::Center);

        let log_subcard = container(log_editor).width(Fill).height(Fill).clip(true);

        container(
            column![log_header, log_subcard]
                .spacing(6)
                .width(Fill)
                .height(Fill),
        )
        .padding(10)
        .width(Fill)
        .height(Fill)
        .style(card_style)
        .into()
    }

    /// Action bar: Defaults, Cancel, Preview, Start (inset matches card padding).
    fn view_action_bar(&self, f: FocusSlot) -> Element<'_, Message> {
        let actions = container(
            row![
                self.defaults_button(f),
                Space::new().width(Fill),
                self.cancel_button(),
                self.preview_button(f),
                self.start_button(f),
            ]
            .align_y(Alignment::Center)
            .spacing(8),
        )
        .padding(iced::Padding {
            top: 0.0,
            right: 10.0,
            bottom: 0.0,
            left: 10.0,
        })
        .width(Fill);
        actions.into()
    }

    // Defaults always visible; disabled when settings match factory defaults (or busy).
    fn defaults_button(&self, f: FocusSlot) -> Element<'_, Message> {
        let on_press = (self.settings_dirty() && !self.is_busy()).then_some(Message::ResetDefaults);
        field_btn(
            "Defaults",
            primary_button_style(f == FocusSlot::Defaults),
            on_press,
        )
        .into()
    }

    // Enabled only while Start/Preview is running and cancel is not already requested.
    fn cancel_button(&self) -> Element<'_, Message> {
        let job_running = self.is_saving() || self.is_previewing();
        let cancel_requested = self.cancel_requested();
        let can_click = job_running && !cancel_requested;
        let label = if job_running && cancel_requested {
            "Cancelling"
        } else {
            "Cancel"
        };
        let style = if can_click {
            primary_button
        } else {
            unavailable_button
        };
        field_btn(label, style, can_click.then_some(Message::CancelSave)).into()
    }

    // Disabled for bad Source / Pages / Skip and while busy. Shows "Playing" during a
    // preview until Cancel is pressed.
    fn preview_button(&self, f: FocusSlot) -> Element<'_, Message> {
        let can_click = self.can_preview();
        let label = if self.is_previewing() && !self.cancel_requested() {
            "Playing"
        } else {
            "Preview"
        };
        let style = if can_click {
            primary_button_style(f == FocusSlot::Preview)
        } else {
            unavailable_button
        };
        field_btn(label, style, can_click.then_some(Message::Preview)).into()
    }

    // Disabled until Source, Pages/Skip, and Output (hard errors) are OK. Shows
    // "Starting" only while a Start job is still spinning up.
    fn start_button(&self, f: FocusSlot) -> Element<'_, Message> {
        let can_click = self.can_start();
        let label = if !self.cancel_requested() && self.save_still_starting() {
            "Starting"
        } else {
            "Start"
        };
        let style = if can_click {
            primary_button_style(f == FocusSlot::Save)
        } else {
            unavailable_button
        };
        field_btn(label, style, can_click.then_some(Message::Speak)).into()
    }

    fn cancel_requested(&self) -> bool {
        self.cancel_flag.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Progress while Start/Preview runs, or sticky 100% Done after a finish
    /// (cleared on next Start/Preview or when Source changes).
    fn view_progress_row(&self) -> Element<'_, Message> {
        let (prog_frac, prog_label) = self
            .progress_shared
            .lock()
            .ok()
            .map_or((0.0, String::new()), |g| {
                (g.fraction.clamp(0.0, 1.0), g.label.clone())
            });
        let busy_progress = self.is_saving() || self.is_previewing();
        let sticky_done =
            !busy_progress && prog_frac >= 0.999 && prog_label.trim().eq_ignore_ascii_case("Done");
        let show_job_progress = busy_progress || sticky_done;
        let prog_frac = if sticky_done { 1.0 } else { prog_frac };
        let pct = (prog_frac * 100.0).round() as i32;
        let stage_label = if sticky_done {
            "Done".into()
        } else if show_job_progress {
            let stage = prog_label.trim();
            if stage.is_empty() {
                "Working…".into()
            } else if stage.chars().count() > 36 {
                let s: String = stage.chars().take(34).collect();
                format!("{s}…")
            } else {
                stage.to_string()
            }
        } else {
            "Progress".into()
        };
        let pct_label = if sticky_done {
            "100%".into()
        } else if show_job_progress {
            format!("{pct}%")
        } else {
            String::new()
        };
        let bar_value = if sticky_done {
            1.0
        } else if show_job_progress {
            prog_frac
        } else {
            0.0
        };
        container(
            column![
                row![
                    text(stage_label).size(11).color(muted_text()),
                    Space::new().width(Fill),
                    text(pct_label).size(11).color(muted_text()),
                ]
                .align_y(Alignment::Center),
                progress_bar(0.0..=1.0, bar_value)
                    .girth(Length::Fixed(8.0))
                    .style(progress_bar_style),
            ]
            .spacing(4)
            .width(Fill),
        )
        .padding(iced::Padding {
            top: 2.0,
            right: 10.0,
            bottom: 2.0,
            left: 10.0,
        })
        .width(Fill)
        .into()
    }
}
