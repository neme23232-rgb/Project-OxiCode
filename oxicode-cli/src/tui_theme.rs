use oxicode_core::highlight::{self, TokenKind};
use oxicode_core::theme::{self, Rgb, ThemeKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub raw: theme::Palette,
    pub bg: Color,
    pub bg_panel: Color,
    pub bg_element: Color,
    pub border: Color,
    pub border_active: Color,
    pub primary: Color,
    pub accent: Color,
    pub secondary: Color,
    pub text: Color,
    pub muted: Color,
    pub green: Color,
    pub red: Color,
    pub yellow: Color,
    pub cyan: Color,
}

fn to_color(c: Rgb) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

pub fn palette_for(kind: ThemeKind) -> Palette {
    let p = kind.palette();
    Palette {
        raw: p,
        bg: p.bg.map(to_color).unwrap_or(Color::Reset),
        bg_panel: p.bg_panel.map(to_color).unwrap_or(Color::Reset),
        bg_element: to_color(p.bg_element),
        border: to_color(p.border),
        border_active: to_color(p.border_active),
        primary: to_color(p.primary),
        accent: to_color(p.accent),
        secondary: to_color(p.secondary),
        text: p.text.map(to_color).unwrap_or(Color::Reset),
        muted: to_color(p.muted),
        green: to_color(p.green),
        red: to_color(p.red),
        yellow: to_color(p.yellow),
        cyan: to_color(p.cyan),
    }
}

pub struct FileIcon {
    pub badge: &'static str,
    pub color: Color,
}

pub fn file_icon(path: &str, pal: Palette) -> FileIcon {
    let fi = highlight::file_icon(path, pal.raw);
    FileIcon {
        badge: fi.badge,
        color: to_color(fi.color),
    }
}

pub fn lang_badge(lang: &str, pal: Palette) -> (&'static str, Color) {
    let (label, c) = highlight::lang_badge(lang, pal.raw);
    (label, to_color(c))
}

pub fn highlight_code_line<'a>(
    line: &'a str,
    lang: &str,
    pal: Palette,
    monochrome: bool,
) -> Vec<Span<'a>> {
    let bg = pal.bg_panel;
    highlight::tokenize_code_line(line, lang)
        .into_iter()
        .map(|(tok, kind)| {
            let style = match kind {
                TokenKind::Comment => Style::default()
                    .fg(pal.muted)
                    .bg(bg)
                    .add_modifier(Modifier::ITALIC),
                TokenKind::String => Style::default()
                    .fg(if monochrome { pal.cyan } else { pal.green })
                    .bg(bg),
                TokenKind::Keyword => Style::default()
                    .fg(pal.accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
                TokenKind::Type => Style::default().fg(pal.cyan).bg(bg),
                TokenKind::Number => Style::default()
                    .fg(if monochrome { pal.text } else { pal.yellow })
                    .bg(bg),
                TokenKind::Punct => Style::default().fg(pal.primary).bg(bg),
                TokenKind::Plain => Style::default().fg(pal.text).bg(bg),
            };
            Span::styled(tok, style)
        })
        .collect()
}
