//! Holiday logos (issue #23): on certain days the app's logo -- window
//! icon, the now-playing bar, About -- dresses up for the occasion. Pride
//! flags mode shows a different pride flag each month; on a holiday, the
//! holiday's version takes its place. Every icon to choose from has a
//! version for each (`logo_files`, drawn by `compile/logos/generate.py`).

use crate::logo_files::{LogoIcon, ICONS, VERSIONS};
use chrono::Datelike;

/// The days the logo changes for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occasion {
    /// 1 May, International Workers' Day.
    WorkersDay,
    /// June.
    PrideMonth,
    /// 24-26 December and 6 January.
    Christmas,
    /// 31 December and 1 January.
    NewYear,
    /// 15 January to 15 February -- a fixed window around the actual
    /// (moving) date, as the issue asks.
    ChineseNewYear,
}

impl Occasion {
    pub fn on(month: u32, day: u32) -> Option<Self> {
        Some(match (month, day) {
            (5, 1) => Self::WorkersDay,
            (6, _) => Self::PrideMonth,
            (12, 24..=26) | (1, 6) => Self::Christmas,
            (12, 31) | (1, 1) => Self::NewYear,
            (1, 15..=31) | (2, 1..=15) => Self::ChineseNewYear,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WorkersDay => "International Workers' Day",
            Self::PrideMonth => "Pride Month",
            Self::Christmas => "Christmas",
            Self::NewYear => "New Year",
            Self::ChineseNewYear => "Chinese New Year",
        }
    }

    /// Its logo version (see `logo_files::VERSIONS`).
    fn version(self) -> &'static str {
        match self {
            Self::WorkersDay => "workers-day",
            Self::PrideMonth => "pride-month",
            Self::Christmas => "christmas",
            Self::NewYear => "new-year",
            Self::ChineseNewYear => "chinese-new-year",
        }
    }
}

/// Pride flags mode's flag for each month, January first (their colours:
/// `compile/logos/generate.py`).
const MONTHLY_FLAGS: [&str; 12] = [
    "Rainbow",
    "Transgender",
    "Bisexual",
    "Pansexual",
    "Lesbian",
    "Progress Pride",
    "Non-binary",
    "Asexual",
    "Genderfluid",
    "Intersex",
    "Aromantic",
    "Agender",
];

/// Which logo to show and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoChoice {
    pub occasion: Option<Occasion>,
    /// Pride flags mode: the month whose flag is shown.
    pub flag_month: Option<u32>,
}

impl LogoChoice {
    /// What the settings call for on `month`/`day`. Pride Month always
    /// shows, even with holiday logos switched off (as issue #23 asks). A
    /// holiday's logo wins over the month's pride flag: Christmas shows
    /// Christmas.
    pub fn for_date(month: u32, day: u32, holiday_logos: bool, pride_flags: bool) -> Self {
        let occasion = Occasion::on(month, day)
            .filter(|occasion| holiday_logos || *occasion == Occasion::PrideMonth);
        let holiday = occasion.is_some_and(|occasion| occasion != Occasion::PrideMonth);
        Self {
            occasion,
            flag_month: (pride_flags && !holiday).then_some(month),
        }
    }

    /// "Christmas", "Bisexual flag", "Pride Month, Progress Pride flag",
    /// or "".
    pub fn describe(&self) -> String {
        let flag = self.flag_month.map(|m| format!("{} flag", monthly_flag(m)));
        [self.occasion.map(|o| o.label().to_string()), flag]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Which version of the icon this is (see `logo_files::VERSIONS`):
    /// a holiday's own, else the month's flag, else Pride Month's.
    pub fn version(&self) -> String {
        match (self.occasion, self.flag_month) {
            (Some(occasion), _) if occasion != Occasion::PrideMonth => occasion.version().to_string(),
            (_, Some(month)) => format!("flag-{:02}", month.clamp(1, 12)),
            (Some(occasion), None) => occasion.version().to_string(),
            (None, None) => "regular".to_string(),
        }
    }
}

fn monthly_flag(month: u32) -> &'static str {
    MONTHLY_FLAGS[(month.clamp(1, 12) - 1) as usize]
}

/// The icon saved as `id`, or the default (the first) if there's none
/// such any more.
pub fn icon(id: &str) -> &'static LogoIcon {
    ICONS.iter().find(|icon| icon.id == id).unwrap_or(&ICONS[0])
}

/// Whether `version` is one every icon has.
pub fn is_version(version: &str) -> bool {
    VERSIONS.contains(&version)
}

/// `icon`'s `version` as SVG; its regular one for a version it lacks.
pub fn logo_svg(icon: &LogoIcon, version: &str) -> &'static str {
    let index = VERSIONS.iter().position(|v| *v == version).unwrap_or(0);
    icon.versions[index]
}

