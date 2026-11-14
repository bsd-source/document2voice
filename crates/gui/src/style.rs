//! Light desktop-style theme and widget looks (GNOME Adwaita-inspired).

use crate::Message;
use iced::border::Border;
use iced::theme::Palette;
use iced::widget::button::Status as BtnStatus;
use iced::widget::{button, container, text};
use iced::{Background, Color, Fill, Length, Shadow, Theme};

pub(crate) const MAIN_W: f32 = 443.0;
/// Sized to fit Speech + Audio + Files + Log + progress + action buttons.
pub(crate) const MAIN_H: f32 = 700.0;
/// Shared size for Defaults / Cancel / Preview / Start.
pub(crate) const FIELD_BTN_W: f32 = 70.0;
pub(crate) const FIELD_BTN_H: f32 = 28.0;
pub(crate) const FIELD_BTN_TEXT: f32 = 11.0;
/// Compact “…” control next to path fields (opens a file dialog).
/// Height matches Pages From/To edit boxes (~12px text + pad 3+3 + 1.3 line).
pub(crate) const ELLIPSIS_BTN_W: f32 = 22.0;
pub(crate) const ELLIPSIS_BTN_H: f32 = 22.0;
pub(crate) const ELLIPSIS_BTN_TEXT: f32 = 14.0;

/// Soft light palette inspired by GNOME Adwaita.
pub(crate) fn adwaita_theme() -> Theme {
    Theme::custom(
        "Adwaita-like",
        Palette {
            // Window chrome / app background
            background: Color::from_rgb8(0xF6, 0xF5, 0xF4), // light window background
            text: Color::from_rgb8(0x24, 0x1F, 0x31),       // dark charcoal text
            primary: Color::from_rgb8(0x35, 0x84, 0xE4),    // familiar desktop blue
            success: Color::from_rgb8(0x2E, 0xC2, 0x7E),
            warning: Color::from_rgb8(0xE5, 0xA5, 0x0A),
            danger: Color::from_rgb8(0xE0, 0x1B, 0x24),
        },
    )
}

// --- surface colors used in custom widget styles ---
pub(crate) fn card_bg() -> Color {
    Color::from_rgb8(0xFF, 0xFF, 0xFF)
}
pub(crate) fn muted_text() -> Color {
    Color::from_rgb8(0x5E, 0x5C, 0x64)
}
pub(crate) fn border_color() -> Color {
    Color::from_rgb8(0xDE, 0xDD, 0xDA)
}

/// Soft near-white for disabled text fields (Pages/Skip before Source is set).
pub(crate) fn disabled_control_bg() -> Color {
    Color::from_rgb8(0xFA, 0xF9, 0xF8)
}
pub(crate) fn disabled_control_border() -> Color {
    Color::from_rgb8(0xF0, 0xEF, 0xED)
}
pub(crate) fn disabled_control_text() -> Color {
    Color::from_rgb8(0xCE, 0xCD, 0xCA)
}

/// Slim progress bar: grey track, bright green fill.
pub(crate) fn progress_bar_style(_theme: &Theme) -> iced::widget::progress_bar::Style {
    use iced::widget::progress_bar;
    progress_bar::Style {
        background: Background::Color(Color::from_rgb8(0xE6, 0xE5, 0xE2)),
        // Bright green for the filled (working) portion.
        bar: Background::Color(Color::from_rgb8(0x00, 0xFF, 0x66)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 4.0.into(),
        },
    }
}

pub(crate) fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(card_bg())),
        text_color: Some(Color::from_rgb8(0x24, 0x1F, 0x31)),
        border: Border {
            color: border_color(),
            width: 1.0,
            radius: 12.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.06),
            offset: iced::Vector::new(0.0, 1.0),
            blur_radius: 4.0,
        },
        snap: false,
    }
}

/// Floating dialog card for overwrite confirm (elevated above the dimmed page).
pub(crate) fn dialog_card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(card_bg())),
        text_color: Some(Color::from_rgb8(0x24, 0x1F, 0x31)),
        border: Border {
            color: border_color(),
            width: 1.0,
            radius: 16.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.22),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 28.0,
        },
        snap: false,
    }
}

