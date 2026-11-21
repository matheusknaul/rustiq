use iced::{
    Border, Color, Font, Theme, font,
    widget::{button, container, text_input},
};

pub const BASE: Color = Color::from_rgb8(36, 39, 58);
pub const MANTLE: Color = Color::from_rgb8(30, 32, 48);
pub const SURFACE: Color = Color::from_rgb8(54, 58, 79);
pub const BORDER: Color = Color::from_rgb8(73, 77, 100);
pub const TEXT: Color = Color::from_rgb8(202, 211, 245);
pub const MUTED: Color = Color::from_rgb8(147, 154, 183);
pub const BLUE: Color = Color::from_rgb8(138, 173, 244);
pub const GREEN: Color = Color::from_rgb8(166, 218, 149);
pub const YELLOW: Color = Color::from_rgb8(238, 212, 159);
pub const RED: Color = Color::from_rgb8(237, 135, 150);
pub const MAUVE: Color = Color::from_rgb8(198, 160, 246);
pub const MONO: Font = Font::with_name("DejaVu Sans Mono");
pub const ICONS: Font = Font {
    family: font::Family::Name("Font Awesome 6 Free"),
    weight: font::Weight::Black,
    ..Font::DEFAULT
};

pub fn rustiq_theme() -> Theme {
    Theme::custom(
        "RustiQ Macchiato",
        iced::theme::Palette {
            background: BASE,
            text: TEXT,
            primary: BLUE,
            success: GREEN,
            danger: RED,
            ..Theme::Dark.palette()
        },
    )
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(BASE.into()),
        border: Border {
            color: BORDER,
            width: 1.,
            radius: 3.into(),
        },
        ..Default::default()
    }
}
pub fn inset(_: &Theme) -> container::Style {
    container::Style {
        background: Some(MANTLE.into()),
        ..panel(&Theme::Dark)
    }
}
pub fn bar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(MANTLE.into()),
        ..Default::default()
    }
}
pub fn control(_: &Theme, status: button::Status) -> button::Style {
    let disabled = status == button::Status::Disabled;
    button::Style {
        background: Some(
            if status == button::Status::Hovered {
                SURFACE
            } else {
                BASE
            }
            .into(),
        ),
        text_color: if disabled { BORDER } else { BLUE },
        border: Border {
            color: if disabled { SURFACE } else { BORDER },
            width: 1.,
            radius: 3.into(),
        },
        ..Default::default()
    }
}
pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let disabled = status == button::Status::Disabled;
    button::Style {
        background: Some(
            if disabled {
                SURFACE
            } else if status == button::Status::Hovered {
                GREEN
            } else {
                BLUE
            }
            .into(),
        ),
        text_color: if disabled { MUTED } else { MANTLE },
        border: Border {
            radius: 3.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: MANTLE.into(),
        border: Border {
            color: if matches!(status, text_input::Status::Focused { .. }) {
                BLUE
            } else {
                BORDER
            },
            width: 1.,
            radius: 3.into(),
        },
        icon: BLUE,
        placeholder: MUTED,
        value: TEXT,
        selection: SURFACE,
    }
}
pub fn method_color(method: &str) -> Color {
    match method {
        "GET" | "HEAD" => GREEN,
        "POST" => YELLOW,
        "PUT" | "PATCH" => BLUE,
        "DELETE" => RED,
        _ => MAUVE,
    }
}

pub fn editor(
    _: &Theme,
    status: iced::widget::text_editor::Status,
) -> iced::widget::text_editor::Style {
    iced::widget::text_editor::Style {
        background: MANTLE.into(),
        border: Border {
            color: if matches!(status, iced::widget::text_editor::Status::Focused { .. }) {
                BLUE
            } else {
                BORDER
            },
            width: 1.,
            radius: 3.into(),
        },
        placeholder: MUTED,
        value: TEXT,
        selection: SURFACE,
    }
}
