use rand::seq::{IndexedRandom, SliceRandom};

/// A randomized play order over `0..len` track indices that cycles fairly:
/// every index appears exactly once per cycle before any index repeats,
/// which is what "shuffle" means to a listener (never an early repeat
/// partway through the playlist).
pub struct ShuffleOrder {
    order: Vec<usize>,
    position: usize,
}

impl ShuffleOrder {
    /// Builds a fresh shuffled cycle over `0..len`. If `keep_first` is
    /// `Some(index)` -- the currently playing track -- it's placed first so
    /// turning shuffle on mid-playback doesn't interrupt what's already
    /// playing; the rest of the cycle is a random permutation of every
    /// other index.
    pub fn new(len: usize, keep_first: Option<usize>) -> Self {
        let mut order: Vec<usize> = (0..len).collect();
        match keep_first.filter(|&i| i < len) {
            Some(first) => {
                order.retain(|&i| i != first);
                order.shuffle(&mut rand::rng());
                order.insert(0, first);
            }
            None => order.shuffle(&mut rand::rng()),
        }
        Self { order, position: 0 }
    }

    pub fn current(&self) -> Option<usize> {
        self.order.get(self.position).copied()
    }

    /// Advances to the next track in the cycle, reshuffling into a brand
    /// new cycle once every track has played once. Returns `None` only when
    /// there are no tracks at all.
    pub fn advance(&mut self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        let just_played = self.current();
        self.position += 1;
        if self.position >= self.order.len() {
            self.reshuffle_avoiding_immediate_repeat(just_played);
        }
        self.current()
    }

    /// Moves back to the previous track in the current cycle, or does
    /// nothing if already at its start (mirrors `Playlist::retreat`).
    pub fn retreat(&mut self) -> Option<usize> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        self.current()
    }

    /// The Random button in shuffle mode: jumps to a random track that
    /// `allowed` accepts among those this cycle hasn't played yet, and the
    /// cycle carries on from it -- it becomes the next step, so everything
    /// else still plays once before any repeat, and `retreat` goes back to
    /// the track before. `heard_current` says whether the cycle's current
    /// track has been played (rather than just waiting to start); if it
    /// hasn't, it can be picked too.
    ///
    /// Once every allowed track has played this cycle, a new cycle starts
    /// with one of them -- not the one just heard, unless it's the only
    /// one. `None`, changing nothing, when `allowed` accepts no track.
    pub fn jump_to_random(&mut self, allowed: impl Fn(usize) -> bool, heard_current: bool) -> Option<usize> {
        if !self.order.iter().any(|&i| allowed(i)) {
            return None;
        }
        let next = if heard_current { self.position + 1 } else { self.position };
        let unplayed: Vec<usize> = (next..self.order.len())
            .filter(|&p| allowed(self.order[p]))
            .collect();
        if let Some(&pick) = unplayed.choose(&mut rand::rng()) {
            self.order.swap(next, pick);
            self.position = next;
            return self.current();
        }

        let just_played = self.current().filter(|_| heard_current);
        self.order.shuffle(&mut rand::rng());
        let allowed_positions: Vec<usize> = (0..self.order.len())
            .filter(|&p| allowed(self.order[p]))
            .collect();
        let others: Vec<usize> = allowed_positions
            .iter()
            .copied()
            .filter(|&p| Some(self.order[p]) != just_played)
            .collect();
        let pick = others
            .choose(&mut rand::rng())
            .or(allowed_positions.first())
            .copied()?;
        self.order.swap(0, pick);
        self.position = 0;
        self.current()
    }

    /// Every track still plays before any repeat regardless of this -- it
    /// only avoids the jarring special case of the same track playing
    /// twice back-to-back right at the seam between one cycle and the next.
    fn reshuffle_avoiding_immediate_repeat(&mut self, just_played: Option<usize>) {
        self.order.shuffle(&mut rand::rng());
        if self.order.len() > 1 && self.order.first().copied() == just_played {
            self.order.swap(0, 1);
        }
        self.position = 0;
    }
}

