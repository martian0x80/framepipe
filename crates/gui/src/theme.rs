use iced::theme::{Theme, palette};

#[derive(clap::ValueEnum, Clone)]
pub enum ThemeKind {
    Graphite,
    Green,
    Warm,
    Cyber,
    Obs,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 5] = [
        ThemeKind::Graphite,
        ThemeKind::Green,
        ThemeKind::Warm,
        ThemeKind::Cyber,
        ThemeKind::Obs,
    ];
}

impl From<ThemeKind> for Theme {
    fn from(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Graphite => graphite_theme(),
            ThemeKind::Green => green_theme(),
            ThemeKind::Warm => warm_theme(),
            ThemeKind::Cyber => cyber_theme(),
            ThemeKind::Obs => obs_theme(),
        }
    }
}

pub fn get_all_themes() -> Vec<Theme> {
    let existing_themes = Theme::ALL
        .iter()
        .cloned()
        .map(Theme::from)
        .collect::<Vec<_>>();
    let custom_themes = ThemeKind::ALL
        .iter()
        .cloned()
        .map(Theme::from)
        .collect::<Vec<_>>();
    existing_themes.into_iter().chain(custom_themes).collect()
}

pub fn green_theme() -> Theme {
    let base = Theme::Nord;
    let mut palette = base.palette();

    palette.primary = iced::Color::from_rgb8(0x3d, 0xdc, 0x97);
    palette.background = iced::Color::from_rgb8(0x0e, 0x12, 0x10);
    palette.text = iced::Color::from_rgb8(0xe8, 0xf0, 0xea);

    Theme::custom_with_fn("Green", palette, |p| {
        let mut ext = palette::Extended::generate(p);

        ext.primary.strong.color = iced::Color::from_rgb8(0x5c, 0xf2, 0xae);
        ext.primary.weak.color = iced::Color::from_rgb8(0x2d, 0xbf, 0x82);

        ext.background.strong.color = iced::Color::from_rgb8(0x15, 0x1a, 0x17);
        ext.background.weak.color = iced::Color::from_rgb8(0x12, 0x16, 0x14);

        ext
    })
}

pub fn graphite_theme() -> Theme {
    let base = Theme::TokyoNight;
    let mut palette = base.palette();

    palette.primary = iced::Color::from_rgb8(0x4f, 0x8c, 0xff);
    palette.background = iced::Color::from_rgb8(0x0f, 0x11, 0x15);
    palette.text = iced::Color::from_rgb8(0xe6, 0xe6, 0xe6);

    Theme::custom_with_fn("Graphite", palette, |p| {
        let mut ext = palette::Extended::generate(p);

        ext.primary.strong.color = iced::Color::from_rgb8(0x6a, 0xa3, 0xff);
        ext.primary.weak.color = iced::Color::from_rgb8(0x3a, 0x6e, 0xdc);

        ext.background.strong.color = iced::Color::from_rgb8(0x18, 0x1a, 0x20);
        ext.background.weak.color = iced::Color::from_rgb8(0x14, 0x16, 0x1b);

        ext
    })
}

pub fn warm_theme() -> Theme {
    let base = Theme::SolarizedDark;
    let mut palette = base.palette();

    palette.primary = iced::Color::from_rgb8(0xff, 0x8a, 0x3d);
    palette.background = iced::Color::from_rgb8(0x14, 0x12, 0x10);
    palette.text = iced::Color::from_rgb8(0xf2, 0xed, 0xe8);

    Theme::custom_with_fn("Warm", palette, |p| {
        let mut ext = palette::Extended::generate(p);

        ext.primary.strong.color = iced::Color::from_rgb8(0xff, 0xa3, 0x66);
        ext.primary.weak.color = iced::Color::from_rgb8(0xd9, 0x6c, 0x2b);

        ext.background.strong.color = iced::Color::from_rgb8(0x1e, 0x1a, 0x17);
        ext.background.weak.color = iced::Color::from_rgb8(0x18, 0x15, 0x13);

        ext
    })
}

pub fn cyber_theme() -> Theme {
    let base = Theme::Oxocarbon;
    let mut palette = base.palette();

    palette.primary = iced::Color::from_rgb8(0x00, 0xe0, 0xff);
    palette.background = iced::Color::from_rgb8(0x0a, 0x0a, 0x0a);
    palette.text = iced::Color::from_rgb8(0xff, 0xff, 0xff);

    Theme::custom_with_fn("Cyber", palette, |p| {
        let mut ext = palette::Extended::generate(p);

        ext.primary.strong.color = iced::Color::from_rgb8(0x33, 0xe8, 0xff);
        ext.primary.weak.color = iced::Color::from_rgb8(0x00, 0xb8, 0xcc);

        ext.background.strong.color = iced::Color::from_rgb8(0x14, 0x14, 0x14);
        ext.background.weak.color = iced::Color::from_rgb8(0x10, 0x10, 0x10);

        ext
    })
}

pub fn obs_theme() -> Theme {
    let base = Theme::Dark;
    let mut palette = base.palette();

    palette.primary = iced::Color::from_rgb8(0xff, 0x3b, 0x3b);
    palette.background = iced::Color::from_rgb8(0x18, 0x18, 0x18);
    palette.text = iced::Color::from_rgb8(0xff, 0xff, 0xff);

    Theme::custom_with_fn("OBS", palette, |p| {
        let mut ext = palette::Extended::generate(p);

        ext.primary.strong.color = iced::Color::from_rgb8(0xff, 0x5c, 0x5c);
        ext.primary.weak.color = iced::Color::from_rgb8(0xc9, 0x2a, 0x2a);

        ext.background.strong.color = iced::Color::from_rgb8(0x20, 0x20, 0x20);
        ext.background.weak.color = iced::Color::from_rgb8(0x1c, 0x1c, 0x1c);

        ext
    })
}
