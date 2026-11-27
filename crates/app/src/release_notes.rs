//! What's new in this release, shown once after updating to it (and after
//! the welcome on a first start). Only this release's: each release replaces
//! these with its own.

/// The version these notes are for: this build's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// (feature, a line or two about it).
pub const NOTES: &[(&str, &str)] = &[
    (
        "Your year in review",
        "From 20 December, a shiny card by the playback buttons opens your year in music: top songs, artists, genres and more.",
    ),
    (
        "Listening statistics",
        "View \u{203a} Statistics: plays, time listened, top songs, artists, genres and albums, by hour and by month.",
    ),
    (
        "Lyrics that follow along",
        "The lyrics scroll with the song, synced or not, with the sung line lit up -- and a scroll look, like ink on parchment.",
    ),
    (
        "A better lyrics editor",
        "Ctrl+Enter stamps a line, Ctrl+P plays and pauses, Alt+arrows skip, all timestamps shift by a tenth of a second, and the line being sung shows as you sync.",
    ),
    (
        "Your record, your way",
        "Customize Vinyl: any colour (with a colour picker), see-through vinyl and labels, or a picture of your own wrapped round it.",
    ),
    (
        "Icons and themes",
        "Eight app icons with holiday and pride versions, themes as CSS files to import and export, and a high-contrast theme.",
    ),
    (
        "Smoother under load",
        "With a demanding game running, Openbar5000 draws at a lower frame rate for a while instead of struggling -- and copes better when the system runs short.",
    ),
    (
        "Little things",
        "Share Album copies an album's track list, the speaker icon mutes, tabs scroll when squeezed, and the artist info fits narrow panels.",
    ),
];
