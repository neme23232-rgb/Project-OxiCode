use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeKind {
    #[default]
    #[serde(alias = "beige", alias = "opencode-beige")]
    OpenCodeBeige,
    #[serde(alias = "classic", alias = "opencode-classic")]
    OpenCodeClassic,
    Monochrome,
    TokyoNight,
    GruvboxDark,
    Dracula,
    Nord,
    CatppuccinMocha,
    RosePine,
    Kanagawa,
    OneDark,
    Everforest,
    Vesper,
    GithubDark,
    Terminal,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Palette {
    pub bg: Option<Rgb>,
    pub bg_panel: Option<Rgb>,
    pub bg_element: Rgb,
    pub border: Rgb,
    pub border_active: Rgb,
    pub primary: Rgb,
    pub accent: Rgb,
    pub secondary: Rgb,
    pub text: Option<Rgb>,
    pub muted: Rgb,
    pub green: Rgb,
    pub red: Rgb,
    pub yellow: Rgb,
    pub cyan: Rgb,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeCss {
    pub id: &'static str,
    pub label: &'static str,
    pub bg: String,
    pub bg_panel: String,
    pub bg_element: String,
    pub border: String,
    pub border_active: String,
    pub primary: String,
    pub accent: String,
    pub secondary: String,
    pub text: String,
    pub muted: String,
    pub green: String,
    pub red: String,
    pub yellow: String,
    pub cyan: String,
}

impl ThemeKind {
    pub fn all() -> &'static [ThemeKind] {
        &[
            ThemeKind::OpenCodeBeige,
            ThemeKind::OpenCodeClassic,
            ThemeKind::Monochrome,
            ThemeKind::TokyoNight,
            ThemeKind::GruvboxDark,
            ThemeKind::Dracula,
            ThemeKind::Nord,
            ThemeKind::CatppuccinMocha,
            ThemeKind::RosePine,
            ThemeKind::Kanagawa,
            ThemeKind::OneDark,
            ThemeKind::Everforest,
            ThemeKind::Vesper,
            ThemeKind::GithubDark,
            ThemeKind::Terminal,
        ]
    }

    pub fn is_monochrome(&self) -> bool {
        matches!(self, ThemeKind::Monochrome)
    }

    pub fn name(&self) -> &'static str {
        match self {
            ThemeKind::OpenCodeBeige => "beige",
            ThemeKind::OpenCodeClassic => "classic",
            ThemeKind::Monochrome => "monochrome",
            ThemeKind::TokyoNight => "tokyo-night",
            ThemeKind::GruvboxDark => "gruvbox-dark",
            ThemeKind::Dracula => "dracula",
            ThemeKind::Nord => "nord",
            ThemeKind::CatppuccinMocha => "catppuccin-mocha",
            ThemeKind::RosePine => "rose-pine",
            ThemeKind::Kanagawa => "kanagawa",
            ThemeKind::OneDark => "one-dark",
            ThemeKind::Everforest => "everforest",
            ThemeKind::Vesper => "vesper",
            ThemeKind::GithubDark => "github-dark",
            ThemeKind::Terminal => "terminal",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ThemeKind::OpenCodeBeige => "Beige (Default)",
            ThemeKind::OpenCodeClassic => "Classic",
            ThemeKind::Monochrome => "Monochrome",
            ThemeKind::TokyoNight => "Tokyo Night",
            ThemeKind::GruvboxDark => "Gruvbox Dark",
            ThemeKind::Dracula => "Dracula",
            ThemeKind::Nord => "Nord",
            ThemeKind::CatppuccinMocha => "Catppuccin Mocha",
            ThemeKind::RosePine => "Rosé Pine",
            ThemeKind::Kanagawa => "Kanagawa",
            ThemeKind::OneDark => "One Dark",
            ThemeKind::Everforest => "Everforest",
            ThemeKind::Vesper => "Vesper",
            ThemeKind::GithubDark => "GitHub Dark",
            ThemeKind::Terminal => "Terminal Native",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "beige" | "opencode-beige" | "default" => Some(ThemeKind::OpenCodeBeige),
            "classic" | "opencode-classic" | "opencode" => Some(ThemeKind::OpenCodeClassic),
            "monochrome" | "mono" | "bw" | "black-white" => Some(ThemeKind::Monochrome),
            "tokyo-night" | "tokyonight" | "tokyo" => Some(ThemeKind::TokyoNight),
            "gruvbox-dark" | "gruvbox" => Some(ThemeKind::GruvboxDark),
            "dracula" => Some(ThemeKind::Dracula),
            "nord" => Some(ThemeKind::Nord),
            "catppuccin-mocha" | "catppuccin" => Some(ThemeKind::CatppuccinMocha),
            "rose-pine" | "rosepine" => Some(ThemeKind::RosePine),
            "kanagawa" => Some(ThemeKind::Kanagawa),
            "one-dark" | "onedark" => Some(ThemeKind::OneDark),
            "everforest" => Some(ThemeKind::Everforest),
            "vesper" => Some(ThemeKind::Vesper),
            "github-dark" | "github" => Some(ThemeKind::GithubDark),
            "terminal" | "native" => Some(ThemeKind::Terminal),
            _ => None,
        }
    }

    pub fn to_css(&self) -> ThemeCss {
        let p = self.palette();
        ThemeCss {
            id: self.name(),
            label: self.label(),
            bg: p.bg.unwrap_or(Rgb::new(13, 12, 11)).hex(),
            bg_panel: p.bg_panel.unwrap_or(Rgb::new(22, 20, 18)).hex(),
            bg_element: p.bg_element.hex(),
            border: p.border.hex(),
            border_active: p.border_active.hex(),
            primary: p.primary.hex(),
            accent: p.accent.hex(),
            secondary: p.secondary.hex(),
            text: p.text.unwrap_or(Rgb::new(235, 228, 216)).hex(),
            muted: p.muted.hex(),
            green: p.green.hex(),
            red: p.red.hex(),
            yellow: p.yellow.hex(),
            cyan: p.cyan.hex(),
        }
    }

    pub fn palette(&self) -> Palette {
        match self {
            ThemeKind::OpenCodeBeige => Palette {
                bg: Some(Rgb::new(13, 12, 11)),
                bg_panel: Some(Rgb::new(22, 20, 18)),
                bg_element: Rgb::new(33, 30, 26),
                border: Rgb::new(68, 61, 53),
                border_active: Rgb::new(232, 216, 190),
                primary: Rgb::new(232, 216, 190),
                accent: Rgb::new(250, 178, 131),
                secondary: Rgb::new(122, 162, 247),
                text: Some(Rgb::new(235, 228, 216)),
                muted: Rgb::new(138, 129, 117),
                green: Rgb::new(127, 216, 143),
                red: Rgb::new(224, 108, 117),
                yellow: Rgb::new(229, 192, 123),
                cyan: Rgb::new(166, 209, 214),
            },
            ThemeKind::OpenCodeClassic => Palette {
                bg: Some(Rgb::new(10, 10, 10)),
                bg_panel: Some(Rgb::new(20, 20, 20)),
                bg_element: Rgb::new(30, 30, 30),
                border: Rgb::new(72, 72, 72),
                border_active: Rgb::new(250, 178, 131),
                primary: Rgb::new(250, 178, 131),
                accent: Rgb::new(157, 124, 216),
                secondary: Rgb::new(92, 156, 245),
                text: Some(Rgb::new(235, 235, 235)),
                muted: Rgb::new(128, 128, 128),
                green: Rgb::new(127, 216, 143),
                red: Rgb::new(224, 108, 117),
                yellow: Rgb::new(229, 192, 123),
                cyan: Rgb::new(86, 182, 194),
            },
            ThemeKind::Monochrome => Palette {
                bg: Some(Rgb::new(10, 10, 12)),
                bg_panel: Some(Rgb::new(18, 18, 22)),
                bg_element: Rgb::new(34, 34, 40),
                border: Rgb::new(68, 68, 76),
                border_active: Rgb::new(244, 244, 248),
                primary: Rgb::new(250, 250, 252),
                accent: Rgb::new(218, 218, 226),
                secondary: Rgb::new(182, 182, 192),
                text: Some(Rgb::new(236, 236, 240)),
                muted: Rgb::new(124, 124, 134),
                // Keep diffs & change indicators colorful
                green: Rgb::new(120, 222, 138),
                red: Rgb::new(236, 106, 118),
                yellow: Rgb::new(235, 198, 110),
                cyan: Rgb::new(204, 204, 214),
            },
            ThemeKind::TokyoNight => Palette {
                bg: Some(Rgb::new(26, 27, 38)),
                bg_panel: Some(Rgb::new(22, 22, 30)),
                bg_element: Rgb::new(41, 46, 66),
                border: Rgb::new(65, 72, 104),
                border_active: Rgb::new(122, 162, 247),
                primary: Rgb::new(122, 162, 247),
                accent: Rgb::new(187, 154, 247),
                secondary: Rgb::new(125, 207, 255),
                text: Some(Rgb::new(169, 177, 214)),
                muted: Rgb::new(86, 95, 137),
                green: Rgb::new(158, 206, 106),
                red: Rgb::new(247, 118, 142),
                yellow: Rgb::new(224, 175, 104),
                cyan: Rgb::new(125, 207, 255),
            },
            ThemeKind::GruvboxDark => Palette {
                bg: Some(Rgb::new(40, 40, 40)),
                bg_panel: Some(Rgb::new(29, 32, 33)),
                bg_element: Rgb::new(60, 56, 54),
                border: Rgb::new(102, 92, 84),
                border_active: Rgb::new(254, 128, 25),
                primary: Rgb::new(235, 219, 178),
                accent: Rgb::new(254, 128, 25),
                secondary: Rgb::new(131, 165, 152),
                text: Some(Rgb::new(235, 219, 178)),
                muted: Rgb::new(146, 131, 116),
                green: Rgb::new(184, 187, 38),
                red: Rgb::new(251, 73, 52),
                yellow: Rgb::new(250, 189, 47),
                cyan: Rgb::new(142, 192, 124),
            },
            ThemeKind::Dracula => Palette {
                bg: Some(Rgb::new(40, 42, 54)),
                bg_panel: Some(Rgb::new(33, 34, 44)),
                bg_element: Rgb::new(68, 71, 90),
                border: Rgb::new(98, 114, 164),
                border_active: Rgb::new(255, 121, 198),
                primary: Rgb::new(189, 147, 249),
                accent: Rgb::new(255, 121, 198),
                secondary: Rgb::new(139, 233, 253),
                text: Some(Rgb::new(248, 248, 242)),
                muted: Rgb::new(98, 114, 164),
                green: Rgb::new(80, 250, 123),
                red: Rgb::new(255, 85, 85),
                yellow: Rgb::new(241, 250, 140),
                cyan: Rgb::new(139, 233, 253),
            },
            ThemeKind::Nord => Palette {
                bg: Some(Rgb::new(46, 52, 64)),
                bg_panel: Some(Rgb::new(36, 41, 51)),
                bg_element: Rgb::new(59, 66, 82),
                border: Rgb::new(76, 86, 106),
                border_active: Rgb::new(136, 192, 208),
                primary: Rgb::new(136, 192, 208),
                accent: Rgb::new(129, 161, 193),
                secondary: Rgb::new(94, 129, 172),
                text: Some(Rgb::new(216, 222, 233)),
                muted: Rgb::new(107, 118, 141),
                green: Rgb::new(163, 190, 140),
                red: Rgb::new(191, 97, 106),
                yellow: Rgb::new(235, 203, 139),
                cyan: Rgb::new(143, 188, 187),
            },
            ThemeKind::CatppuccinMocha => Palette {
                bg: Some(Rgb::new(30, 30, 46)),
                bg_panel: Some(Rgb::new(24, 24, 37)),
                bg_element: Rgb::new(49, 50, 68),
                border: Rgb::new(88, 91, 112),
                border_active: Rgb::new(203, 166, 247),
                primary: Rgb::new(203, 166, 247),
                accent: Rgb::new(245, 194, 231),
                secondary: Rgb::new(137, 180, 250),
                text: Some(Rgb::new(205, 214, 244)),
                muted: Rgb::new(147, 153, 178),
                green: Rgb::new(166, 227, 161),
                red: Rgb::new(243, 139, 168),
                yellow: Rgb::new(249, 226, 175),
                cyan: Rgb::new(148, 226, 213),
            },
            ThemeKind::RosePine => Palette {
                bg: Some(Rgb::new(25, 23, 36)),
                bg_panel: Some(Rgb::new(31, 29, 46)),
                bg_element: Rgb::new(38, 35, 58),
                border: Rgb::new(82, 79, 103),
                border_active: Rgb::new(235, 188, 186),
                primary: Rgb::new(235, 188, 186),
                accent: Rgb::new(196, 167, 231),
                secondary: Rgb::new(156, 207, 216),
                text: Some(Rgb::new(224, 222, 244)),
                muted: Rgb::new(110, 106, 134),
                green: Rgb::new(156, 207, 216),
                red: Rgb::new(235, 111, 146),
                yellow: Rgb::new(246, 193, 119),
                cyan: Rgb::new(49, 116, 143),
            },
            ThemeKind::Kanagawa => Palette {
                bg: Some(Rgb::new(31, 31, 40)),
                bg_panel: Some(Rgb::new(22, 22, 29)),
                bg_element: Rgb::new(42, 42, 55),
                border: Rgb::new(84, 84, 109),
                border_active: Rgb::new(220, 215, 186),
                primary: Rgb::new(220, 215, 186),
                accent: Rgb::new(149, 127, 184),
                secondary: Rgb::new(126, 156, 216),
                text: Some(Rgb::new(220, 215, 186)),
                muted: Rgb::new(114, 113, 105),
                green: Rgb::new(152, 187, 108),
                red: Rgb::new(195, 64, 67),
                yellow: Rgb::new(230, 195, 132),
                cyan: Rgb::new(122, 168, 159),
            },
            ThemeKind::OneDark => Palette {
                bg: Some(Rgb::new(40, 44, 52)),
                bg_panel: Some(Rgb::new(33, 37, 43)),
                bg_element: Rgb::new(53, 59, 69),
                border: Rgb::new(75, 82, 99),
                border_active: Rgb::new(97, 175, 239),
                primary: Rgb::new(97, 175, 239),
                accent: Rgb::new(198, 120, 221),
                secondary: Rgb::new(86, 182, 194),
                text: Some(Rgb::new(171, 178, 191)),
                muted: Rgb::new(92, 99, 112),
                green: Rgb::new(152, 195, 121),
                red: Rgb::new(224, 108, 117),
                yellow: Rgb::new(229, 192, 123),
                cyan: Rgb::new(86, 182, 194),
            },
            ThemeKind::Everforest => Palette {
                bg: Some(Rgb::new(45, 53, 59)),
                bg_panel: Some(Rgb::new(35, 42, 46)),
                bg_element: Rgb::new(55, 65, 69),
                border: Rgb::new(86, 99, 95),
                border_active: Rgb::new(167, 192, 128),
                primary: Rgb::new(167, 192, 128),
                accent: Rgb::new(219, 188, 127),
                secondary: Rgb::new(127, 187, 179),
                text: Some(Rgb::new(211, 198, 170)),
                muted: Rgb::new(133, 146, 137),
                green: Rgb::new(167, 192, 128),
                red: Rgb::new(230, 126, 128),
                yellow: Rgb::new(219, 188, 127),
                cyan: Rgb::new(131, 192, 146),
            },
            ThemeKind::Vesper => Palette {
                bg: Some(Rgb::new(16, 16, 16)),
                bg_panel: Some(Rgb::new(24, 24, 24)),
                bg_element: Rgb::new(36, 36, 36),
                border: Rgb::new(64, 64, 64),
                border_active: Rgb::new(255, 199, 153),
                primary: Rgb::new(255, 199, 153),
                accent: Rgb::new(153, 255, 228),
                secondary: Rgb::new(255, 199, 153),
                text: Some(Rgb::new(255, 255, 255)),
                muted: Rgb::new(126, 126, 126),
                green: Rgb::new(153, 255, 228),
                red: Rgb::new(255, 128, 128),
                yellow: Rgb::new(255, 199, 153),
                cyan: Rgb::new(153, 255, 228),
            },
            ThemeKind::GithubDark => Palette {
                bg: Some(Rgb::new(13, 17, 23)),
                bg_panel: Some(Rgb::new(22, 27, 34)),
                bg_element: Rgb::new(33, 38, 45),
                border: Rgb::new(48, 54, 61),
                border_active: Rgb::new(88, 166, 255),
                primary: Rgb::new(88, 166, 255),
                accent: Rgb::new(210, 168, 255),
                secondary: Rgb::new(121, 192, 255),
                text: Some(Rgb::new(201, 209, 217)),
                muted: Rgb::new(139, 148, 158),
                green: Rgb::new(63, 185, 80),
                red: Rgb::new(248, 81, 73),
                yellow: Rgb::new(210, 153, 34),
                cyan: Rgb::new(118, 227, 234),
            },
            ThemeKind::Terminal => Palette {
                bg: None,
                bg_panel: None,
                bg_element: Rgb::new(60, 60, 60),
                border: Rgb::new(90, 90, 90),
                border_active: Rgb::new(229, 192, 123),
                primary: Rgb::new(229, 192, 123),
                accent: Rgb::new(86, 182, 194),
                secondary: Rgb::new(92, 156, 245),
                text: None,
                muted: Rgb::new(140, 140, 140),
                green: Rgb::new(127, 216, 143),
                red: Rgb::new(224, 108, 117),
                yellow: Rgb::new(229, 192, 123),
                cyan: Rgb::new(86, 182, 194),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simplified_opencode_and_monochrome_themes() {
        assert_eq!(ThemeKind::parse("beige"), Some(ThemeKind::OpenCodeBeige));
        assert_eq!(
            ThemeKind::parse("opencode-beige"),
            Some(ThemeKind::OpenCodeBeige)
        );
        assert_eq!(ThemeKind::parse("classic"), Some(ThemeKind::OpenCodeClassic));
        assert_eq!(
            ThemeKind::parse("opencode-classic"),
            Some(ThemeKind::OpenCodeClassic)
        );
        assert_eq!(ThemeKind::parse("monochrome"), Some(ThemeKind::Monochrome));
        assert_eq!(ThemeKind::parse("mono"), Some(ThemeKind::Monochrome));
        assert_eq!(ThemeKind::OpenCodeBeige.name(), "beige");
        assert_eq!(ThemeKind::OpenCodeClassic.name(), "classic");
        assert_eq!(ThemeKind::Monochrome.name(), "monochrome");
        assert!(ThemeKind::Monochrome.is_monochrome());
        assert!(!ThemeKind::OpenCodeBeige.is_monochrome());
    }

    #[test]
    fn monochrome_palette_keeps_only_change_colors() {
        let pal = ThemeKind::Monochrome.palette();
        assert!(pal.green.g > pal.green.r && pal.green.g > pal.green.b);
        assert!(pal.red.r > pal.red.g && pal.red.r > pal.red.b);
        assert!(pal.yellow.r > pal.yellow.b && pal.yellow.g > pal.yellow.b);
        assert!(pal.primary.r >= 240 && pal.primary.g >= 240 && pal.primary.b >= 240);
    }
}
