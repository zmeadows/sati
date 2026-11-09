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
    ModusVivendi,
    ModusOperandiDeuteranopia,
    ModusVivendiDeuteranopia,
    ModusOperandiTritanopia,
    ModusVivendiTritanopia,
}

impl FromStr for BuiltinTheme {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .find(|(_, id)| *id == name)
            .map(|(theme, _)| *theme)
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
    pub const ALL: &'static [(Self, &'static str)] = &[
        (Self::Default, "default"),
        (Self::GruvboxMaterial, "gruvbox-material"),
        (Self::CatppuccinMocha, "catppuccin-mocha"),
        (Self::TokyoNightStorm, "tokyonight-storm"),
        (Self::KanagawaWave, "kanagawa-wave"),
        (Self::RosePine, "rose-pine"),
        (Self::Nightfox, "nightfox"),
        (Self::OneDark, "onedark"),
        (Self::GithubDark, "github-dark"),
        (Self::EverforestMedium, "everforest-medium"),
        (Self::CyberdreamMuted, "cyberdream-muted"),
        (Self::Nord, "nord"),
        (Self::Dracula, "dracula"),
        (Self::Darcula, "darcula"),
        (Self::Gruvbox, "gruvbox"),
        (Self::SolarizedDark, "solarized-dark"),
        (Self::SolarizedLight, "solarized-light"),
        (Self::GruvboxLight, "gruvbox-light"),
        (Self::CatppuccinLatte, "catppuccin-latte"),
        (Self::GithubLight, "github-light"),
        (Self::RosePineDawn, "rose-pine-dawn"),
        (Self::EverforestLight, "everforest-light"),
        (Self::ModusOperandi, "modus-operandi"),
        (Self::EfSummer, "ef-summer"),
        (Self::Monokai, "monokai"),
        (Self::ModusVivendi, "modus-vivendi"),
        (Self::ModusOperandiDeuteranopia, "modus-operandi-deuteranopia"),
        (Self::ModusVivendiDeuteranopia, "modus-vivendi-deuteranopia"),
        (Self::ModusOperandiTritanopia, "modus-operandi-tritanopia"),
        (Self::ModusVivendiTritanopia, "modus-vivendi-tritanopia"),
    ];

    pub fn id(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(theme, _)| *theme == self)
            .expect("registered theme")
            .1
    }

    pub fn cycle(self, forward: bool) -> Self {
        let index = Self::ALL
            .iter()
            .position(|(theme, _)| *theme == self)
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
                heading: Style::default().fg(Color::LightCyan),
                carried: Style::default().fg(Color::Yellow),
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
                    heading: plain.fg(Color::Rgb(169, 182, 101)),
                    carried: plain.fg(Color::Rgb(216, 166, 87)),
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
                    heading: plain.fg(Color::Rgb(148, 226, 213)),
                    carried: plain.fg(Color::Rgb(250, 179, 135)),
                }
            }
            Self::TokyoNightStorm => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(192, 202, 245)).bg(Color::Rgb(36, 40, 59)),
                    muted: plain.fg(Color::Rgb(115, 122, 162)),
                    completed: plain.fg(Color::Rgb(115, 122, 162)),
                    border: plain.fg(Color::Rgb(84, 92, 126)),
                    accent: plain.fg(Color::Rgb(122, 162, 247)),
                    heading: plain.fg(Color::Rgb(158, 206, 106)),
                    carried: plain.fg(Color::Rgb(224, 175, 104)),
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
                    heading: plain.fg(Color::Rgb(152, 187, 108)),
                    carried: plain.fg(Color::Rgb(230, 195, 132)),
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
                    heading: plain.fg(Color::Rgb(156, 207, 216)),
                    carried: plain.fg(Color::Rgb(246, 193, 119)),
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
                    heading: plain.fg(Color::Rgb(129, 178, 154)),
                    carried: plain.fg(Color::Rgb(219, 192, 116)),
                }
            }
            Self::OneDark => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(171, 178, 191)).bg(Color::Rgb(40, 44, 52)),
                    muted: plain.fg(Color::Rgb(132, 139, 152)),
                    completed: plain.fg(Color::Rgb(132, 139, 152)),
                    border: plain.fg(Color::Rgb(57, 63, 74)),
                    accent: plain.fg(Color::Rgb(97, 175, 239)),
                    heading: plain.fg(Color::Rgb(152, 195, 121)),
                    carried: plain.fg(Color::Rgb(229, 192, 123)),
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
                    heading: plain.fg(Color::Rgb(63, 185, 80)),
                    carried: plain.fg(Color::Rgb(210, 153, 34)),
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
                    heading: plain.fg(Color::Rgb(127, 187, 179)),
                    carried: plain.fg(Color::Rgb(219, 188, 127)),
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
                    heading: plain.fg(Color::Rgb(126, 224, 136)),
                    carried: plain.fg(Color::Rgb(227, 200, 117)),
                }
            }
            Self::Nord => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(216, 222, 233)).bg(Color::Rgb(46, 52, 64)),
                    muted: plain.fg(Color::Rgb(156, 168, 184)),
                    completed: plain.fg(Color::Rgb(156, 168, 184)),
                    border: plain.fg(Color::Rgb(67, 76, 94)),
                    accent: plain.fg(Color::Rgb(136, 192, 208)),
                    heading: plain.fg(Color::Rgb(163, 190, 140)),
                    carried: plain.fg(Color::Rgb(235, 203, 139)),
                }
            }
            Self::Dracula => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(248, 248, 242)).bg(Color::Rgb(40, 42, 54)),
                    muted: plain.fg(Color::Rgb(98, 114, 164)),
                    completed: plain.fg(Color::Rgb(98, 114, 164)),
                    border: plain.fg(Color::Rgb(68, 71, 90)),
                    accent: plain.fg(Color::Rgb(189, 147, 249)),
                    heading: plain.fg(Color::Rgb(139, 233, 253)),
                    carried: plain.fg(Color::Rgb(255, 184, 108)),
                }
            }
            Self::Darcula => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(169, 183, 198)).bg(Color::Rgb(43, 43, 43)),
                    muted: plain.fg(Color::Rgb(128, 128, 128)),
                    completed: plain.fg(Color::Rgb(128, 128, 128)),
                    border: plain.fg(Color::Rgb(96, 96, 96)),
                    accent: plain.fg(Color::Rgb(255, 198, 109)),
                    heading: plain.fg(Color::Rgb(104, 151, 187)),
                    carried: plain.fg(Color::Rgb(204, 120, 50)),
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
                    heading: plain.fg(Color::Rgb(142, 192, 124)),
                    carried: plain.fg(Color::Rgb(250, 189, 47)),
                }
            }
            Self::SolarizedDark => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(131, 148, 150)).bg(Color::Rgb(0, 43, 54)),
                    muted: plain.fg(Color::Rgb(101, 123, 131)),
                    completed: plain.fg(Color::Rgb(101, 123, 131)),
                    border: plain.fg(Color::Rgb(88, 110, 117)),
                    accent: plain.fg(Color::Rgb(38, 139, 210)),
                    heading: plain.fg(Color::Rgb(42, 161, 152)),
                    carried: plain.fg(Color::Rgb(181, 137, 0)),
                }
            }
            Self::SolarizedLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(101, 123, 131)).bg(Color::Rgb(253, 246, 227)),
                    muted: plain.fg(Color::Rgb(131, 148, 150)),
                    completed: plain.fg(Color::Rgb(131, 148, 150)),
                    border: plain.fg(Color::Rgb(147, 161, 161)),
                    accent: plain.fg(Color::Rgb(38, 139, 210)),
                    heading: plain.fg(Color::Rgb(42, 161, 152)),
                    carried: plain.fg(Color::Rgb(181, 137, 0)),
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
                    heading: plain.fg(Color::Rgb(66, 123, 88)),
                    carried: plain.fg(Color::Rgb(181, 118, 20)),
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
                    heading: plain.fg(Color::Rgb(30, 102, 245)),
                    carried: plain.fg(Color::Rgb(254, 100, 11)),
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
                    heading: plain.fg(Color::Rgb(26, 127, 55)),
                    carried: plain.fg(Color::Rgb(154, 103, 0)),
                }
            }
            Self::RosePineDawn => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(70, 66, 97)).bg(Color::Rgb(250, 244, 237)),
                    muted: plain.fg(Color::Rgb(121, 117, 147)),
                    completed: plain.fg(Color::Rgb(121, 117, 147)),
                    border: plain.fg(Color::Rgb(206, 202, 205)),
                    accent: plain.fg(Color::Rgb(180, 99, 122)),
                    heading: plain.fg(Color::Rgb(40, 105, 131)),
                    carried: plain.fg(Color::Rgb(144, 122, 169)),
                }
            }
            Self::EverforestLight => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(92, 106, 114)).bg(Color::Rgb(253, 246, 227)),
                    muted: plain.fg(Color::Rgb(130, 145, 129)),
                    completed: plain.fg(Color::Rgb(130, 145, 129)),
                    border: plain.fg(Color::Rgb(147, 159, 145)),
                    accent: plain.fg(Color::Rgb(141, 161, 1)),
                    heading: plain.fg(Color::Rgb(53, 167, 124)),
                    carried: plain.fg(Color::Rgb(245, 125, 38)),
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
                    heading: plain.fg(Color::Rgb(114, 16, 69)),
                    carried: plain.fg(Color::Rgb(111, 85, 0)),
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
                    heading: plain.fg(Color::Rgb(15, 123, 143)),
                    carried: plain.fg(Color::Rgb(164, 95, 34)),
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
                    heading: plain.fg(Color::Rgb(102, 217, 239)),
                    carried: plain.fg(Color::Rgb(253, 151, 31)),
                }
            }
            Self::ModusVivendi => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(0, 0, 0)),
                    muted: plain.fg(Color::Rgb(152, 152, 152)),
                    completed: plain.fg(Color::Rgb(152, 152, 152)),
                    border: plain.fg(Color::Rgb(100, 100, 100)),
                    accent: plain.fg(Color::Rgb(47, 175, 255)),
                    heading: plain.fg(Color::Rgb(254, 172, 208)),
                    carried: plain.fg(Color::Rgb(208, 188, 0)),
                }
            }
            Self::ModusOperandiDeuteranopia => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255)),
                    muted: plain.fg(Color::Rgb(89, 89, 89)),
                    completed: plain.fg(Color::Rgb(89, 89, 89)),
                    border: plain.fg(Color::Rgb(89, 89, 89)),
                    accent: plain.fg(Color::Rgb(0, 49, 169)),
                    heading: plain.fg(Color::Rgb(83, 26, 182)),
                    carried: plain.fg(Color::Rgb(105, 85, 0)),
                }
            }
            Self::ModusVivendiDeuteranopia => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(0, 0, 0)),
                    muted: plain.fg(Color::Rgb(152, 152, 152)),
                    completed: plain.fg(Color::Rgb(152, 152, 152)),
                    border: plain.fg(Color::Rgb(100, 100, 100)),
                    accent: plain.fg(Color::Rgb(47, 175, 255)),
                    heading: plain.fg(Color::Rgb(182, 160, 255)),
                    carried: plain.fg(Color::Rgb(202, 191, 0)),
                }
            }
            Self::ModusOperandiTritanopia => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255)),
                    muted: plain.fg(Color::Rgb(89, 89, 89)),
                    completed: plain.fg(Color::Rgb(89, 89, 89)),
                    border: plain.fg(Color::Rgb(89, 89, 89)),
                    accent: plain.fg(Color::Rgb(0, 94, 139)),
                    heading: plain.fg(Color::Rgb(166, 0, 0)),
                    carried: plain.fg(Color::Rgb(160, 19, 47)),
                }
            }
            Self::ModusVivendiTritanopia => {
                let plain = Style::default().remove_modifier(Modifier::all());
                Styles {
                    normal: plain.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(0, 0, 0)),
                    muted: plain.fg(Color::Rgb(152, 152, 152)),
                    completed: plain.fg(Color::Rgb(152, 152, 152)),
                    border: plain.fg(Color::Rgb(100, 100, 100)),
                    accent: plain.fg(Color::Rgb(0, 211, 208)),
                    heading: plain.fg(Color::Rgb(255, 95, 89)),
                    carried: plain.fg(Color::Rgb(255, 127, 134)),
                }
            }
        }
    }
}

