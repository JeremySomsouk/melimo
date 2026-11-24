use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Padding},
};

const BACKGROUND: Color = Color::Rgb(16, 24, 28);
const FOREGROUND: Color = Color::Rgb(235, 242, 232);
const MINT: Color = Color::Rgb(130, 227, 196);
const MUTED: Color = Color::Rgb(158, 181, 181);
const BORDER: Color = Color::Rgb(70, 96, 99);

pub struct Theme {
    monochrome: bool,
    light: bool,
}

impl Theme {
    pub fn from_env() -> Self {
        let mode = std::env::var("MELIMO_THEME").unwrap_or_default();
        Self {
            light: mode == "light",
            monochrome: mode == "mono"
                || std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
                || std::env::var("TERM").is_ok_and(|value| value == "dumb"),
        }
    }

    fn color(&self, dark: Color, light: Color) -> Color {
        if self.light { light } else { dark }
    }

    pub fn base(&self) -> Style {
        if self.monochrome {
            Style::default()
        } else {
            Style::default()
                .fg(self.color(FOREGROUND, Color::Rgb(28, 43, 43)))
                .bg(self.color(BACKGROUND, Color::Rgb(247, 247, 239)))
        }
    }

    pub fn title(&self) -> Style {
        self.base().add_modifier(Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        if self.monochrome {
            self.title()
        } else {
            self.title().fg(self.color(MINT, Color::Rgb(16, 103, 84)))
        }
    }

    pub fn selected(&self) -> Style {
        if self.monochrome {
            self.title().add_modifier(Modifier::REVERSED)
        } else {
            self.title()
                .fg(self.color(BACKGROUND, Color::Rgb(247, 247, 239)))
                .bg(self.color(MINT, Color::Rgb(16, 103, 84)))
        }
    }

    pub fn secondary(&self) -> Style {
        if self.monochrome {
            self.base().add_modifier(Modifier::DIM)
        } else {
            self.base().fg(self.color(MUTED, Color::Rgb(76, 102, 100)))
        }
    }

    pub fn signal(&self) -> Style {
        if self.monochrome {
            self.title()
        } else {
            self.title()
                .fg(self.color(Color::Rgb(255, 193, 117), Color::Rgb(135, 71, 15)))
        }
    }

    pub fn border(&self) -> Style {
        if self.monochrome {
            self.base()
        } else {
            self.base()
                .fg(self.color(BORDER, Color::Rgb(119, 143, 137)))
        }
    }

    pub fn panel<'a>(&self, title: impl Into<Line<'a>>) -> Block<'a> {
        Block::bordered()
            .border_type(BorderType::Plain)
            .padding(Padding::horizontal(1))
            .style(self.base())
            .border_style(self.border())
            .title_style(self.accent())
            .title(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_secondary_and_accent_text_have_readable_contrast() {
        fn luminance(color: Color) -> f64 {
            let Color::Rgb(r, g, b) = color else {
                panic!("expected RGB palette")
            };
            [r, g, b]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(channel, weight)| {
                    let value = f64::from(channel) / 255.0;
                    weight
                        * if value <= 0.04045 {
                            value / 12.92
                        } else {
                            ((value + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        for foreground in [FOREGROUND, MUTED, MINT] {
            let ratio = (luminance(foreground) + 0.05) / (luminance(BACKGROUND) + 0.05);
            assert!(ratio >= 4.5, "insufficient contrast: {ratio}");
        }
    }

    #[test]
    fn monochrome_preserves_terminal_colors_and_selection_cue() {
        let theme = Theme {
            monochrome: true,
            light: false,
        };
        for style in [
            theme.base(),
            theme.title(),
            theme.accent(),
            theme.secondary(),
        ] {
            assert_eq!(style.fg, None);
            assert_eq!(style.bg, None);
        }
        assert!(theme.selected().add_modifier.contains(Modifier::REVERSED));
    }
}
