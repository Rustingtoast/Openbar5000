use rand::seq::SliceRandom;

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
}
