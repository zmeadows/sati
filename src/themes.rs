use std::{
    io::{self, Write},
    str::FromStr,
};

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::styles::{DEFAULT_ACCENT, Styles};

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub enum BuiltinTheme {
    #[default]
    Default,
    GruvboxMaterial,
    CatppuccinMocha,
    TokyoNightStorm,
    KanagawaWave,
    RosePine,
    Nightfox,
    OneDark,
    GithubDark,
    EverforestMedium,
    CyberdreamMuted,
    Nord,
    Dracula,
    Darcula,
    Gruvbox,
    SolarizedDark,
    SolarizedLight,
    GruvboxLight,
    CatppuccinLatte,
    GithubLight,
    RosePineDawn,
    EverforestLight,
    ModusOperandi,
    EfSummer,
    Monokai,
}

impl FromStr for BuiltinTheme {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .find(|(_, id, _)| *id == name)
            .map(|(theme, _, _)| *theme)
            .ok_or_else(|| {
                format!(
                    "unknown theme: {name:?}; run `sati --list-themes` to see the available themes"
                )
            })
    }
}

impl TryFrom<String> for BuiltinTheme {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        name.parse()
    }
}

impl BuiltinTheme {
    #[rustfmt::skip]
    pub const ALL: &'static [(Self, &'static str, &'static str)] = &[
        (Self::Default, "default", "Default"),
        (Self::GruvboxMaterial, "gruvbox-material", "Gruvbox Material"),
        (Self::CatppuccinMocha, "catppuccin-mocha", "Catppuccin Mocha"),
        (Self::TokyoNightStorm, "tokyonight-storm", "Tokyo Night Storm"),
        (Self::KanagawaWave, "kanagawa-wave", "Kanagawa Wave"),
        (Self::RosePine, "rose-pine", "Rosé Pine"),
        (Self::Nightfox, "nightfox", "Nightfox"),
        (Self::OneDark, "onedark", "One Dark"),
        (Self::GithubDark, "github-dark", "GitHub Dark"),
        (Self::EverforestMedium, "everforest-medium", "Everforest Dark / Medium"),
        (Self::CyberdreamMuted, "cyberdream-muted", "Cyberdream Muted"),
        (Self::Nord, "nord", "Nord"),
        (Self::Dracula, "dracula", "Dracula"),
        (Self::Darcula, "darcula", "Darcula"),
        (Self::Gruvbox, "gruvbox", "Gruvbox Classic"),
        (Self::SolarizedDark, "solarized-dark", "Solarized Dark"),
        (Self::SolarizedLight, "solarized-light", "Solarized Light"),
        (Self::GruvboxLight, "gruvbox-light", "Gruvbox Light"),
        (Self::CatppuccinLatte, "catppuccin-latte", "Catppuccin Latte"),
        (Self::GithubLight, "github-light", "GitHub Light"),
        (Self::RosePineDawn, "rose-pine-dawn", "Rosé Pine Dawn"),
        (Self::EverforestLight, "everforest-light", "Everforest Light"),
        (Self::ModusOperandi, "modus-operandi", "Modus Operandi"),
        (Self::EfSummer, "ef-summer", "Ef Summer"),
        (Self::Monokai, "monokai", "Monokai"),
    ];

    pub fn id(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(theme, _, _)| *theme == self)
            .expect("registered theme")
            .1
    }

    pub fn display_name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(theme, _, _)| *theme == self)
            .expect("registered theme")
            .2
    }

    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL
            .iter()
            .position(|(theme, _, _)| *theme == self)
            .expect("registered theme");

        let next = if forward {
            (index + 1) % Self::ALL.len()
        } else {
            (index + Self::ALL.len() - 1) % Self::ALL.len()
        };

        Self::ALL[next].0
    }

    // Role defaults patch their resolved parent, allowing TOML overrides on that
    // parent to cascade before the child's theme defaults and overrides apply.
    #[rustfmt::skip]
    pub fn base_styles(self) -> Styles {
        match self {
            Self::Default => Styles {
                normal: Style::default().fg(Color::Reset).remove_modifier(Modifier::all()),
                muted: Style::default().add_modifier(Modifier::DIM),
                completed: Style::default(),
                border: Style::default(),
                accent: Style::default().fg(DEFAULT_ACCENT),
            },
            Self::GruvboxMaterial => {
                // Each palette role explicitly clears inherited modifiers.
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(212, 190, 152)).bg(Color::Rgb(40, 40, 40)),
                    muted: plain.fg(Color::Rgb(168, 153, 132)),
                    completed: plain.fg(Color::Rgb(146, 131, 116)),
                    border: plain.fg(Color::Rgb(90, 82, 76)),
                    accent: plain.fg(Color::Rgb(231, 138, 78)),
                }
            }
            Self::CatppuccinMocha => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(205, 214, 244)).bg(Color::Rgb(30, 30, 46)),
                    muted: plain.fg(Color::Rgb(166, 173, 200)),
                    completed: plain.fg(Color::Rgb(108, 112, 134)),
                    border: plain.fg(Color::Rgb(88, 91, 112)),
                    accent: plain.fg(Color::Rgb(203, 166, 247)),
                }
            }
            Self::TokyoNightStorm => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(192, 202, 245)).bg(Color::Rgb(36, 40, 59)),
                    muted: plain.fg(Color::Rgb(115, 122, 162)),
                    completed: plain.fg(Color::Rgb(86, 95, 137)),
                    border: plain.fg(Color::Rgb(84, 92, 126)),
                    accent: plain.fg(Color::Rgb(122, 162, 247)),
                }
            }
            Self::KanagawaWave => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(220, 215, 186)).bg(Color::Rgb(31, 31, 40)),
                    muted: plain.fg(Color::Rgb(200, 192, 147)),
                    completed: plain.fg(Color::Rgb(114, 113, 105)),
                    border: plain.fg(Color::Rgb(84, 84, 109)),
                    accent: plain.fg(Color::Rgb(126, 156, 216)),
                }
            }
            Self::RosePine => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(224, 222, 244)).bg(Color::Rgb(25, 23, 36)),
                    muted: plain.fg(Color::Rgb(144, 140, 170)),
                    completed: plain.fg(Color::Rgb(110, 106, 134)),
                    border: plain.fg(Color::Rgb(82, 79, 103)),
                    accent: plain.fg(Color::Rgb(235, 188, 186)),
                }
            }
            Self::Nightfox => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(205, 206, 207)).bg(Color::Rgb(25, 35, 48)),
                    muted: plain.fg(Color::Rgb(174, 175, 176)),
                    completed: plain.fg(Color::Rgb(115, 128, 145)),
                    border: plain.fg(Color::Rgb(57, 80, 109)),
                    accent: plain.fg(Color::Rgb(113, 156, 214)),
                }
            }
            Self::OneDark => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(171, 178, 191)).bg(Color::Rgb(40, 44, 52)),
                    muted: plain.fg(Color::Rgb(132, 139, 152)),
                    completed: plain.fg(Color::Rgb(92, 99, 112)),
                    border: plain.fg(Color::Rgb(57, 63, 74)),
                    accent: plain.fg(Color::Rgb(97, 175, 239)),
                }
            }
            Self::GithubDark => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(201, 209, 217)).bg(Color::Rgb(13, 17, 23)),
                    muted: plain.fg(Color::Rgb(139, 148, 158)),
                    completed: plain.fg(Color::Rgb(110, 118, 129)),
                    border: plain.fg(Color::Rgb(48, 54, 61)),
                    accent: plain.fg(Color::Rgb(88, 166, 255)),
                }
            }
            Self::EverforestMedium => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(211, 198, 170)).bg(Color::Rgb(45, 53, 59)),
                    muted: plain.fg(Color::Rgb(157, 169, 160)),
                    completed: plain.fg(Color::Rgb(122, 132, 120)),
                    border: plain.fg(Color::Rgb(79, 88, 94)),
                    accent: plain.fg(Color::Rgb(167, 192, 128)),
                }
            }
            Self::CyberdreamMuted => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(22, 24, 26)),
                    muted: plain.fg(Color::Rgb(155, 163, 178)),
                    completed: plain.fg(Color::Rgb(123, 132, 150)),
                    border: plain.fg(Color::Rgb(60, 64, 72)),
                    accent: plain.fg(Color::Rgb(120, 168, 232)),
                }
            }
            Self::Nord => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(216, 222, 233)).bg(Color::Rgb(46, 52, 64)),
                    muted: plain.fg(Color::Rgb(156, 168, 184)),
                    completed: plain.fg(Color::Rgb(76, 86, 106)),
                    border: plain.fg(Color::Rgb(67, 76, 94)),
                    accent: plain.fg(Color::Rgb(136, 192, 208)),
                }
            }
            Self::Dracula => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(248, 248, 242)).bg(Color::Rgb(40, 42, 54)),
                    muted: plain.fg(Color::Rgb(98, 114, 164)),
                    completed: plain.fg(Color::Rgb(75, 82, 99)),
                    border: plain.fg(Color::Rgb(68, 71, 90)),
                    accent: plain.fg(Color::Rgb(189, 147, 249)),
                }
            }
            Self::Darcula => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(169, 183, 198)).bg(Color::Rgb(43, 43, 43)),
                    muted: plain.fg(Color::Rgb(128, 128, 128)),
                    completed: plain.fg(Color::Rgb(96, 99, 102)),
                    border: plain.fg(Color::Rgb(96, 96, 96)),
                    accent: plain.fg(Color::Rgb(255, 198, 109)),
                }
            }
            Self::Gruvbox => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(235, 219, 178)).bg(Color::Rgb(40, 40, 40)),
                    muted: plain.fg(Color::Rgb(168, 153, 132)),
                    completed: plain.fg(Color::Rgb(146, 131, 116)),
                    border: plain.fg(Color::Rgb(102, 92, 84)),
                    accent: plain.fg(Color::Rgb(254, 128, 25)),
                }
            }
            Self::SolarizedDark => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(131, 148, 150)).bg(Color::Rgb(0, 43, 54)),
                    muted: plain.fg(Color::Rgb(101, 123, 131)),
                    completed: plain.fg(Color::Rgb(88, 110, 117)),
                    border: plain.fg(Color::Rgb(88, 110, 117)),
                    accent: plain.fg(Color::Rgb(38, 139, 210)),
                }
            }
            Self::SolarizedLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(101, 123, 131)).bg(Color::Rgb(253, 246, 227)),
                    muted: plain.fg(Color::Rgb(131, 148, 150)),
                    completed: plain.fg(Color::Rgb(147, 161, 161)),
                    border: plain.fg(Color::Rgb(147, 161, 161)),
                    accent: plain.fg(Color::Rgb(38, 139, 210)),
                }
            }
            Self::GruvboxLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(60, 56, 54)).bg(Color::Rgb(251, 241, 199)),
                    muted: plain.fg(Color::Rgb(124, 111, 100)),
                    completed: plain.fg(Color::Rgb(146, 131, 116)),
                    border: plain.fg(Color::Rgb(168, 153, 132)),
                    accent: plain.fg(Color::Rgb(175, 58, 3)),
                }
            }
            Self::CatppuccinLatte => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(76, 79, 105)).bg(Color::Rgb(239, 241, 245)),
                    muted: plain.fg(Color::Rgb(108, 111, 133)),
                    completed: plain.fg(Color::Rgb(156, 160, 176)),
                    border: plain.fg(Color::Rgb(172, 176, 190)),
                    accent: plain.fg(Color::Rgb(136, 57, 239)),
                }
            }
            Self::GithubLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(36, 41, 47)).bg(Color::Rgb(255, 255, 255)),
                    muted: plain.fg(Color::Rgb(87, 96, 106)),
                    completed: plain.fg(Color::Rgb(110, 119, 129)),
                    border: plain.fg(Color::Rgb(208, 215, 222)),
                    accent: plain.fg(Color::Rgb(9, 105, 218)),
                }
            }
            Self::RosePineDawn => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(70, 66, 97)).bg(Color::Rgb(250, 244, 237)),
                    muted: plain.fg(Color::Rgb(121, 117, 147)),
                    completed: plain.fg(Color::Rgb(152, 147, 165)),
                    border: plain.fg(Color::Rgb(206, 202, 205)),
                    accent: plain.fg(Color::Rgb(215, 130, 126)),
                }
            }
            Self::EverforestLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(92, 106, 114)).bg(Color::Rgb(253, 246, 227)),
                    muted: plain.fg(Color::Rgb(130, 145, 129)),
                    completed: plain.fg(Color::Rgb(166, 176, 160)),
                    border: plain.fg(Color::Rgb(147, 159, 145)),
                    accent: plain.fg(Color::Rgb(141, 161, 1)),
                }
            }
            Self::ModusOperandi => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255)),
                    muted: plain.fg(Color::Rgb(89, 89, 89)),
                    completed: plain.fg(Color::Rgb(89, 89, 89)),
                    border: plain.fg(Color::Rgb(159, 159, 159)),
                    accent: plain.fg(Color::Rgb(0, 49, 169)),
                }
            }
            Self::EfSummer => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(79, 64, 115)).bg(Color::Rgb(255, 242, 243)),
                    muted: plain.fg(Color::Rgb(120, 110, 116)),
                    completed: plain.fg(Color::Rgb(120, 110, 116)),
                    border: plain.fg(Color::Rgb(198, 187, 198)),
                    accent: plain.fg(Color::Rgb(203, 26, 170)),
                }
            }
            Self::Monokai => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(248, 248, 242)).bg(Color::Rgb(39, 40, 34)),
                    muted: plain.fg(Color::Rgb(144, 144, 138)),
                    completed: plain.fg(Color::Rgb(117, 113, 94)),
                    border: plain.fg(Color::Rgb(100, 100, 94)),
                    accent: plain.fg(Color::Rgb(249, 38, 114)),
                }
            }
        }
    }
}

