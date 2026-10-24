//! 设计系统令牌 → egui。调性 = iA Writer（Information Architects）：
//! 纸感中性色（昼=暖纸白 / 夜=软石墨）、单一克制蓝作唯一点缀、排版承担层级
//! （字重与灰阶，而非填充盒）、去铬（无阴影、无 pill、发丝线已是最重装饰）、打字机式平静。
//! 布局不随调性变化（多窗口/标签/终端/输入行，域 18）。令牌与 iac/src/theme.rs 同名同步。

use eframe::egui::{Color32, Context, FontData, FontDefinitions, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Day,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // 令牌表全集，部分槽位随面板接线启用
pub enum Token {
    /// 画布（纸：昼=暖纸白，夜=软石墨；iA Writer 的"纸面"）
    Bg,
    /// 抬升面一级（条区容器；与画布对比刻意极低——层级靠留白而非盒子）
    Surface,
    /// 抬升面二级（保留槽位：仅最少场合使用）
    SurfaceRaised,
    /// 悬停/反馈面
    SurfaceHover,
    /// 发丝线（最重的装饰）
    Hairline,
    /// 正文（墨：暖调，非蓝白）
    Fg,
    /// 次级文本
    Fg60,
    /// 三级文本
    Fg40,
    /// 弱化文本
    Fg25,
    /// 唯一点缀：iA 静蓝（仅光标/焦点/活动位/链接）
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

/// token → (R,G,B)。夜=软石墨系（#1E1F23 底，微暖），昼=暖纸系（#F7F5F0 底）；
/// 语义色统一降饱和（鼠尾草绿/沙金/陶土/柔红），蓝槽=品牌静蓝。
pub fn rgb(mode: Mode, token: Token) -> (u8, u8, u8) {
    let hex: &str = match (mode, token) {
        (_, Token::Bg) => match mode {
            Mode::Day => "F7F5F0",
            Mode::Night => "242322",
        },
        (_, Token::Surface) => match mode {
            Mode::Day => "F1EEE8",
            Mode::Night => "292826",
        },
        (_, Token::SurfaceRaised) => match mode {
            Mode::Day => "FBFAF6",
            Mode::Night => "2C2B29",
        },
        (_, Token::SurfaceHover) => match mode {
            Mode::Day => "EBE8E1",
            Mode::Night => "312F2D",
        },
        (_, Token::Hairline) => match mode {
            Mode::Day => "E2DED5",
            Mode::Night => "383633",
        },
        (_, Token::Fg) => match mode {
            Mode::Day => "292926",
            Mode::Night => "D9D7D0",
        },
        (_, Token::Fg60) => match mode {
            Mode::Day => "6D6B63",
            Mode::Night => "9D9B93",
        },
        (_, Token::Fg40) => match mode {
            Mode::Day => "9B988F",
            Mode::Night => "716F67",
        },
        (_, Token::Fg25) => match mode {
            Mode::Day => "C5C1B7",
            Mode::Night => "4D4C46",
        },
        // iA 静蓝：昼=墨水蓝，夜=粉末蓝（降饱和，非霓虹）
        (_, Token::Accent) => match mode {
            Mode::Day => "3B6FD4",
            Mode::Night => "6E9CE8",
        },
        (_, Token::Safe) => match mode {
            Mode::Day => "4E7A56",
            Mode::Night => "7FB98A",
        },
        (_, Token::Caution) => match mode {
            Mode::Day => "8F6B2B",
            Mode::Night => "C9A26B",
        },
        (_, Token::Dangerous) => match mode {
            Mode::Day => "B25A3A",
            Mode::Night => "CE8370",
        },
        (_, Token::Destructive) => match mode {
            Mode::Day => "AF4B44",
            Mode::Night => "C97070",
        },
        (_, Token::SynKeyword) => match mode {
            Mode::Day => "7E5CC0",
            Mode::Night => "A78BB5",
        },
        (_, Token::SynString) => match mode {
            Mode::Day => "3E7D74",
            Mode::Night => "74A8A4",
        },
    };
    (
        u8::from_str_radix(&hex[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
    )
}

/// 全局样式：iA Writer 去铬——常态控件透明（文本即控件），无阴影；
/// 悬停/激活才有极轻的面反馈；圆角收敛到 4~6px；间距安静（8/6）。
pub fn apply_style(ctx: &Context, mode: Mode) {
    let build = || {
        let ghost = |fg: Color32| egui::style::WidgetVisuals {
            bg_fill: Color32::TRANSPARENT,
            weak_bg_fill: Color32::TRANSPARENT,
            bg_stroke: egui::Stroke::NONE,
            corner_radius: egui::CornerRadius::same(4),
            fg_stroke: egui::Stroke::new(1.0_f32, fg),
            expansion: 0.0,
        };
        let soft = |fg: Color32, bg: Color32, stroke: egui::Stroke| egui::style::WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: stroke,
            corner_radius: egui::CornerRadius::same(4),
            fg_stroke: egui::Stroke::new(1.0_f32, fg),
            expansion: 0.0,
        };
        egui::Visuals {
            panel_fill: color(mode, Token::Bg),
            window_fill: color(mode, Token::Bg),
            extreme_bg_color: color(mode, Token::Bg),
            faint_bg_color: color(mode, Token::SurfaceHover),
            override_text_color: Some(color(mode, Token::Fg)),
            hyperlink_color: color(mode, Token::Accent),
            selection: egui::style::Selection {
                bg_fill: color(mode, Token::Accent).gamma_multiply(0.16),
                stroke: egui::Stroke::NONE,
            },
            window_stroke: egui::Stroke::new(1.0_f32, color(mode, Token::Hairline)),
            window_corner_radius: egui::CornerRadius::same(6),
            menu_corner_radius: egui::CornerRadius::same(6),
            window_shadow: egui::Shadow::NONE,
            widgets: egui::style::Widgets {
                noninteractive: ghost(color(mode, Token::Fg60)),
                inactive: ghost(color(mode, Token::Fg)),
                hovered: soft(
                    color(mode, Token::Fg),
                    color(mode, Token::SurfaceHover),
                    egui::Stroke::NONE,
                ),
                active: soft(
                    color(mode, Token::Fg),
                    color(mode, Token::SurfaceHover),
                    egui::Stroke::new(1.0_f32, color(mode, Token::Accent)),
                ),
                open: soft(
                    color(mode, Token::Fg),
                    color(mode, Token::SurfaceHover),
                    egui::Stroke::NONE,
                ),
            },
            ..egui::Visuals::default()
        }
    };
    let mut style = Style {
        visuals: build(),
        ..Style::default()
    };
    // 间距安静；菜单内边距 4
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.menu_margin = egui::Margin::symmetric(4, 4);
    // 排版即层级：字号阶梯平缓（iA 节奏），等宽为默认工作字体
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(16.0)),
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

/// 终端 ANSI：与纸面同底（夜=软石墨、昼=暖纸），ANSI 降饱和的暖调集，
/// 蓝槽=品牌静蓝（终端与面板同一支点缀）。
pub fn terminal_palette(mode: Mode) -> egui_term::ColorPalette {
    if mode == Mode::Night {
        return egui_term::ColorPalette {
            foreground: "#D9D7D0".into(),
            background: "#242322".into(),
            bright_foreground: Some("#EDEBE4".into()),
            dim_foreground: "#9C9A92".into(),
            black: "#2A2927".into(),
            red: "#C97070".into(),
            green: "#7FB98A".into(),
            yellow: "#C9A26B".into(),
            blue: "#6E9CE8".into(),
            magenta: "#A78BB5".into(),
            cyan: "#74A8A4".into(),
            white: "#9C9A92".into(),
            bright_black: "#3E3C39".into(),
            bright_red: "#D98282".into(),
            bright_green: "#93C79E".into(),
            bright_yellow: "#D8B480".into(),
            bright_blue: "#86B0F0".into(),
            bright_magenta: "#BBA3C6".into(),
            bright_cyan: "#8CBAB6".into(),
            bright_white: "#EDEBE4".into(),
            dim_black: "#262522".into(),
            dim_red: "#A65E5E".into(),
            dim_green: "#6B9A75".into(),
            dim_yellow: "#A98A5C".into(),
            dim_blue: "#5C84C2".into(),
            dim_magenta: "#8C7699".into(),
            dim_cyan: "#628F8B".into(),
            dim_white: "#83817A".into(),
        };
    }
    egui_term::ColorPalette {
        foreground: "#292926".into(),
        background: "#F7F5F0".into(),
        bright_foreground: Some("#292926".into()),
        dim_foreground: "#9B988F".into(),
        black: "#E8E5DE".into(),
        red: "#AF4B44".into(),
        green: "#4E7A56".into(),
        yellow: "#8F6B2B".into(),
        blue: "#3B6FD4".into(),
        magenta: "#7E5CC0".into(),
        cyan: "#3E7D74".into(),
        white: "#6D6B63".into(),
        bright_black: "#C5C1B7".into(),
        bright_red: "#9E443E".into(),
        bright_green: "#456B4C".into(),
        bright_yellow: "#7D5E26".into(),
        bright_blue: "#3461BA".into(),
        bright_magenta: "#6E4FAB".into(),
        bright_cyan: "#376E66".into(),
        bright_white: "#292926".into(),
        dim_black: "#EFEDE6".into(),
        dim_red: "#B86058".into(),
        dim_green: "#5F8867".into(),
        dim_yellow: "#9E7B39".into(),
        dim_blue: "#5A82DC".into(),
        dim_magenta: "#9074CC".into(),
        dim_cyan: "#55918A".into(),
        dim_white: "#9B988F".into(),
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
        // iA 调性锚点：夜=软石墨纸、昼=暖纸、点缀=静蓝
        assert_eq!(
            color(Mode::Night, Token::Bg),
            Color32::from_rgb(0x24, 0x23, 0x22)
        );
        assert_eq!(
            color(Mode::Day, Token::Bg),
            Color32::from_rgb(0xF7, 0xF5, 0xF0)
        );
        assert_eq!(
            color(Mode::Day, Token::Accent),
            Color32::from_rgb(0x3B, 0x6F, 0xD4)
        );
    }

    #[test]
    fn surface_ladder_stays_quiet() {
        // iA 调性：阶梯存在但对比刻意低（层级靠留白），发丝线仅作最重装饰
        let lum = |t: Token| {
            let (r, g, b) = rgb(Mode::Night, t);
            r as u32 + g as u32 + b as u32
        };
        assert!(lum(Token::Bg) < lum(Token::Surface));
        assert!(lum(Token::Surface) < lum(Token::SurfaceHover));
        // 对比度刻意小：相邻阶梯亮度差 < 32（安静，非 SaaS 强分层）
        assert!(lum(Token::Surface) - lum(Token::Bg) < 32);
        assert_ne!(
            color(Mode::Night, Token::Hairline),
            color(Mode::Night, Token::Surface)
        );
    }

    #[test]
    fn night_bg_is_soft_not_pure_black() {
        // iA 夜=软石墨：三通道均在 0x18~0x28 区间且微暖（R≥B），不是 Linear 式纯黑
        let (r, g, b) = rgb(Mode::Night, Token::Bg);
        assert!((0x18..=0x28).contains(&r));
        assert!((0x18..=0x28).contains(&g));
        assert!((0x18..=0x28).contains(&b));
        assert!(r >= b, "微暖：R ≥ B");
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
    fn terminal_shares_paper_with_panels() {
        // 终端与面板同一张纸（无色缝）；蓝槽=同一支静蓝
        let night = terminal_palette(Mode::Night);
        assert_eq!(night.background, "#242322");
        assert_eq!(night.foreground, "#D9D7D0");
        assert_eq!(night.blue, "#6E9CE8");
        let day = terminal_palette(Mode::Day);
        assert_eq!(day.background, "#F7F5F0");
        assert_eq!(day.blue, "#3B6FD4");
    }
}
