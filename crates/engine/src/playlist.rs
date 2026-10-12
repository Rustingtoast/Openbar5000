use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub duration: Duration,
    /// Raw embedded cover art bytes (still JPEG/PNG-encoded), if the file
    /// has any. Decoding to pixels is a UI-layer concern.
    pub cover_art: Option<Vec<u8>>,
    /// Short codec label for display, e.g. "FLAC", "MP3".
    pub format_label: String,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub path: PathBuf,
    pub metadata: TrackMetadata,
}

#[derive(Debug, Clone, Default)]
pub struct Playlist {
    tracks: Vec<Track>,
    current_index: Option<usize>,
}

impl Playlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_tracks(tracks: Vec<Track>) -> Self {
        Self {
            tracks,
            current_index: None,
        }
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn current(&self) -> Option<&Track> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }

    pub fn set_current_index(&mut self, index: usize) -> Option<&Track> {
        if index < self.tracks.len() {
            self.current_index = Some(index);
            self.tracks.get(index)
        } else {
            None
        }
    }

    /// Advances to the next track, returning it, or `None` at end of playlist.
    pub fn advance(&mut self) -> Option<&Track> {
        let next = match self.current_index {
            Some(i) => i + 1,
            None => 0,
        };
        self.set_current_index(next)
    }

    /// Moves to the previous track, returning it, or `None` if already at the start.
    pub fn retreat(&mut self) -> Option<&Track> {
        match self.current_index {
            Some(i) if i > 0 => self.set_current_index(i - 1),
            _ => None,
        }
    }

    /// Removes the track at `index`, if any, adjusting `current_index` so it
    /// still points at the same track (or `None` if that track was the one
    /// removed -- the caller is responsible for stopping playback in that
    /// case).
    pub fn remove(&mut self, index: usize) {
        if index >= self.tracks.len() {
            return;
        }
        self.tracks.remove(index);
        self.current_index = match self.current_index {
            Some(current) if current == index => None,
            Some(current) if current > index => Some(current - 1),
            other => other,
        };
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
    }
}