/// A random track out of `candidates` for the Random button, never
/// `playing` (unless it's the only candidate), so pressing it always
/// changes the song. `None` only when there are no candidates.
pub fn random_track(candidates: &[usize], playing: Option<usize>) -> Option<usize> {
    let others: Vec<usize> = candidates.iter().copied().filter(|&i| Some(i) != playing).collect();
    others.choose(&mut rand::rng()).or(candidates.first()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn advance_visits_every_track_exactly_once_before_repeating() {
        let mut shuffle = ShuffleOrder::new(5, None);
        let mut visited = HashSet::new();
        visited.insert(shuffle.current().unwrap());
        for _ in 0..4 {
            visited.insert(shuffle.advance().unwrap());
        }
        assert_eq!(visited, (0..5).collect());
    }

    #[test]
    fn reshuffles_into_a_new_full_cycle_after_exhausting_one() {
        const LEN: usize = 5;
        let mut shuffle = ShuffleOrder::new(LEN, None);

        // Exhaust the first cycle: the starting track plus LEN-1 advances.
        let mut first_cycle = HashSet::new();
        first_cycle.insert(shuffle.current().unwrap());
        for _ in 0..LEN - 1 {
            first_cycle.insert(shuffle.advance().unwrap());
        }
        assert_eq!(first_cycle, (0..LEN).collect());

        // The very next `advance()` crosses into a freshly reshuffled
        // cycle; those LEN results must again cover every track exactly
        // once (not `current()` plus LEN-1 advances -- `current()` here
        // would still be the *previous* cycle's last track).
        let mut second_cycle = HashSet::new();
        for _ in 0..LEN {
            second_cycle.insert(shuffle.advance().unwrap());
        }
        assert_eq!(second_cycle, (0..LEN).collect());
    }

    #[test]
    fn new_with_keep_first_starts_on_that_track_without_losing_any_other() {
        let shuffle = ShuffleOrder::new(5, Some(2));
        assert_eq!(shuffle.current(), Some(2));
    }

    #[test]
    fn retreat_is_a_no_op_at_the_start_of_a_cycle() {
        let mut shuffle = ShuffleOrder::new(3, None);
        let first = shuffle.current();
        assert_eq!(shuffle.retreat(), None);
        assert_eq!(shuffle.current(), first);
    }

    #[test]
    fn retreat_undoes_the_previous_advance() {
        let mut shuffle = ShuffleOrder::new(4, None);
        let first = shuffle.current();
        shuffle.advance();
        assert_eq!(shuffle.retreat(), first);
    }

    #[test]
    fn single_track_always_repeats_itself() {
        let mut shuffle = ShuffleOrder::new(1, None);
        assert_eq!(shuffle.current(), Some(0));
        assert_eq!(shuffle.advance(), Some(0));
        assert_eq!(shuffle.advance(), Some(0));
    }

    #[test]
    fn empty_playlist_never_yields_a_track() {
        let mut shuffle = ShuffleOrder::new(0, None);
        assert_eq!(shuffle.current(), None);
        assert_eq!(shuffle.advance(), None);
    }

    #[test]
    fn random_track_reaches_every_other_candidate_but_never_the_one_playing() {
        let mut picked = HashSet::new();
        for _ in 0..1000 {
            let pick = random_track(&[0, 1, 2, 3, 4], Some(2)).unwrap();
            assert_ne!(pick, 2);
            picked.insert(pick);
        }
        assert_eq!(picked, HashSet::from([0, 1, 3, 4]));
    }

    #[test]
    fn random_track_only_picks_candidates() {
        let picked: HashSet<_> = (0..1000).map(|_| random_track(&[1, 4, 7], Some(4)).unwrap()).collect();
        assert_eq!(picked, HashSet::from([1, 7]));
        // The track playing isn't a candidate (a search hides it): any
        // candidate can come up.
        let picked: HashSet<_> = (0..1000).map(|_| random_track(&[1, 4], Some(9)).unwrap()).collect();
        assert_eq!(picked, HashSet::from([1, 4]));
    }

    #[test]
    fn random_track_with_nothing_playing_can_pick_any_candidate() {
        let picked: HashSet<_> = (0..1000).map(|_| random_track(&[0, 1, 2], None).unwrap()).collect();
        assert_eq!(picked, HashSet::from([0, 1, 2]));
    }

    #[test]
    fn random_track_replays_a_lone_candidate_and_finds_none_without_any() {
        assert_eq!(random_track(&[3], Some(3)), Some(3));
        assert_eq!(random_track(&[3], None), Some(3));
        assert_eq!(random_track(&[], None), None);
        assert_eq!(random_track(&[], Some(0)), None);
    }

    #[test]
    fn jump_to_random_keeps_the_cycle_fair() {
        const LEN: usize = 8;
        let mut shuffle = ShuffleOrder::new(LEN, Some(5));
        let mut heard = vec![shuffle.current().unwrap()];
        heard.push(shuffle.advance().unwrap());
        // Random twice, then let the cycle run out: every track exactly once.
        heard.push(shuffle.jump_to_random(|_| true, true).unwrap());
        heard.push(shuffle.jump_to_random(|_| true, true).unwrap());
        for _ in 0..LEN - 4 {
            heard.push(shuffle.advance().unwrap());
        }
        let distinct: HashSet<_> = heard.iter().copied().collect();
        assert_eq!(distinct, (0..LEN).collect(), "a track repeated within the cycle: {heard:?}");
    }

    #[test]
    fn retreat_after_jump_to_random_goes_back_to_the_track_before() {
        let mut shuffle = ShuffleOrder::new(6, Some(0));
        let before = shuffle.advance();
        let jumped = shuffle.jump_to_random(|_| true, true);
        assert_ne!(jumped, before);
        assert_eq!(shuffle.retreat(), before);
    }

    #[test]
    fn jump_to_random_only_picks_allowed_tracks() {
        for _ in 0..200 {
            let mut shuffle = ShuffleOrder::new(10, Some(1));
            for _ in 0..8 {
                // Across cycles too: 5 even tracks, 8 jumps.
                let pick = shuffle.jump_to_random(|i| i % 2 == 0, true).unwrap();
                assert_eq!(pick % 2, 0);
            }
        }
    }

    #[test]
    fn jump_to_random_starts_a_new_cycle_once_every_allowed_track_played() {
        for _ in 0..200 {
            let mut shuffle = ShuffleOrder::new(6, Some(0));
            let allowed = |i: usize| i == 2 || i == 3;
            let first = shuffle.jump_to_random(allowed, true).unwrap();
            let second = shuffle.jump_to_random(allowed, true).unwrap();
            assert_eq!(HashSet::from([first, second]), HashSet::from([2, 3]));
            // Both played: a new cycle, which doesn't start on the track
            // just heard.
            assert_eq!(shuffle.jump_to_random(allowed, true), Some(first));
            assert_eq!(shuffle.retreat(), None, "a new cycle starts at its beginning");
        }
    }

    #[test]
    fn jump_to_random_can_pick_a_cycle_start_not_yet_heard() {
        let mut shuffle = ShuffleOrder::new(4, None);
        let start = shuffle.current().unwrap();
        assert_eq!(shuffle.jump_to_random(|i| i == start, false), Some(start));
        // Heard: it's this cycle's only allowed track, so a new cycle
        // replays it.
        assert_eq!(shuffle.jump_to_random(|i| i == start, true), Some(start));
    }

    #[test]
    fn jump_to_random_with_nothing_allowed_changes_nothing() {
        let mut shuffle = ShuffleOrder::new(5, Some(3));
        let next = shuffle.clone_order();
        assert_eq!(shuffle.jump_to_random(|_| false, true), None);
        assert_eq!(shuffle.clone_order(), next);
        assert_eq!(shuffle.current(), Some(3));
    }

    impl ShuffleOrder {
        fn clone_order(&self) -> Vec<usize> {
            self.order.clone()
        }
    }
}
