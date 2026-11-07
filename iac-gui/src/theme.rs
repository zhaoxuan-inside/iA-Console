//! 设计系统令牌 → egui（重建 UI：Linear 式四级 surface 阶梯 + 克制点缀色，
//! 终端调色板参照 Warp 的暖炭底现代 ANSI）。规范来源：design/refs/*.design.md。
//! 令牌语义与 iac/src/theme.rs 保持同名（前端各自实现映射，避免 bin 互依赖）。

use eframe::egui::{Color32, Context, FontData, FontDefinitions, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Day,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // 令牌表全集，部分槽位随面板接线启用
pub enum Token {
    /// 画布（最深底，Linear canvas）
    Bg,
    /// 抬升面一级（终端块/条区容器）
    Surface,
    /// 抬升面二级（卡片/输入框）
    SurfaceRaised,
    /// 悬停面（surface-3）
    SurfaceHover,
    /// 发丝线（边框/分隔）
    Hairline,
    /// 正文（ink）
    Fg,
    /// 次级文本（ink-subtle）
    Fg60,
    /// 三级文本（ink-tertiary）
    Fg40,
    /// 弱化文本/微弱底
    Fg25,
    /// 品牌点缀（薰衣草蓝，克制使用：焦点/主操作/活动位）
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

/// token → (R,G,B)。色值出处：design/refs/linear.app.design.md（surface 阶梯/ink 阶梯/点缀色）
/// 与 design/refs/warp.design.md（终端现代 ANSI 的克制饱和度取向）。
pub fn rgb(mode: Mode, token: Token) -> (u8, u8, u8) {
    let hex: &str = match (mode, token) {
        // canvas：夜=#010102（Linear 最深底）；昼=近白带冷灰
        (_, Token::Bg) => match mode {
            Mode::Day => "fbfbfc",
            Mode::Night => "010102",
        },
        // surface-1 阶梯
        (_, Token::Surface) => match mode {
            Mode::Day => "f2f3f5",
            Mode::Night => "0f1011",
        },
        // surface-2
        (_, Token::SurfaceRaised) => match mode {
            Mode::Day => "ffffff",
            Mode::Night => "141516",
        },
        // surface-3（悬停）
        (_, Token::SurfaceHover) => match mode {
            Mode::Day => "e9eaee",
            Mode::Night => "18191a",
        },
        // 发丝线
        (_, Token::Hairline) => match mode {
            Mode::Day => "e3e4e8",
            Mode::Night => "23252a",
        },
        // ink
        (_, Token::Fg) => match mode {
            Mode::Day => "1c1c1f",
            Mode::Night => "f7f8f8",
        },
        // ink-subtle
        (_, Token::Fg60) => match mode {
            Mode::Day => "6c707a",
            Mode::Night => "8a8f98",
        },
        // ink-tertiary
        (_, Token::Fg40) => match mode {
            Mode::Day => "9b9ea6",
            Mode::Night => "62666d",
        },
        // 弱化文本（比 tertiary 更隐）
        (_, Token::Fg25) => match mode {
            Mode::Day => "b7bac1",
            Mode::Night => "4b4f56",
        },
        // 点缀：昼=品牌薰衣草 #5e6ad2；夜=hover 亮调 #828fff（暗底可读）
        (_, Token::Accent) => match mode {
            Mode::Day => "5e6ad2",
            Mode::Night => "828fff",
        },
        (_, Token::Safe) => match mode {
            Mode::Day => "217a53",
            Mode::Night => "3dd68c",
        },
        (_, Token::Caution) => match mode {
            Mode::Day => "ad5f00",
            Mode::Night => "ffb224",
        },
        // dangerous=橙红，destructive=红（等级差由色相区分）
        (_, Token::Dangerous) => match mode {
            Mode::Day => "bd4b00",
            Mode::Night => "f76b15",
        },
        (_, Token::Destructive) => match mode {
            Mode::Day => "d13438",
            Mode::Night => "ff6369",
        },
        (_, Token::SynKeyword) => match mode {
            Mode::Day => "8250df",
            Mode::Night => "b18aff",
        },
        (_, Token::SynString) => match mode {
            Mode::Day => "0a7c6e",
            Mode::Night => "45d4b8",
        },
    };
    (
        u8::from_str_radix(&hex[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
    )
}

/// 全局样式：Linear 式表面层次（surface 阶梯承载层级，不用阴影）、控件圆角、紧凑间距。
pub fn apply_style(ctx: &Context, mode: Mode) {
    let build = || {
        let mut v = egui::Visuals {
            panel_fill: color(mode, Token::Bg),
            window_fill: color(mode, Token::Surface),
            extreme_bg_color: color(mode, Token::Surface),
            faint_bg_color: color(mode, Token::SurfaceHover),
            override_text_color: Some(color(mode, Token::Fg)),
            hyperlink_color: color(mode, Token::Accent),
            selection: egui::style::Selection {
                bg_fill: color(mode, Token::Accent).gamma_multiply(0.28),
                stroke: egui::Stroke::NONE,
            },
            window_stroke: egui::Stroke::new(1.0_f32, color(mode, Token::Hairline)),
            window_corner_radius: egui::CornerRadius::same(10),
            menu_corner_radius: egui::CornerRadius::same(8),
            ..egui::Visuals::default()
        };
        // 控件：surface-2 底 + 发丝线描边 + 6px 圆角（Linear 紧凑圆角语义）
        let widget = |bg: Color32, stroke: Color32| egui::style::WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: egui::Stroke::new(1.0_f32, stroke),
            corner_radius: egui::CornerRadius::same(6),
            fg_stroke: egui::Stroke::new(1.0_f32, color(mode, Token::Fg)),
            expansion: 0.0,
        };
        v.widgets.noninteractive =
            widget(color(mode, Token::Surface), color(mode, Token::Hairline));
        v.widgets.inactive = widget(
            color(mode, Token::SurfaceRaised),
            color(mode, Token::Hairline),
        );
        v.widgets.hovered = widget(color(mode, Token::SurfaceHover), color(mode, Token::Fg40));
        v.widgets.active = widget(color(mode, Token::SurfaceHover), color(mode, Token::Accent));
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, color(mode, Token::Accent));
        v
    };
    let mut style = Style {
        visuals: build(),
        ..Style::default()
    };
    // 间距：紧凑（Linear spacing.xs~sm）；菜单内边距 6
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.menu_margin = egui::Margin::symmetric(6, 6);
    // 字号阶梯（Linear body-sm/caption 节奏；等宽 13px）
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(17.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(13.5)),
        (egui::TextStyle::Button, egui::FontId::proportional(13.0)),
        (egui::TextStyle::Small, egui::FontId::proportional(11.5)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
    ]
    .into();
    // 两套 egui 主题都应用同一令牌表（日夜由我们的 Mode 决定）
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_theme(match mode {
        Mode::Day => egui::Theme::Light,
        Mode::Night => egui::Theme::Dark,
    });
}

/// 终端 ANSI 调色板：夜=Linear surface-1 暖炭底 + 现代克制 ANSI（Warp 取向，点缀蓝入 brand 槽）；
/// 昼=白底深 ANSI。派生关系仅 black/bright 白，其余为独立现代值。
pub fn terminal_palette(mode: Mode) -> egui_term::ColorPalette {
    if mode == Mode::Night {
        return egui_term::ColorPalette {
            foreground: "#f7f8f8".into(),
            background: "#0f1011".into(),
            bright_foreground: Some("#f7f8f8".into()),
            dim_foreground: "#8a8f98".into(),
            black: "#1c1d21".into(),
            red: "#e5484d".into(),
            green: "#30a46c".into(),
            yellow: "#ffb224".into(),
            blue: "#5e6ad2".into(),
            magenta: "#bf7af0".into(),
            cyan: "#4cc2d1".into(),
            white: "#8a8f98".into(),
            bright_black: "#34343a".into(),
            bright_red: "#ff6369".into(),
            bright_green: "#3dd68c".into(),
            bright_yellow: "#ffd60a".into(),
            bright_blue: "#828fff".into(),
            bright_magenta: "#d38cff".into(),
            bright_cyan: "#7cd6e0".into(),
            bright_white: "#f7f8f8".into(),
            dim_black: "#1c1d21".into(),
            dim_red: "#c25658".into(),
            dim_green: "#2a8a58".into(),
            dim_yellow: "#d09a2e".into(),
            dim_blue: "#5a61bd".into(),
            dim_magenta: "#a56ad0".into(),
            dim_cyan: "#45a8b4".into(),
            dim_white: "#6c707a".into(),
        };
    }
    egui_term::ColorPalette {
        foreground: "#1c1c1f".into(),
        background: "#ffffff".into(),
        bright_foreground: Some("#1c1c1f".into()),
        dim_foreground: "#9b9ea6".into(),
        black: "#ececee".into(),
        red: "#d13438".into(),
        green: "#217a53".into(),
        yellow: "#ad5f00".into(),
        blue: "#5e6ad2".into(),
        magenta: "#8347b5".into(),
        cyan: "#0f7e93".into(),
        white: "#6c707a".into(),
        bright_black: "#c9c9cf".into(),
        bright_red: "#e5484d".into(),
        bright_green: "#2fa06b".into(),
        bright_yellow: "#c97a10".into(),
        bright_blue: "#6f79e0".into(),
        bright_magenta: "#9a5fd0".into(),
        bright_cyan: "#2b96ab".into(),
        bright_white: "#1c1c1f".into(),
        dim_black: "#e3e4e8".into(),
        dim_red: "#b84043".into(),
        dim_green: "#256c4c".into(),
        dim_yellow: "#96560e".into(),
        dim_blue: "#545ec4".into(),
        dim_magenta: "#7743a5".into(),
        dim_cyan: "#127285".into(),
        dim_white: "#9b9ea6".into(),
    }
}

// ---------- 组件助手（2025-2026 组件语言：状态点 / kbd 键帽；Linear/Raycast 语义） ----------

/// 状态点：实心小圆（daemon 连接 / 标签运行指示）。
pub fn status_dot(ui: &mut egui::Ui, color: Color32, radius: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(radius * 2.0, radius * 2.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
}

/// kbd 键帽：mono 小字 + surface-2 底 + 发丝线 + 4px 圆角；可点击（返回 Response）。
pub fn kbd_chip(ui: &mut egui::Ui, mode: Mode, keys: &str, id_salt: &str) -> egui::Response {
    let inner = egui::Frame::NONE
        .fill(color(mode, Token::SurfaceRaised))
        .stroke(egui::Stroke::new(1.0_f32, color(mode, Token::Hairline)))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(keys)
                    .monospace()
                    .small()
                    .color(color(mode, Token::Fg40)),
            );
        });
    ui.interact(
        inner.response.rect,
        egui::Id::new(id_salt),
        egui::Sense::click(),
    )
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
            fonts
                .font_data
                .insert("cjk".into(), FontData::from_owned(data).into());
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
        // Linear canvas 与 surface 阶梯锚点
        assert_eq!(
            color(Mode::Night, Token::Bg),
            Color32::from_rgb(0x01, 0x01, 0x02)
        );
        assert_eq!(
            color(Mode::Night, Token::Surface),
            Color32::from_rgb(0x0f, 0x10, 0x11)
        );
        assert_eq!(
            color(Mode::Day, Token::Accent),
            Color32::from_rgb(0x5e, 0x6a, 0xd2)
        );
    }

    #[test]
    fn surface_ladder_ascends_in_dark_mode() {
        // 夜间阶梯：Bg < Surface < SurfaceRaised < SurfaceHover（亮度递增承载层级）
        let lum = |t: Token| {
            let (r, g, b) = rgb(Mode::Night, t);
            r as u32 + g as u32 + b as u32
        };
        assert!(lum(Token::Bg) < lum(Token::Surface));
        assert!(lum(Token::Surface) < lum(Token::SurfaceRaised));
        assert!(lum(Token::SurfaceRaised) <= lum(Token::SurfaceHover));
        // 发丝线必须与 surface 可区分（边框可见性）
        assert_ne!(
            color(Mode::Night, Token::Hairline),
            color(Mode::Night, Token::Surface)
        );
    }

    #[test]
    fn rgb_parses_all_tokens() {
        for token in [
            Token::Bg,
            Token::Surface,
            Token::SurfaceRaised,
            Token::SurfaceHover,
            Token::Hairline,
            Token::Fg,
            Token::Fg60,
            Token::Fg40,
            Token::Fg25,
            Token::Accent,
            Token::Safe,
            Token::Caution,
            Token::Dangerous,
            Token::Destructive,
            Token::SynKeyword,
            Token::SynString,
        ] {
            let (r, g, b) = rgb(Mode::Day, token);
            assert!((r as u32) + (g as u32) + (b as u32) > 0);
        }
    }

    #[test]
    fn terminal_palette_tracks_design_anchors() {
        let night = terminal_palette(Mode::Night);
        assert_eq!(night.background, "#0f1011", "夜终端=surface-1 抬升面");
        assert_eq!(night.blue, "#5e6ad2", "ANSI 蓝槽=品牌点缀");
        assert_eq!(night.bright_blue, "#828fff");
        let day = terminal_palette(Mode::Day);
        assert_eq!(day.background, "#ffffff");
    }
}
