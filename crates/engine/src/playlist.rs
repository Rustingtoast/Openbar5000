use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub duration: Duration,
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
}
