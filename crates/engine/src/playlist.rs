use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    /// The album's artist when tagged (often unset); "" otherwise.
    pub album_artist: String,
    pub album: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    /// Release/recording date exactly as tagged (often just a year, e.g.
    /// "2025", sometimes a full "2019-11-15") -- empty if untagged.
    pub date: String,
    pub genre: String,
    pub composer: String,
    pub bpm: Option<u32>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
    pub duration: Duration,
    /// Embedded cover art, already decoded and shrunk to playlist-thumbnail
    /// size at load time (off the UI thread). Full-size art is deliberately
    /// not kept: a 3000px cover is ~36 MB of pixels, and a large playlist
    /// holds hundreds of them. `Arc` so the playlist clones sent to the UI
    /// stay cheap.
    pub cover_thumbnail: Option<Arc<CoverThumbnail>>,
    /// Short codec label for display, e.g. "FLAC", "MP3".
    pub format_label: String,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
}

/// Straight (non-premultiplied) RGBA8 pixels, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverThumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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

    /// Reorders the playlist so position `k` holds the track previously at
    /// `order[k]`, keeping `current_index` on the same track. Returns
    /// `false` (and changes nothing) unless `order` is a permutation of
    /// `0..len()` -- e.g. one computed against a playlist that has since
    /// changed length.
    pub fn reorder(&mut self, order: &[usize]) -> bool {
        if !is_permutation(order, self.tracks.len()) {
            return false;
        }
        let mut old: Vec<Option<Track>> = self.tracks.drain(..).map(Some).collect();
        self.tracks = order
            .iter()
            .map(|&i| old[i].take().expect("permutation visits each index once"))
            .collect();
        self.current_index = self
            .current_index
            .and_then(|current| order.iter().position(|&i| i == current));
        true
    }

    /// Replaces every track's metadata (one entry per track, in order),
    /// keeping order and `current_index` untouched. Ignored on a length
    /// mismatch.
    pub fn replace_metadata(&mut self, metadata: Vec<TrackMetadata>) {
        if metadata.len() != self.tracks.len() {
            return;
        }
        for (track, metadata) in self.tracks.iter_mut().zip(metadata) {
            track.metadata = metadata;
        }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = None;
    }
}

/// `true` if `order` contains every index in `0..len` exactly once.
pub(crate) fn is_permutation(order: &[usize], len: usize) -> bool {
    if order.len() != len {
        return false;
    }
    let mut seen = vec![false; len];
    order
        .iter()
        .all(|&i| i < len && !std::mem::replace(&mut seen[i], true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str) -> Track {
        Track {
            path: PathBuf::from(name),
            metadata: TrackMetadata::default(),
        }
    }

    fn names(playlist: &Playlist) -> Vec<String> {
        playlist
            .tracks()
            .iter()
            .map(|t| t.path.display().to_string())
            .collect()
    }

    #[test]
    fn reorder_moves_tracks_and_follows_the_current_one() {
        let mut playlist = Playlist::from_tracks(vec![track("a"), track("b"), track("c")]);
        playlist.set_current_index(0);
        assert!(playlist.reorder(&[2, 0, 1]));
        assert_eq!(names(&playlist), ["c", "a", "b"]);
        assert_eq!(playlist.current_index(), Some(1));
    }

    #[test]
    fn reorder_rejects_non_permutations() {
        let mut playlist = Playlist::from_tracks(vec![track("a"), track("b")]);
        assert!(!playlist.reorder(&[0, 0]));
        assert!(!playlist.reorder(&[0]));
        assert!(!playlist.reorder(&[0, 2]));
        assert_eq!(names(&playlist), ["a", "b"]);
    }
}