/// Soft path “chip” inside the dialog (monospace path on a light inset surface).
pub(crate) fn dialog_path_chip_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0xF6, 0xF5, 0xF4))),
        text_color: Some(Color::from_rgb8(0x24, 0x1F, 0x31)),
        border: Border {
            color: Color::from_rgb8(0xE6, 0xE5, 0xE2),
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Dimmed cool-grey scrim behind dialogs.
pub(crate) fn dialog_scrim_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.14, 0.12, 0.16, 0.42))),
        ..container::Style::default()
    }
}

/// Outline / flat Cancel button on dialogs (neutral, not blue-filled).
pub(crate) fn dialog_cancel_button(_theme: &Theme, status: BtnStatus) -> button::Style {
    flat_button(status, false)
}

/// Destructive confirm (Overwrite) — Adwaita-style danger red.
pub(crate) fn dialog_danger_button(_theme: &Theme, status: BtnStatus) -> button::Style {
    let base = Color::from_rgb8(0xE0, 0x1B, 0x24);
    let bg = match status {
        BtnStatus::Active => base,
        BtnStatus::Hovered => Color::from_rgb8(0xC0, 0x17, 0x1E),
        BtnStatus::Pressed => Color::from_rgb8(0xA5, 0x14, 0x19),
        BtnStatus::Disabled => Color::from_rgb8(0xF5, 0xA6, 0xA9),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: Color::WHITE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub(crate) fn page_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0xF6, 0xF5, 0xF4))),
        text_color: Some(Color::from_rgb8(0x24, 0x1F, 0x31)),
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Floating hover-tip bubble (dark background, light text).
pub(crate) fn tip_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x2E, 0x2A, 0x33))),
        text_color: Some(Color::from_rgb8(0xF6, 0xF5, 0xF4)),
        border: Border {
            color: Color::from_rgb8(0x5E, 0x5C, 0x64),
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        snap: false,
    }
}

/// Shared focus highlight — Tab/click focus. Brighter than normal control blue (#3584E4).
pub(crate) fn focus_accent() -> Color {
    Color::from_rgb8(0x6C, 0xB3, 0xFF)
}