// Prints one theme ID per line for `sati --list-themes`.
pub fn print_list() -> crate::storage::Result<()> {
    let mut contents = String::new();
    for (_, id, _) in BuiltinTheme::ALL {
        contents.push_str(id);
        contents.push('\n');
    }

    let mut stdout = io::stdout().lock();
    let written = stdout
        .write_all(contents.as_bytes())
        .and_then(|()| stdout.flush());

    // A reader that stops early (`sati --list-themes | head`) is not a failure.
    match written {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_named_theme_backgrounds_are_exact_and_inherited() {
        for (name, rgb) in [
            ("catppuccin-mocha", (30, 30, 46)),
            ("tokyonight-storm", (36, 40, 59)),
            ("kanagawa-wave", (31, 31, 40)),
            ("rose-pine", (25, 23, 36)),
            ("nightfox", (25, 35, 48)),
            ("onedark", (40, 44, 52)),
            ("gruvbox-material", (40, 40, 40)),
            ("github-dark", (13, 17, 23)),
            ("everforest-medium", (45, 53, 59)),
            ("cyberdream-muted", (22, 24, 26)),
            ("nord", (46, 52, 64)),
            ("dracula", (40, 42, 54)),
            ("darcula", (43, 43, 43)),
            ("gruvbox", (40, 40, 40)),
            ("solarized-dark", (0, 43, 54)),
            ("solarized-light", (253, 246, 227)),
            ("gruvbox-light", (251, 241, 199)),
            ("catppuccin-latte", (239, 241, 245)),
            ("github-light", (255, 255, 255)),
            ("rose-pine-dawn", (250, 244, 237)),
            ("everforest-light", (253, 246, 227)),
            ("modus-operandi", (255, 255, 255)),
            ("ef-summer", (255, 242, 243)),
            ("monokai", (39, 40, 34)),
        ] {
            let styles = Styles::resolve(
                name.parse().unwrap(),
                &crate::config::StyleOverrides::default(),
                None,
            );

            for role in [
                styles.normal,
                styles.muted,
                styles.completed,
                styles.border,
                styles.accent,
            ] {
                assert_eq!(role.bg, Some(Color::Rgb(rgb.0, rgb.1, rgb.2)), "{name}");
            }
        }

        assert_eq!(Styles::default().normal.bg, None);
    }

    #[test]
    fn registry_drives_both_cycle_directions_and_wraparound() {
        for (index, &(theme, id, display)) in BuiltinTheme::ALL.iter().enumerate() {
            assert_eq!(id.parse::<BuiltinTheme>().unwrap(), theme);
            assert_eq!(theme.display_name(), display);
            assert_eq!(theme.id(), id);
            assert_eq!(
                theme.cycle(true),
                BuiltinTheme::ALL[(index + 1) % BuiltinTheme::ALL.len()].0
            );
            assert_eq!(theme.cycle(true).cycle(false), theme);
            assert_eq!(theme.cycle(false).cycle(true), theme);
        }

        assert_eq!(BuiltinTheme::Default.cycle(false), BuiltinTheme::Monokai);
    }

    #[test]
    fn all_available_themes_parse_and_preserve_overrides_and_modifier_resets() {
        let names: Vec<_> = BuiltinTheme::ALL.iter().map(|(_, name, _)| *name).collect();
        assert_eq!(names.len(), 25);
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            25
        );

        for name in names {
            let theme = name.parse::<BuiltinTheme>().unwrap();
            let config: crate::config::Config = toml::from_str(&format!("theme='{name}'")).unwrap();
            assert_eq!(config.theme, theme);
            assert!(name.to_uppercase().parse::<BuiltinTheme>().is_err());

            let resolved = Styles::resolve(theme, &config.styles, None);

            if theme != BuiltinTheme::Default {
                for role in [
                    resolved.normal,
                    resolved.muted,
                    resolved.completed,
                    resolved.border,
                    resolved.accent,
                ] {
                    assert!(matches!(role.fg, Some(Color::Rgb(_, _, _))));
                    assert_eq!(role.bg, resolved.normal.bg);
                    assert!(role.add_modifier.is_empty());
                    assert_eq!(role.sub_modifier, Modifier::all());
                }
            }

            let config: crate::config::Config = toml::from_str(&format!("theme='{name}'\n[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers=['italic']\n[styles.accent]\nfg='red'\nmodifiers=['bold']")).unwrap();
            let resolved = Styles::resolve(config.theme, &config.styles, Some(Color::Blue));
            assert_eq!(resolved.normal.add_modifier, Modifier::BOLD);
            assert_eq!(resolved.muted.fg, Some(Color::Green));
            assert_eq!(resolved.muted.add_modifier, Modifier::ITALIC);
            assert_eq!(resolved.accent.fg, Some(Color::Blue));
            assert_eq!(resolved.accent.add_modifier, Modifier::BOLD);

            if theme != BuiltinTheme::Default {
                assert!(resolved.completed.add_modifier.is_empty());
                assert!(resolved.border.add_modifier.is_empty());
            }
        }
    }

    #[test]
    fn light_and_monokai_themes_have_exact_palettes() {
        #[rustfmt::skip]
        let palettes = [
            ("solarized-light", [(253, 246, 227), (101, 123, 131), (131, 148, 150), (147, 161, 161), (147, 161, 161), (38, 139, 210)]),
            ("gruvbox-light", [(251, 241, 199), (60, 56, 54), (124, 111, 100), (146, 131, 116), (168, 153, 132), (175, 58, 3)]),
            ("catppuccin-latte", [(239, 241, 245), (76, 79, 105), (108, 111, 133), (156, 160, 176), (172, 176, 190), (136, 57, 239)]),
            ("github-light", [(255, 255, 255), (36, 41, 47), (87, 96, 106), (110, 119, 129), (208, 215, 222), (9, 105, 218)]),
            ("rose-pine-dawn", [(250, 244, 237), (70, 66, 97), (121, 117, 147), (152, 147, 165), (206, 202, 205), (215, 130, 126)]),
            ("everforest-light", [(253, 246, 227), (92, 106, 114), (130, 145, 129), (166, 176, 160), (147, 159, 145), (141, 161, 1)]),
            ("modus-operandi", [(255, 255, 255), (0, 0, 0), (89, 89, 89), (89, 89, 89), (159, 159, 159), (0, 49, 169)]),
            ("ef-summer", [(255, 242, 243), (79, 64, 115), (120, 110, 116), (120, 110, 116), (198, 187, 198), (203, 26, 170)]),
            ("monokai", [(39, 40, 34), (248, 248, 242), (144, 144, 138), (117, 113, 94), (100, 100, 94), (249, 38, 114)]),
        ];

        for (name, [background, normal, muted, completed, border, accent]) in palettes {
            let styles = Styles::resolve(
                name.parse().unwrap(),
                &crate::config::StyleOverrides::default(),
                None,
            );

            for (role, (r, g, b)) in [
                (styles.normal, normal),
                (styles.muted, muted),
                (styles.completed, completed),
                (styles.border, border),
                (styles.accent, accent),
            ] {
                assert_eq!(role.fg, Some(Color::Rgb(r, g, b)), "{name}");
                assert_eq!(
                    role.bg,
                    Some(Color::Rgb(background.0, background.1, background.2)),
                    "{name}"
                );
                assert!(role.add_modifier.is_empty(), "{name}");
                assert_eq!(role.sub_modifier, Modifier::all(), "{name}");
            }
        }
    }

    #[test]
    fn gruvbox_material_has_exact_palette_and_clears_inherited_modifiers() {
        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'").unwrap();
        assert_eq!(config.theme, BuiltinTheme::GruvboxMaterial);
        assert_eq!(
            "gruvbox-material".parse::<BuiltinTheme>().unwrap(),
            config.theme
        );
        assert!("Gruvbox-Material".parse::<BuiltinTheme>().is_err());

        let styles = Styles::resolve(config.theme, &config.styles, None);

        for (role, color) in [
            (styles.normal, Color::Rgb(212, 190, 152)),
            (styles.muted, Color::Rgb(168, 153, 132)),
            (styles.completed, Color::Rgb(146, 131, 116)),
            (styles.border, Color::Rgb(90, 82, 76)),
            (styles.accent, Color::Rgb(231, 138, 78)),
        ] {
            assert_eq!(role.fg, Some(color));
            assert_eq!(role.bg, Some(Color::Rgb(40, 40, 40)));
            assert!(role.add_modifier.is_empty());
            assert_eq!(role.sub_modifier, Modifier::all());
        }

        let inherited: crate::config::Config =
            toml::from_str("theme='gruvbox-material'\n[styles.normal]\nmodifiers=['bold']")
                .unwrap();
        let inherited = Styles::resolve(inherited.theme, &inherited.styles, None);

        assert_eq!(inherited.normal.add_modifier, Modifier::BOLD);

        for role in [
            inherited.muted,
            inherited.completed,
            inherited.border,
            inherited.accent,
        ] {
            assert!(role.add_modifier.is_empty());
        }

        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'\n[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers=['italic']\n[styles.completed]\nmodifiers=['dim']\n[styles.border]\nmodifiers=[]\n[styles.accent]\nfg='blue'\nmodifiers=['bold']").unwrap();
        let styles = Styles::resolve(config.theme, &config.styles, Some(Color::Red));
        assert_eq!(styles.normal.add_modifier, Modifier::BOLD);
        assert_eq!(styles.muted.fg, Some(Color::Green));
        assert_eq!(styles.muted.add_modifier, Modifier::ITALIC);
        assert_eq!(styles.completed.add_modifier, Modifier::DIM);
        assert!(styles.border.add_modifier.is_empty());
        assert_eq!(styles.accent.fg, Some(Color::Red));
        assert_eq!(styles.accent.add_modifier, Modifier::BOLD);
    }

    #[test]
    fn names_are_strict_and_default_preserves_the_existing_palette() {
        assert_eq!(
            "default".parse::<BuiltinTheme>().unwrap(),
            BuiltinTheme::Default
        );

        for name in ["", "Default", "DEFAULT", "not-a-theme", " default"] {
            let error = name.parse::<BuiltinTheme>().unwrap_err();
            assert!(error.contains("sati --list-themes"), "{error}");
        }

        let omitted: crate::config::Config = toml::from_str("").unwrap();
        let explicit: crate::config::Config = toml::from_str("theme='default'").unwrap();
        assert_eq!(omitted.theme, explicit.theme);

        let actual = Styles::resolve(explicit.theme, &explicit.styles, None);
        let normal = Style::default()
            .fg(Color::Reset)
            .remove_modifier(Modifier::all());
        let muted = normal.add_modifier(Modifier::DIM);

        assert_eq!(
            actual,
            Styles {
                normal,
                muted,
                completed: muted,
                border: muted,
                accent: normal.fg(Color::LightYellow)
            }
        );

        for text in ["theme='Default'", "theme='unknown'", "theme=1", "theme=[]"] {
            assert!(toml::from_str::<crate::config::Config>(text).is_err());
        }
    }
}
