use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeName {
    AmberPlotter,
    GreenRadar,
    CyanAnalyzer,
    RedSchematic,
    MonoField,
}

impl ThemeName {
    pub const ALL: [Self; 5] = [
        Self::AmberPlotter,
        Self::GreenRadar,
        Self::CyanAnalyzer,
        Self::RedSchematic,
        Self::MonoField,
    ];

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|theme| *theme == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AmberPlotter => "AMBER // PLOTTER",
            Self::GreenRadar => "GREEN // RADAR",
            Self::CyanAnalyzer => "CYAN // ANALYZER",
            Self::RedSchematic => "RED // SCHEMATIC",
            Self::MonoField => "MONO // FIELD",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::AmberPlotter => Palette::from_hexes(
                0x080704, 0x100d08, 0xd69b3c, 0xffd37a, 0x765b2f, 0xf1e3c2, 0x8c8068, 0x2a2114,
                0xf06445, 0xb78e64,
            ),
            Self::GreenRadar => Palette::from_hexes(
                0x040805, 0x071009, 0x69c76d, 0xb7ffaf, 0x356a3d, 0xdaebd8, 0x738475, 0x142b18,
                0xe2a64a, 0x8ead79,
            ),
            Self::CyanAnalyzer => Palette::from_hexes(
                0x030809, 0x061113, 0x48b9c7, 0xa8f5ff, 0x2d6870, 0xdcf1f2, 0x73898b, 0x123036,
                0xe0a15b, 0x7aa6a4,
            ),
            Self::RedSchematic => Palette::from_hexes(
                0x090505, 0x130909, 0xc95c4a, 0xff9a7d, 0x703a32, 0xf0ded8, 0x8c7670, 0x311611,
                0xf2b34e, 0xa9856d,
            ),
            Self::MonoField => Palette::from_hexes(
                0x070707, 0x0f0f0f, 0xb8b8ad, 0xffffff, 0x666862, 0xe6e6de, 0x85857e, 0x242521,
                0xffffff, 0x9a9a92,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub panel: Color,
    pub primary: Color,
    pub hot: Color,
    pub secondary: Color,
    pub text: Color,
    pub muted: Color,
    pub grid: Color,
    pub warning: Color,
    pub inferred: Color,
}

impl Palette {
    #[allow(clippy::too_many_arguments)]
    const fn from_hexes(
        background: u32,
        panel: u32,
        primary: u32,
        hot: u32,
        secondary: u32,
        text: u32,
        muted: u32,
        grid: u32,
        warning: u32,
        inferred: u32,
    ) -> Self {
        Self {
            background: rgb(background),
            panel: rgb(panel),
            primary: rgb(primary),
            hot: rgb(hot),
            secondary: rgb(secondary),
            text: rgb(text),
            muted: rgb(muted),
            grid: rgb(grid),
            warning: rgb(warning),
            inferred: rgb(inferred),
        }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn themes_keep_semantic_roles_distinct() {
        let primaries: HashSet<_> = ThemeName::ALL
            .iter()
            .map(|theme| theme.palette().primary)
            .collect();
        assert_eq!(primaries.len(), ThemeName::ALL.len());

        for theme in ThemeName::ALL {
            let palette = theme.palette();
            assert_ne!(palette.background, palette.primary);
            assert_ne!(palette.primary, palette.hot);
            assert_ne!(palette.grid, palette.text);
            assert_ne!(palette.inferred, palette.background);
        }
    }

    #[test]
    fn theme_palettes_match_the_reviewed_tokens() {
        insta::assert_debug_snapshot!(
            "theme_palettes",
            ThemeName::ALL.map(|theme| (theme.label(), theme.palette()))
        );
    }
}