/// Shared blue filled button (all app actions use this look).
/// Focus only brightens the fill — border stays transparent so size doesn’t change.
fn blue_button(theme: &Theme, status: BtnStatus, focused: bool) -> button::Style {
    let base = if focused {
        focus_accent()
    } else {
        theme.palette().primary
    };
    let shade = |k: f32| Color {
        r: (base.r * k).min(1.0),
        g: (base.g * k).min(1.0),
        b: (base.b * k).min(1.0),
        a: 1.0,
    };
    let bg = match status {
        BtnStatus::Active => base,
        BtnStatus::Hovered => shade(0.90),
        BtnStatus::Pressed => shade(0.82),
        BtnStatus::Disabled => Color::from_rgb8(0x99, 0xC1, 0xF1), // washed-out blue
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: Color::WHITE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub(crate) fn primary_button(theme: &Theme, status: BtnStatus) -> button::Style {
    blue_button(theme, status, false)
}

fn primary_button_focus(theme: &Theme, status: BtnStatus) -> button::Style {
    blue_button(theme, status, true)
}

/// Blue action button, brighter when it has keyboard focus.
pub(crate) fn primary_button_style(focused: bool) -> fn(&Theme, BtnStatus) -> button::Style {
    if focused {
        primary_button_focus
    } else {
        primary_button
    }
}

/// Washed-out look while a job runs or an action is unavailable (focus has no effect).
pub(crate) fn unavailable_button(theme: &Theme, _status: BtnStatus) -> button::Style {
    blue_button(theme, BtnStatus::Disabled, false)
}

/// Flat / secondary control (path “…” pickers), desktop-style.
/// Neutral gray surface + dark glyph; hover darkens; focus uses accent border.
/// Border width is always 1 so layout never jumps.
pub(crate) fn flat_button(status: BtnStatus, focused: bool) -> button::Style {
    // Light desktop: window ~#FAFAFA, header/button surfaces ~#EBEBEB / #E4E2DE
    let (bg, fg) = match status {
        BtnStatus::Active => (
            Color::from_rgb8(0xE8, 0xE6, 0xE3),
            Color::from_rgb8(0x24, 0x1F, 0x31),
        ),
        BtnStatus::Hovered => (
            Color::from_rgb8(0xDC, 0xDA, 0xD6),
            Color::from_rgb8(0x24, 0x1F, 0x31),
        ),
        BtnStatus::Pressed => (
            Color::from_rgb8(0xCE, 0xCC, 0xC7),
            Color::from_rgb8(0x24, 0x1F, 0x31),
        ),
        BtnStatus::Disabled => (
            Color::from_rgb8(0xF0, 0xEF, 0xED),
            Color::from_rgb8(0x9A, 0x99, 0x96),
        ),
    };
    // Focus: keep same fill family but lean slightly cooler + accent edge.
    let bg = if focused && !matches!(status, BtnStatus::Disabled) {
        match status {
            BtnStatus::Hovered | BtnStatus::Pressed => Color::from_rgb8(0xD4, 0xE3, 0xF7),
            _ => Color::from_rgb8(0xE4, 0xEE, 0xFB),
        }
    } else {
        bg
    };
    let border = if focused {
        focus_accent()
    } else {
        Color::from_rgb8(0xCD, 0xC7, 0xC2)
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: if focused {
            Color::from_rgb8(0x1A, 0x5F, 0xB4) // blue accent for the “…” glyph when focused
        } else {
            fg
        },
        border: Border {
            color: border,
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub(crate) fn flat_button_style(focused: bool) -> fn(&Theme, BtnStatus) -> button::Style {
    if focused {
        |_: &Theme, status| flat_button(status, true)
    } else {
        |_: &Theme, status| flat_button(status, false)
    }
}

/// Fixed-size action button with centered label (Defaults / Cancel / Preview / Start).
pub(crate) fn field_btn(
    label: &str,
    style: fn(&Theme, BtnStatus) -> button::Style,
    on_press: Option<Message>,
) -> iced::widget::Button<'_, Message> {
    let label = text(label).size(FIELD_BTN_TEXT).width(Fill).center();
    let body = container(label)
        .width(Fill)
        .height(Fill)
        .center(Length::Fill);
    button(body)
        .width(Length::Fixed(FIELD_BTN_W))
        .height(Length::Fixed(FIELD_BTN_H))
        .padding(0)
        .style(style)
        .on_press_maybe(on_press)
}

/// Square-ish “…” button beside a path field — flat secondary look, opens a dialog.
pub(crate) fn ellipsis_btn(
    focused: bool,
    on_press: Option<Message>,
) -> iced::widget::Button<'static, Message> {
    let label = text("…").size(ELLIPSIS_BTN_TEXT).width(Fill).center();
    let body = container(label)
        .width(Fill)
        .height(Fill)
        .center(Length::Fill);
    button(body)
        .width(Length::Fixed(ELLIPSIS_BTN_W))
        .height(Length::Fixed(ELLIPSIS_BTN_H))
        .padding(0)
        .style(flat_button_style(focused))
        .on_press_maybe(on_press)
}

pub(crate) fn pick_list_style(
    focused: bool,
) -> impl Fn(&Theme, iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    move |theme: &Theme, status: iced::widget::pick_list::Status| {
        let mut s = iced::widget::pick_list::default(theme, status);
        s.border.width = 1.0;
        s.border.radius = 6.0.into();
        s.text_color = Color::from_rgb8(0x24, 0x1F, 0x31);
        s.handle_color = Color::from_rgb8(0x5E, 0x5C, 0x64);
        s.background = Background::Color(if focused {
            Color::from_rgb8(0xE8, 0xF1, 0xFC)
        } else {
            Color::from_rgb8(0xF5, 0xF4, 0xF3)
        });
        s.border.color = if focused {
            focus_accent()
        } else {
            match status {
                iced::widget::pick_list::Status::Hovered
                | iced::widget::pick_list::Status::Opened { .. } => focus_accent(),
                iced::widget::pick_list::Status::Active => border_color(),
            }
        };
        if focused {
            s.handle_color = focus_accent();
        }
        s
    }
}

/// Opened dropdown menu panel — slightly off-white background.
pub(crate) fn pick_menu_style(theme: &Theme) -> iced::widget::overlay::menu::Style {
    let mut s = iced::widget::overlay::menu::default(theme);
    s.background = Background::Color(Color::from_rgb8(0xF5, 0xF4, 0xF3));
    s.border = Border {
        width: 1.0,
        radius: 6.0.into(),
        color: border_color(),
    };
    s.text_color = Color::from_rgb8(0x24, 0x1F, 0x31);
    s.selected_background = Background::Color(Color::from_rgb8(0xE8, 0xF1, 0xFC));
    s.selected_text_color = Color::from_rgb8(0x24, 0x1F, 0x31);
    s
}

/// Text field chrome: focus, error (red border), or disabled.
///
/// Focus comes from the app's own [`crate::focus::FocusSlot`], not iced's
/// `Focused` status, so the accent follows the app's Tab/click focus.
pub(crate) fn text_field_style(
    focused: bool,
    error: bool,
    enabled: bool,
) -> impl Fn(&Theme, iced::widget::text_input::Status) -> iced::widget::text_input::Style {
    let error_border = Color::from_rgb8(0xC0, 0x1C, 0x28);
    move |theme: &Theme, status: iced::widget::text_input::Status| {
        let mut s = iced::widget::text_input::default(theme, status);
        s.border.width = 1.0;
        s.border.radius = 6.0.into();
        if !enabled {
            s.border.color = disabled_control_border();
            s.background = Background::Color(disabled_control_bg());
            s.value = disabled_control_text();
            s.placeholder = disabled_control_text();
            s.icon = disabled_control_text();
        } else if error {
            // Invalid path: red wins over focus accent so the problem stays visible.
            s.border.color = error_border;
            s.background = Background::Color(Color::from_rgb8(0xFF, 0xFB, 0xFA));
        } else if focused {
            s.border.color = focus_accent();
            s.background = Background::Color(Color::from_rgb8(0xFF, 0xFF, 0xFF));
        } else {
            s.border.color = border_color();
        }
        s
    }
}

/// Inactive (gray) track stays the same always.
pub(crate) fn slider_style(
    focused: bool,
) -> impl Fn(&Theme, iced::widget::slider::Status) -> iced::widget::slider::Style {
    move |theme: &Theme, status: iced::widget::slider::Status| {
        use iced::widget::slider::{Handle, HandleShape, Rail};
        let base = if focused {
            focus_accent()
        } else {
            theme.palette().primary
        };
        let color = match status {
            iced::widget::slider::Status::Active => base,
            iced::widget::slider::Status::Hovered => Color {
                r: (base.r * 0.92).min(1.0),
                g: (base.g * 0.92).min(1.0),
                b: (base.b * 0.92).min(1.0),
                a: 1.0,
            },
            iced::widget::slider::Status::Dragged => Color {
                r: (base.r * 0.85).min(1.0),
                g: (base.g * 0.85).min(1.0),
                b: (base.b * 0.85).min(1.0),
                a: 1.0,
            },
        };
        iced::widget::slider::Style {
            rail: Rail {
                backgrounds: (
                    Background::Color(color),
                    // Unfilled track — never changes on focus.
                    Background::Color(Color::from_rgb8(0xDE, 0xDD, 0xDA)),
                ),
                width: 4.0,
                border: Border {
                    radius: 2.0.into(),
                    width: 0.0,
                    color: Color::TRANSPARENT,
                },
            },
            handle: Handle {
                shape: HandleShape::Circle { radius: 7.0 },
                background: Background::Color(color),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}
