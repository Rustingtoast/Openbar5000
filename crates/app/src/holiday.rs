//! Holiday logos (issue #23): on certain days the app's logo -- window
//! icon, the now-playing bar, About -- dresses up for the occasion. A
//! "special holiday mode" puts a different pride flag behind the logo each
//! month instead. Every variant is the normal music-box logo drawn as SVG
//! with a different backdrop and a few decorations, so there are no image
//! files to keep in sync.

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
}

/// A flag as horizontal stripes, top to bottom; some have extras drawn on
/// top (the Progress Pride chevron, the intersex ring).
struct Flag {
    name: &'static str,
    stripes: &'static [&'static str],
}

const RAINBOW: Flag = Flag {
    name: "Rainbow",
    stripes: &["#e40303", "#ff8c00", "#ffed00", "#008026", "#004dff", "#750787"],
};

/// Special holiday mode's flag for each month, January first.
const MONTHLY_FLAGS: [Flag; 12] = [
    RAINBOW,
    Flag { name: "Transgender", stripes: &["#5bcefa", "#f5a9b8", "#ffffff", "#f5a9b8", "#5bcefa"] },
    Flag { name: "Bisexual", stripes: &["#d60270", "#d60270", "#9b4f96", "#0038a8", "#0038a8"] },
    Flag { name: "Pansexual", stripes: &["#ff218c", "#ffd800", "#21b1ff"] },
    Flag { name: "Lesbian", stripes: &["#d52d00", "#ff9a56", "#ffffff", "#d362a4", "#a30262"] },
    Flag { name: "Progress Pride", stripes: RAINBOW.stripes },
    Flag { name: "Non-binary", stripes: &["#fcf434", "#ffffff", "#9c59d1", "#2c2c2c"] },
    Flag { name: "Asexual", stripes: &["#000000", "#a3a3a3", "#ffffff", "#800080"] },
    Flag { name: "Genderfluid", stripes: &["#ff76a4", "#ffffff", "#c011d7", "#000000", "#2f3cbe"] },
    Flag { name: "Intersex", stripes: &["#ffd800"] },
    Flag { name: "Aromantic", stripes: &["#3da542", "#a7d379", "#ffffff", "#a9a9a9", "#000000"] },
    Flag { name: "Agender", stripes: &["#000000", "#b9b9b9", "#ffffff", "#b8f483", "#ffffff", "#b9b9b9", "#000000"] },
];

/// Which logo to show and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoChoice {
    pub occasion: Option<Occasion>,
    /// Special holiday mode: index into the monthly flags.
    pub flag_month: Option<u32>,
}

impl LogoChoice {
    /// What the settings call for on `month`/`day`. Pride Month always
    /// shows, even with holiday logos switched off (as issue #23 asks).
    pub fn for_date(month: u32, day: u32, holiday_logos: bool, special_mode: bool) -> Self {
        let occasion = Occasion::on(month, day)
            .filter(|occasion| holiday_logos || *occasion == Occasion::PrideMonth);
        Self {
            occasion,
            flag_month: special_mode.then_some(month),
        }
    }

    pub fn is_plain(&self) -> bool {
        self.occasion.is_none() && self.flag_month.is_none()
    }

