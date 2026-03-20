use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Parse a single line containing ANSI SGR escape sequences into a ratatui `Line`.
pub fn parse_ansi_line(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut seq = String::new();
            // Collect parameter bytes until a final byte (@ through ~)
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphabetic() || next == '@' {
                    chars.next();
                    if next == 'm' {
                        // SGR sequence — flush current text then apply style change
                        if !text.is_empty() {
                            spans.push(Span::styled(text.clone(), style));
                            text.clear();
                        }
                        style = apply_sgr(&seq, style);
                    }
                    // Non-'m' sequences (cursor movement etc.) are silently skipped
                    break;
                }
                seq.push(next);
                chars.next();
            }
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Apply an ANSI SGR parameter string to a base `Style`, returning the updated style.
fn apply_sgr(params: &str, mut style: Style) -> Style {
    if params.is_empty() {
        return Style::default();
    }
    let codes: Vec<u32> = params.split(';').filter_map(|s| s.parse().ok()).collect();
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            7 => style = style.add_modifier(Modifier::REVERSED),
            // Standard foreground
            30 => style = style.fg(Color::Black),
            31 => style = style.fg(Color::Red),
            32 => style = style.fg(Color::Green),
            33 => style = style.fg(Color::Yellow),
            34 => style = style.fg(Color::Blue),
            35 => style = style.fg(Color::Magenta),
            36 => style = style.fg(Color::Cyan),
            37 => style = style.fg(Color::White),
            39 => style = style.fg(Color::Reset),
            // Bright foreground
            90 => style = style.fg(Color::DarkGray),
            91 => style = style.fg(Color::LightRed),
            92 => style = style.fg(Color::LightGreen),
            93 => style = style.fg(Color::LightYellow),
            94 => style = style.fg(Color::LightBlue),
            95 => style = style.fg(Color::LightMagenta),
            96 => style = style.fg(Color::LightCyan),
            97 => style = style.fg(Color::Gray),
            // Standard background
            40 => style = style.bg(Color::Black),
            41 => style = style.bg(Color::Red),
            42 => style = style.bg(Color::Green),
            43 => style = style.bg(Color::Yellow),
            44 => style = style.bg(Color::Blue),
            45 => style = style.bg(Color::Magenta),
            46 => style = style.bg(Color::Cyan),
            47 => style = style.bg(Color::White),
            49 => style = style.bg(Color::Reset),
            // Bright background
            100 => style = style.bg(Color::DarkGray),
            101 => style = style.bg(Color::LightRed),
            102 => style = style.bg(Color::LightGreen),
            103 => style = style.bg(Color::LightYellow),
            104 => style = style.bg(Color::LightBlue),
            105 => style = style.bg(Color::LightMagenta),
            106 => style = style.bg(Color::LightCyan),
            107 => style = style.bg(Color::Gray),
            // 256-color / true-color foreground
            38 if codes.get(i + 1) == Some(&5) => {
                if let Some(&n) = codes.get(i + 2) {
                    style = style.fg(Color::Indexed(n.min(255) as u8));
                    i += 2;
                }
            }
            38 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    style = style.fg(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8));
                    i += 4;
                }
            }
            // 256-color / true-color background
            48 if codes.get(i + 1) == Some(&5) => {
                if let Some(&n) = codes.get(i + 2) {
                    style = style.bg(Color::Indexed(n.min(255) as u8));
                    i += 2;
                }
            }
            48 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    style = style.bg(Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8));
                    i += 4;
                }
            }
            _ => {}
        }
        i += 1;
    }
    style
}
