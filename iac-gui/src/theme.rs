//! 主题令牌 → egui Color32（14 令牌日夜主题，_04/03 设计语言）与 CJK 字体装载。
//! 前端豁免组件；令牌语义与 iac/src/theme.rs 保持一致（前端各自实现映射，避免 bin 互依赖）。

use eframe::egui::{Color32, Context, FontData, FontDefinitions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Day,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // 令牌表全集（CLI/TUI 同构），部分槽位随 M10 面板接线启用
pub enum Token {
    Bg,
    Fg,
    Fg60,
    Fg40,
    Fg25,
    Accent,
    Safe,
    Caution,
    Dangerous,
    Destructive,
    SynKeyword,
    SynString,
}

/// token → Color32。
pub fn color(mode: Mode, token: Token) -> Color32 {
    let (r, g, b) = rgb(mode, token);
    Color32::from_rgb(r, g, b)
}

/// token → (R,G,B)。
pub fn rgb(mode: Mode, token: Token) -> (u8, u8, u8) {
    let hex: &str = match (mode, token) {
        (_, Token::Bg) => match mode {
            Mode::Day => "FAFAF7",
            Mode::Night => "1B1D22",
        },
        (_, Token::Fg) => match mode {
            Mode::Day => "262A33",
            Mode::Night => "D6D8DD",
        },
        (_, Token::Fg60) => match mode {
            Mode::Day => "6B7080",
            Mode::Night => "8B8F98",
        },
        (_, Token::Fg40) => match mode {
            Mode::Day => "9CA1AC",
            Mode::Night => "5C6068",
        },
        (_, Token::Fg25) => match mode {
            Mode::Day => "C3C7CE",
            Mode::Night => "3A3E46",
        },
        (_, Token::Accent) => match mode {
            Mode::Day => "316BFF",
            Mode::Night => "6E9BFF",
        },
        (_, Token::Safe) => match mode {
            Mode::Day => "2E7D32",
            Mode::Night => "66BB6A",
        },
        (_, Token::Caution) => match mode {
            Mode::Day => "B26A00",
            Mode::Night => "FFB74D",
        },
        (_, Token::Dangerous) => match mode {
            Mode::Day => "C7500A",
            Mode::Night => "FF8A50",
        },
        (_, Token::Destructive) => match mode {
            Mode::Day => "B3261E",
            Mode::Night => "FF5C52",
        },
        (_, Token::SynKeyword) => match mode {
            Mode::Day => "8250DF",
            Mode::Night => "C297FF",
        },
        (_, Token::SynString) => match mode {
            Mode::Day => "0A6E6E",
            Mode::Night => "6FC7C7",
        },
    };
    (
        u8::from_str_radix(&hex[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
    )
}

/// 全局样式：背景/正文/弱文本应用到 egui Visuals（iA Writer 式无框淡化）。
pub fn apply_style(ctx: &Context, mode: Mode) {
    let build = || egui::Visuals {
        panel_fill: color(mode, Token::Bg),
        window_fill: color(mode, Token::Bg),
        extreme_bg_color: color(mode, Token::Bg),
        faint_bg_color: color(mode, Token::Fg25),
        override_text_color: Some(color(mode, Token::Fg)),
        hyperlink_color: color(mode, Token::Fg60),
        selection: egui::style::Selection {
            bg_fill: color(mode, Token::Accent).gamma_multiply(0.35),
            stroke: egui::Stroke::new(1.0_f32, color(mode, Token::Accent)),
        },
        ..egui::Visuals::default()
    };
    // 两套 egui 主题都应用同一令牌表（日夜由我们的 Mode 决定，不跟随 egui 偏好）
    ctx.set_visuals_of(egui::Theme::Light, build());
    ctx.set_visuals_of(egui::Theme::Dark, build());
    ctx.set_theme(match mode {
        Mode::Day => egui::Theme::Light,
        Mode::Night => egui::Theme::Dark,
    });
}

/// 终端 ANSI 调色板 ← 14 令牌（终端块与面板同源，消除色缝；派生色仅 black/bright 白）。
pub fn terminal_palette(mode: Mode) -> egui_term::ColorPalette {
    let hex = |t: Token| format!("#{:02X}{:02X}{:02X}", rgb(mode, t).0, rgb(mode, t).1, rgb(mode, t).2);
    let black = match mode {
        Mode::Day => "#2E3138".to_string(),
        Mode::Night => "#0F1115".to_string(),
    };
    let bright_black = hex(Token::Fg40);
    let bright_white = match mode {
        Mode::Day => "#262A33".to_string(),
        Mode::Night => "#F2F3F5".to_string(),
    };
    egui_term::ColorPalette {
        foreground: hex(Token::Fg),
        background: hex(Token::Bg),
        bright_foreground: Some(bright_white.clone()),
        dim_foreground: hex(Token::Fg60),
        black: black.clone(),
        red: hex(Token::Destructive),
        green: hex(Token::Safe),
        yellow: hex(Token::Caution),
        blue: hex(Token::Accent),
        magenta: hex(Token::SynKeyword),
        cyan: hex(Token::SynString),
        white: hex(Token::Fg60),
        bright_black,
        bright_red: hex(Token::Destructive),
        bright_green: hex(Token::Safe),
        bright_yellow: hex(Token::Caution),
        bright_blue: hex(Token::Accent),
        bright_magenta: hex(Token::SynKeyword),
        bright_cyan: hex(Token::SynString),
        bright_white,
        dim_black: black,
        dim_red: hex(Token::Dangerous),
        dim_green: hex(Token::Safe),
        dim_yellow: hex(Token::Caution),
        dim_blue: hex(Token::Accent),
        dim_magenta: hex(Token::SynKeyword),
        dim_cyan: hex(Token::SynString),
        dim_white: hex(Token::Fg60),
    }
}

/// 装载系统中文字体（egui 默认字库无 CJK）：扫描常见 Noto/WQY 路径，找不到则保持默认。
pub fn install_fonts(ctx: &Context) {
    let candidates = [
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJKsc-Regular.otf",
        "/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc",
        "/usr/share/fonts/wqy-zenhei/wqy-zenhei.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
    ];
    let mut fonts = FontDefinitions::default();
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert("cjk".into(), FontData::from_owned(data).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.get_mut(&family).unwrap().push("cjk".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_night_palettes_differ() {
        assert_ne!(color(Mode::Day, Token::Fg), color(Mode::Night, Token::Fg));
        assert_eq!(color(Mode::Night, Token::Bg), Color32::from_rgb(0x1B, 0x1D, 0x22));
        assert_eq!(color(Mode::Day, Token::Destructive), Color32::from_rgb(0xB3, 0x26, 0x1E));
    }

    #[test]
    fn rgb_parses_all_tokens() {
        for token in [
            Token::Bg, Token::Fg, Token::Fg60, Token::Fg40, Token::Fg25, Token::Accent,
            Token::Safe, Token::Caution, Token::Dangerous, Token::Destructive,
            Token::SynKeyword, Token::SynString,
        ] {
            let (r, g, b) = rgb(Mode::Day, token);
            assert!((r as u32) + (g as u32) + (b as u32) > 0 || matches!(token, Token::Fg25));
        }
    }

    #[test]
    fn terminal_palette_tracks_tokens() {
        for mode in [Mode::Day, Mode::Night] {
            let p = terminal_palette(mode);
            let bg = format!("#{:02X}{:02X}{:02X}", rgb(mode, Token::Bg).0, rgb(mode, Token::Bg).1, rgb(mode, Token::Bg).2);
            let red = format!("#{:02X}{:02X}{:02X}", rgb(mode, Token::Destructive).0, rgb(mode, Token::Destructive).1, rgb(mode, Token::Destructive).2);
            assert_eq!(p.background, bg, "终端背景必须与面板 Bg 同源");
            assert_eq!(p.red, red);
            assert_eq!(p.foreground, format!("#{:02X}{:02X}{:02X}", rgb(mode, Token::Fg).0, rgb(mode, Token::Fg).1, rgb(mode, Token::Fg).2));
        }
    }
}