// Prints one theme ID per line for `sati --list-themes`.
pub fn print_list() -> crate::storage::Result<()> {
    let mut contents = String::new();
    for (_, id) in BuiltinTheme::ALL {
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
    fn named_themes_resolve_to_their_exact_palettes() {
        // In registry order: background, then normal, muted, completed, border, accent,
        // heading, and carried foregrounds.
        #[rustfmt::skip]
        let palettes = [
            ("gruvbox-material", ["#282828", "#d4be98", "#a89984", "#928374", "#5a524c", "#e78a4e", "#a9b665", "#d8a657"]),
            ("catppuccin-mocha", ["#1e1e2e", "#cdd6f4", "#a6adc8", "#6c7086", "#585b70", "#cba6f7", "#94e2d5", "#fab387"]),
            ("tokyonight-storm", ["#24283b", "#c0caf5", "#737aa2", "#737aa2", "#545c7e", "#7aa2f7", "#9ece6a", "#e0af68"]),
            ("kanagawa-wave", ["#1f1f28", "#dcd7ba", "#c8c093", "#727169", "#54546d", "#7e9cd8", "#98bb6c", "#e6c384"]),
            ("rose-pine", ["#191724", "#e0def4", "#908caa", "#6e6a86", "#524f67", "#ebbcba", "#9ccfd8", "#f6c177"]),
            ("nightfox", ["#192330", "#cdcecf", "#aeafb0", "#738091", "#39506d", "#719cd6", "#81b29a", "#dbc074"]),
            ("onedark", ["#282c34", "#abb2bf", "#848b98", "#848b98", "#393f4a", "#61afef", "#98c379", "#e5c07b"]),
            ("github-dark", ["#0d1117", "#c9d1d9", "#8b949e", "#6e7681", "#30363d", "#58a6ff", "#3fb950", "#d29922"]),
            ("everforest-medium", ["#2d353b", "#d3c6aa", "#9da9a0", "#7a8478", "#4f585e", "#a7c080", "#7fbbb3", "#dbbc7f"]),
            ("cyberdream-muted", ["#16181a", "#ffffff", "#9ba3b2", "#7b8496", "#3c4048", "#78a8e8", "#7ee088", "#e3c875"]),
            ("nord", ["#2e3440", "#d8dee9", "#9ca8b8", "#9ca8b8", "#434c5e", "#88c0d0", "#a3be8c", "#ebcb8b"]),
            ("dracula", ["#282a36", "#f8f8f2", "#6272a4", "#6272a4", "#44475a", "#bd93f9", "#8be9fd", "#ffb86c"]),
            ("darcula", ["#2b2b2b", "#a9b7c6", "#808080", "#808080", "#606060", "#ffc66d", "#6897bb", "#cc7832"]),
            ("gruvbox", ["#282828", "#ebdbb2", "#a89984", "#928374", "#665c54", "#fe8019", "#8ec07c", "#fabd2f"]),
            ("solarized-dark", ["#002b36", "#839496", "#657b83", "#657b83", "#586e75", "#268bd2", "#2aa198", "#b58900"]),
            ("solarized-light", ["#fdf6e3", "#657b83", "#839496", "#839496", "#93a1a1", "#268bd2", "#2aa198", "#b58900"]),
            ("gruvbox-light", ["#fbf1c7", "#3c3836", "#7c6f64", "#928374", "#a89984", "#af3a03", "#427b58", "#b57614"]),
            ("catppuccin-latte", ["#eff1f5", "#4c4f69", "#6c6f85", "#9ca0b0", "#acb0be", "#8839ef", "#1e66f5", "#fe640b"]),
            ("github-light", ["#ffffff", "#24292f", "#57606a", "#6e7781", "#d0d7de", "#0969da", "#1a7f37", "#9a6700"]),
            ("rose-pine-dawn", ["#faf4ed", "#464261", "#797593", "#797593", "#cecacd", "#b4637a", "#286983", "#907aa9"]),
            ("everforest-light", ["#fdf6e3", "#5c6a72", "#829181", "#829181", "#939f91", "#8da101", "#35a77c", "#f57d26"]),
            ("modus-operandi", ["#ffffff", "#000000", "#595959", "#595959", "#9f9f9f", "#0031a9", "#721045", "#6f5500"]),
            ("ef-summer", ["#fff2f3", "#4f4073", "#786e74", "#786e74", "#c6bbc6", "#cb1aaa", "#0f7b8f", "#a45f22"]),
            ("monokai", ["#272822", "#f8f8f2", "#90908a", "#75715e", "#64645e", "#f92672", "#66d9ef", "#fd971f"]),
            ("modus-vivendi", ["#000000", "#ffffff", "#989898", "#989898", "#646464", "#2fafff", "#feacd0", "#d0bc00"]),
            ("modus-operandi-deuteranopia", ["#ffffff", "#000000", "#595959", "#595959", "#595959", "#0031a9", "#531ab6", "#695500"]),
            ("modus-vivendi-deuteranopia", ["#000000", "#ffffff", "#989898", "#989898", "#646464", "#2fafff", "#b6a0ff", "#cabf00"]),
            ("modus-operandi-tritanopia", ["#ffffff", "#000000", "#595959", "#595959", "#595959", "#005e8b", "#a60000", "#a0132f"]),
            ("modus-vivendi-tritanopia", ["#000000", "#ffffff", "#989898", "#989898", "#646464", "#00d3d0", "#ff5f59", "#ff7f86"]),
        ];

        assert_eq!(palettes.len(), BuiltinTheme::ALL.len() - 1);

        for ((name, colors), (theme, id)) in palettes.iter().zip(&BuiltinTheme::ALL[1..]) {
            assert_eq!(name, id);

            let [
                background,
                normal,
                muted,
                completed,
                border,
                accent,
                heading,
                carried,
            ] = colors.map(|hex| hex.parse::<Color>().unwrap());
            let styles = Styles::resolve(*theme, &crate::config::StyleOverrides::default(), None);

            for (role, color) in [
                (styles.normal, normal),
                (styles.muted, muted),
                (styles.completed, completed),
                (styles.border, border),
                (styles.accent, accent),
                (styles.heading, heading),
                (styles.carried, carried),
            ] {
                assert!(matches!(color, Color::Rgb(..)), "{name}");
                assert_eq!(role.fg, Some(color), "{name}");
                assert_eq!(role.bg, Some(background), "{name}");
                assert!(role.add_modifier.is_empty(), "{name}");
                assert_eq!(role.sub_modifier, Modifier::all(), "{name}");
            }
        }
    }

    #[test]
    fn registry_drives_both_cycle_directions_and_wraparound() {
        for (index, &(theme, id)) in BuiltinTheme::ALL.iter().enumerate() {
            assert_eq!(id.parse::<BuiltinTheme>().unwrap(), theme);
            assert_eq!(theme.id(), id);
            assert_eq!(
                theme.cycle(true),
                BuiltinTheme::ALL[(index + 1) % BuiltinTheme::ALL.len()].0
            );
            assert_eq!(theme.cycle(true).cycle(false), theme);
            assert_eq!(theme.cycle(false).cycle(true), theme);
        }

        assert_eq!(
            BuiltinTheme::Default.cycle(false),
            BuiltinTheme::ModusVivendiTritanopia
        );
        assert_eq!(
            BuiltinTheme::Monokai.cycle(true),
            BuiltinTheme::ModusVivendi
        );
    }

    #[test]
    fn all_available_themes_parse_and_preserve_overrides_and_modifier_resets() {
        let names: Vec<_> = BuiltinTheme::ALL.iter().map(|(_, name)| *name).collect();
        assert_eq!(names.len(), 30);
        assert_eq!(
            names.iter().collect::<std::collections::HashSet<_>>().len(),
            30
        );

        for name in names {
            let theme = name.parse::<BuiltinTheme>().unwrap();
            let config: crate::config::Config = toml::from_str(&format!("theme='{name}'")).unwrap();
            assert_eq!(config.theme, theme);
            assert!(name.to_uppercase().parse::<BuiltinTheme>().is_err());

            let defaults = Styles::resolve(theme, &config.styles, None);

            let config: crate::config::Config = toml::from_str(&format!("theme='{name}'\n[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers=['italic']\n[styles.accent]\nfg='red'\nmodifiers=['bold']\n[styles.heading]\nfg='cyan'\n[styles.carried]\nmodifiers=['underlined']")).unwrap();
            let resolved = Styles::resolve(config.theme, &config.styles, Some(Color::Blue));
            assert_eq!(resolved.normal.add_modifier, Modifier::BOLD);
            assert_eq!(resolved.muted.fg, Some(Color::Green));
            assert_eq!(resolved.muted.add_modifier, Modifier::ITALIC);
            assert_eq!(resolved.accent.fg, Some(Color::Blue));
            assert_eq!(resolved.accent.add_modifier, Modifier::BOLD);
            assert_eq!(resolved.heading.fg, Some(Color::Cyan));
            assert_eq!(resolved.carried.fg, defaults.carried.fg);
            assert_eq!(resolved.carried.add_modifier, Modifier::UNDERLINED);

            // The default theme inherits normal's modifiers; named themes clear them.
            if theme == BuiltinTheme::Default {
                assert_eq!(resolved.heading.add_modifier, Modifier::BOLD);
            } else {
                assert!(resolved.completed.add_modifier.is_empty());
                assert!(resolved.border.add_modifier.is_empty());
                assert!(resolved.heading.add_modifier.is_empty());
            }
        }
    }

    #[test]
    fn gruvbox_material_clears_inherited_modifiers_and_keeps_overrides() {
        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'").unwrap();
        assert_eq!(config.theme, BuiltinTheme::GruvboxMaterial);
        assert_eq!(
            "gruvbox-material".parse::<BuiltinTheme>().unwrap(),
            config.theme
        );
        assert!("Gruvbox-Material".parse::<BuiltinTheme>().is_err());

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
            inherited.heading,
            inherited.carried,
        ] {
            assert!(role.add_modifier.is_empty());
        }

        let config: crate::config::Config = toml::from_str("theme='gruvbox-material'\n[styles.normal]\nmodifiers=['bold']\n[styles.muted]\nfg='green'\nmodifiers=['italic']\n[styles.completed]\nmodifiers=['dim']\n[styles.border]\nmodifiers=[]\n[styles.accent]\nfg='blue'\nmodifiers=['bold']\n[styles.heading]\nbg='#102030'\nmodifiers=['italic']\n[styles.carried]\nfg='reset'").unwrap();
        let styles = Styles::resolve(config.theme, &config.styles, Some(Color::Red));
        assert_eq!(styles.normal.add_modifier, Modifier::BOLD);
        assert_eq!(styles.muted.fg, Some(Color::Green));
        assert_eq!(styles.muted.add_modifier, Modifier::ITALIC);
        assert_eq!(styles.completed.add_modifier, Modifier::DIM);
        assert!(styles.border.add_modifier.is_empty());
        assert_eq!(styles.accent.fg, Some(Color::Red));
        assert_eq!(styles.accent.add_modifier, Modifier::BOLD);
        assert_eq!(styles.heading.fg, Some(Color::Rgb(169, 182, 101)));
        assert_eq!(styles.heading.bg, Some(Color::Rgb(16, 32, 48)));
        assert_eq!(styles.heading.add_modifier, Modifier::ITALIC);
        assert_eq!(styles.carried.fg, Some(Color::Reset));
        assert_eq!(styles.carried.bg, Some(Color::Rgb(40, 40, 40)));
        assert!(styles.carried.add_modifier.is_empty());
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

        // Terminal palette colors and no background, so the terminal's own scheme shows.
        assert_eq!(
            actual,
            Styles {
                normal,
                muted,
                completed: muted,
                border: muted,
                accent: normal.fg(Color::LightYellow),
                heading: normal.fg(Color::LightCyan),
                carried: normal.fg(Color::Yellow),
            }
        );
        assert_eq!(normal.bg, None);

        for text in ["theme='Default'", "theme='unknown'", "theme=1", "theme=[]"] {
            assert!(toml::from_str::<crate::config::Config>(text).is_err());
        }
    }
}