/// A version's name, as Preferences and the preview show it: "Regular",
/// "Christmas", "Bisexual flag".
pub fn version_label(version: &str) -> String {
    if let Some(month) = version.strip_prefix("flag-").and_then(|m| m.parse::<u32>().ok()) {
        return format!("{} flag", monthly_flag(month));
    }
    match version {
        "workers-day" => Occasion::WorkersDay.label(),
        "pride-month" => Occasion::PrideMonth.label(),
        "christmas" => Occasion::Christmas.label(),
        "new-year" => Occasion::NewYear.label(),
        "chinese-new-year" => Occasion::ChineseNewYear.label(),
        _ => "Regular",
    }
    .to_string()
}

/// Today's local date, or `OPENBAR5000_DATE` (YYYY-MM-DD) to preview
/// another day's logo.
pub fn today() -> (u32, u32) {
    if let Some(date) = std::env::var("OPENBAR5000_DATE")
        .ok()
        .and_then(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok())
    {
        return (date.month(), date.day());
    }
    let now = chrono::Local::now();
    (now.month(), now.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holidays_fall_on_the_issues_dates() {
        assert_eq!(Occasion::on(5, 1), Some(Occasion::WorkersDay));
        assert_eq!(Occasion::on(6, 15), Some(Occasion::PrideMonth));
        assert_eq!(Occasion::on(12, 25), Some(Occasion::Christmas));
        assert_eq!(Occasion::on(1, 6), Some(Occasion::Christmas));
        assert_eq!(Occasion::on(12, 31), Some(Occasion::NewYear));
        assert_eq!(Occasion::on(1, 1), Some(Occasion::NewYear));
        assert_eq!(Occasion::on(1, 15), Some(Occasion::ChineseNewYear));
        assert_eq!(Occasion::on(2, 15), Some(Occasion::ChineseNewYear));
        assert_eq!(Occasion::on(2, 16), None);
        assert_eq!(Occasion::on(12, 27), None);
        assert_eq!(Occasion::on(10, 15), None);
    }

    #[test]
    fn pride_month_shows_even_with_holiday_logos_off() {
        assert_eq!(LogoChoice::for_date(6, 10, false, false).occasion, Some(Occasion::PrideMonth));
        assert_eq!(LogoChoice::for_date(12, 25, false, false).occasion, None);
        assert_eq!(LogoChoice::for_date(10, 15, true, false).version(), "regular");
    }

    #[test]
    fn pride_flags_change_monthly_and_give_way_to_holidays() {
        let march = LogoChoice::for_date(3, 10, true, true);
        assert_eq!(march.describe(), "Bisexual flag");
        assert_eq!(march.version(), "flag-03");
        assert_eq!(LogoChoice::for_date(12, 25, true, true).version(), "christmas");
        assert_eq!(LogoChoice::for_date(12, 20, true, true).describe(), "Agender flag");
        // With holiday logos off there's no holiday to give way to.
        assert_eq!(LogoChoice::for_date(12, 25, false, true).version(), "flag-12");
        let june = LogoChoice::for_date(6, 10, true, true);
        assert_eq!(june.describe(), "Pride Month, Progress Pride flag");
        assert_eq!(june.version(), "flag-06");
        assert_eq!(LogoChoice::for_date(6, 10, true, false).version(), "pride-month");
    }

    #[test]
    fn versions_have_names_and_unknown_ones_fall_back() {
        assert_eq!(version_label("flag-02"), "Transgender flag");
        assert_eq!(version_label("christmas"), "Christmas");
        assert_eq!(version_label("regular"), "Regular");
        assert!(is_version("flag-12") && !is_version("flag-13"));
        assert_eq!(icon("no-such-icon").id, ICONS[0].id);
        assert_eq!(logo_svg(icon("music-box"), "nope"), icon("music-box").versions[0]);
    }

    #[test]
    fn every_icon_version_is_an_svg_slint_can_draw() {
        for icon in ICONS {
            for (version, svg) in VERSIONS.iter().zip(icon.versions) {
                let image = slint::Image::load_from_svg_data(svg.as_bytes());
                assert!(image.is_ok(), "{} {version}: invalid SVG", icon.id);
            }
        }
    }

    #[test]
    fn every_logo_draws_into_a_bitmap() {
        for icon in ICONS {
            let svg = logo_svg(icon, "regular");
            let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &resvg::usvg::Options::default());
            assert!(tree.is_ok(), "{}: not drawable", icon.id);
        }
        let image = crate::logo_preview::raster(ICONS[0].versions[0], 64);
        assert_eq!((image.size().width, image.size().height), (64, 64));
    }

}