    /// "Christmas", "Bisexual flag", "Christmas, Bisexual flag", or "".
    pub fn describe(&self) -> String {
        let flag = self.flag_month.map(|m| format!("{} flag", monthly_flag(m).name));
        [self.occasion.map(|o| o.label().to_string()), flag]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The logo as SVG.
    pub fn svg(&self) -> String {
        let backdrop = match (self.flag_month, self.occasion) {
            (Some(month), _) => flag_backdrop(monthly_flag(month), month),
            (None, Some(Occasion::PrideMonth)) => flag_backdrop(&RAINBOW, 0),
            (None, Some(occasion)) => gradient_backdrop(occasion),
            (None, None) => gradient_backdrop_plain(),
        };
        // Decorations stay inside the rounded tile.
        let clipped = |svg: String| {
            if svg.is_empty() { svg } else { format!(r##"<g clip-path="url(#tile)">{svg}</g>"##) }
        };
        let behind = clipped(self.occasion.map(decorations_behind).unwrap_or_default());
        let front = clipped(self.occasion.map(decorations_front).unwrap_or_default());
        // On a flag the cream note and box need an outline to stand out.
        let outlined = self.flag_month.is_some() || self.occasion == Some(Occasion::PrideMonth);
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">{DEFS}{backdrop}{behind}{}{front}</svg>"##,
            logo_body(outlined)
        )
    }
}

fn monthly_flag(month: u32) -> &'static Flag {
    &MONTHLY_FLAGS[(month.clamp(1, 12) - 1) as usize]
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

const DEFS: &str = r##"<defs>
<linearGradient id="boxGrad" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#d98b3f"/><stop offset="1" stop-color="#9c5a20"/></linearGradient>
<linearGradient id="lidGrad" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#e8a158"/><stop offset="1" stop-color="#c97a26"/></linearGradient>
<clipPath id="tile"><rect x="0" y="0" width="256" height="256" rx="56"/></clipPath>
</defs>"##;

fn gradient(top: &str, bottom: &str) -> String {
    format!(
        r##"<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient><rect x="0" y="0" width="256" height="256" rx="56" fill="url(#bg)"/>"##
    )
}

fn gradient_backdrop_plain() -> String {
    gradient("#3a2416", "#1c1108")
}

fn gradient_backdrop(occasion: Occasion) -> String {
    match occasion {
        Occasion::WorkersDay => gradient("#8e1b1b", "#3d0a0a"),
        Occasion::Christmas => gradient("#1f5c32", "#0d2b17"),
        Occasion::NewYear => gradient("#1a2550", "#05081a"),
        Occasion::ChineseNewYear => gradient("#b3121b", "#5c070c"),
        Occasion::PrideMonth => gradient_backdrop_plain(),
    }
}

/// `flag` filling the rounded tile; `month` picks the extras (the Progress
/// Pride chevron in June, the intersex ring in October).
fn flag_backdrop(flag: &Flag, month: u32) -> String {
    let height = 256.0 / flag.stripes.len() as f32;
    let mut svg = String::from(r##"<g clip-path="url(#tile)">"##);
    for (i, color) in flag.stripes.iter().enumerate() {
        svg += &format!(
            r##"<rect x="0" y="{:.2}" width="256" height="{:.2}" fill="{color}"/>"##,
            i as f32 * height,
            height + 0.5
        );
    }
    match month {
        // Progress Pride: the chevron on the left.
        6 => {
            for (i, color) in ["#000000", "#784f17", "#5bcefa", "#f5a9b8", "#ffffff"].iter().enumerate() {
                let tip = 128.0 - i as f32 * 22.0;
                svg += &format!(
                    r##"<path d="M{x0} 0 L{tip} 128 L{x0} 256 L-40 256 L-40 0 Z" fill="{color}"/>"##,
                    x0 = tip - 128.0
                );
            }
        }
        // Intersex: a purple ring on yellow.
        10 => svg += r##"<circle cx="128" cy="128" r="70" fill="none" stroke="#7902aa" stroke-width="22"/>"##,
        _ => {}
    }
    svg + "</g>"
}

/// The music box and its escaping note -- the logo itself.
fn logo_body(outlined: bool) -> String {
    let stroke = if outlined { r##" stroke="#2a1a0e" stroke-width="5""## } else { "" };
    let note_outline = if outlined {
        r##"<g transform="translate(150,46) rotate(-12)" stroke="#2a1a0e" stroke-width="5" fill="#2a1a0e">
<rect x="9" y="-6" width="7" height="60" rx="3.5"/>
<path d="M16 -6 C 44 -2, 46 20, 22 30" fill="none" stroke-width="12" stroke-linecap="round"/>
<ellipse cx="4" cy="52" rx="15" ry="11" transform="rotate(-18 4 52)"/></g>"##
    } else {
        ""
    };
    format!(
        r##"{note_outline}<g transform="translate(150,46) rotate(-12)">
<rect x="9" y="-6" width="7" height="60" rx="3.5" fill="#fff4de"/>
<path d="M16 -6 C 44 -2, 46 20, 22 30" fill="none" stroke="#fff4de" stroke-width="7" stroke-linecap="round"/>
<ellipse cx="4" cy="52" rx="15" ry="11" fill="#fff4de" transform="rotate(-18 4 52)"/></g>
<rect x="46" y="116" width="164" height="32" rx="11" fill="url(#lidGrad)"{stroke}/>
<rect x="46" y="134" width="164" height="14" fill="#7a4518" opacity="0.35"/>
<rect x="46" y="146" width="164" height="78" rx="12" fill="url(#boxGrad)"{stroke}/>
<circle cx="128" cy="185" r="12" fill="#fff4de" stroke="#7a4518" stroke-width="4"/>
<circle cx="128" cy="185" r="3.5" fill="#7a4518"/>"##
    )
}

/// Drawn before the logo (it overlaps them).
fn decorations_behind(occasion: Occasion) -> String {
    match occasion {
        // A red flag rising from behind the box.
        Occasion::WorkersDay => r##"<rect x="66" y="30" width="7" height="100" rx="3" fill="#e8d3b0"/>
<path d="M73 34 C 95 26, 108 44, 132 36 L 132 78 C 108 86, 95 68, 73 76 Z" fill="#e23030" stroke="#fff4de" stroke-width="3"/>"##
            .to_string(),
        // Falling snow.
        Occasion::Christmas => [(30, 40), (70, 22), (104, 64), (42, 92), (220, 30), (232, 96), (200, 112), (20, 140), (236, 150)]
            .iter()
            .map(|(x, y)| format!(r##"<circle cx="{x}" cy="{y}" r="5" fill="#ffffff" opacity="0.9"/>"##))
            .collect(),
        // Fireworks and confetti.
        Occasion::NewYear => {
            let mut svg = String::new();
            for (cx, cy, r, color) in [(70.0, 64.0, 38.0, "#f2c14e"), (118.0, 30.0, 20.0, "#ff6fb5"), (222.0, 112.0, 18.0, "#6fd6ff")] {
                for k in 0..12 {
                    let angle = k as f32 * std::f32::consts::TAU / 12.0;
                    let (x1, y1) = (cx + angle.cos() * r * 0.35, cy + angle.sin() * r * 0.35);
                    let (x2, y2) = (cx + angle.cos() * r, cy + angle.sin() * r);
                    svg += &format!(
                        r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{color}" stroke-width="4" stroke-linecap="round"/>"##
                    );
                }
            }
            svg
        }
        // A paper lantern hanging on the left.
        Occasion::ChineseNewYear => r##"<line x1="72" y1="0" x2="72" y2="30" stroke="#f2c14e" stroke-width="3"/>
<rect x="56" y="28" width="32" height="8" rx="3" fill="#f2c14e"/>
<ellipse cx="72" cy="66" rx="30" ry="32" fill="#e8352b" stroke="#f2c14e" stroke-width="4"/>
<path d="M72 34 C 52 50, 52 82, 72 98 M72 34 C 92 50, 92 82, 72 98" fill="none" stroke="#f2c14e" stroke-width="3"/>
<rect x="56" y="96" width="32" height="8" rx="3" fill="#f2c14e"/>
<line x1="72" y1="104" x2="72" y2="114" stroke="#f2c14e" stroke-width="3"/>"##
            .to_string(),
        Occasion::PrideMonth => String::new(),
    }
}

/// Drawn on top of the logo.
fn decorations_front(occasion: Occasion) -> String {
    match occasion {
        // A Santa hat on the note, its tip flopping to the side.
        Occasion::Christmas => r##"<g transform="translate(150,46) rotate(-12)">
<path d="M-2 4 C 4 -16, 18 -26, 40 -18 C 30 -12, 30 -2, 36 4 Z" fill="#d42a2a"/>
<rect x="-6" y="0" width="46" height="11" rx="5.5" fill="#ffffff"/>
<circle cx="42" cy="-17" r="6.5" fill="#ffffff"/></g>"##
            .to_string(),
        // Gold trim on the box.
        Occasion::ChineseNewYear => r##"<rect x="46" y="146" width="164" height="78" rx="12" fill="none" stroke="#f2c14e" stroke-width="4"/>"##.to_string(),
        _ => String::new(),
    }
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
        assert!(LogoChoice::for_date(10, 15, true, false).is_plain());
    }

    #[test]
    fn special_mode_changes_the_flag_monthly() {
        let march = LogoChoice::for_date(3, 10, true, true);
        assert_eq!(march.describe(), "Bisexual flag");
        let christmas = LogoChoice::for_date(12, 25, true, true);
        assert_eq!(christmas.describe(), "Christmas, Agender flag");
        let names: std::collections::HashSet<_> = MONTHLY_FLAGS.iter().map(|f| f.name).collect();
        assert_eq!(names.len(), 12, "every month has its own flag");
    }

    #[test]
    fn every_logo_variant_is_an_svg_slint_can_load() {
        for month in 1..=12 {
            for (holiday, special) in [(true, false), (true, true)] {
                for day in [1, 6, 15, 25, 31] {
                    let svg = LogoChoice::for_date(month, day, holiday, special).svg();
                    assert!(
                        slint::Image::load_from_svg_data(svg.as_bytes()).is_ok(),
                        "{month}-{day} special={special}: invalid SVG"
                    );
                }
            }
        }
    }
}
