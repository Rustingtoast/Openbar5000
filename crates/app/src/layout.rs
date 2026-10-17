//! The main window as a tree of panels (issue #22): every node is either a
//! panel or a horizontal/vertical split of child nodes, each child sized
//! either in fixed pixels or as a share of the remaining space. Rust owns
//! the tree and turns it into flat lists of placed rectangles (panel slots
//! and divider handles) that the Slint side draws -- Slint can't recurse
//! into a component tree of arbitrary depth itself.

use serde::{Deserialize, Serialize};

/// Smallest size (px) a divider drag can shrink a panel to.
const MIN_PANEL_SIZE: f32 = 24.0;
/// Width of the playback buttons strip in the default layout.
const DEFAULT_PLAYBACK_BUTTONS_WIDTH: f32 = 140.0;
/// Height of a tab group's strip of tab buttons.
pub const TAB_STRIP_HEIGHT: f32 = 22.0;
/// Smallest default album-art square (px), used when there's little or
/// nothing beside it to match.
const DEFAULT_ALBUM_ART_SIZE: f32 = 64.0;
/// Thickness of a divider's grab area, centered on the boundary.
pub const DIVIDER_THICKNESS: f32 = 6.0;

/// Every panel type the window knows how to draw. This is the panel
/// registry: a new panel is a new variant here (with its id, label and
/// default size) plus a matching branch in `main_window.slint`'s slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanelKind {
    Playlist,
    Waveform,
    Spectrum,
    Transport,
    StatusBar,
    /// The current track's cover art (issue #10).
    AlbumArt,
    /// One line: track number, title, length, artist, album (issue #10).
    NowPlaying,
    /// Stop / previous / play-pause / next / open, as one compact strip.
    PlaybackButtons,
    /// Elapsed time, seek bar, remaining time, repeat/shuffle and volume.
    SeekBar,
    /// Just the seek slider -- inset like the waveform, so placed above it
    /// the two line up.
    SeekSlider,
    /// Elapsed / remaining time, repeat/shuffle and volume.
    PlaybackOptions,
    /// The current track's tags and technical details.
    Info,
    /// Synced or plain lyrics for the current track (see `crate::lyrics`).
    Lyrics,
    /// Artist biography -- placeholder until there's a source for it.
    Biography,
    /// The media library browser (see `crate::media_library`).
    Library,
    /// The equalizer as a vintage hardware unit with a VFD (issue #7).
    Equalizer,
    /// Placeholder left by Split/Remove until the user picks a panel.
    Empty,
}

impl PanelKind {
    /// Panels offered in "Replace with…", in menu order.
    pub const ALL: [PanelKind; 17] = [
        PanelKind::Playlist,
        PanelKind::NowPlaying,
        PanelKind::AlbumArt,
        PanelKind::Info,
        PanelKind::Lyrics,
        PanelKind::Biography,
        PanelKind::Library,
        PanelKind::Waveform,
        PanelKind::Spectrum,
        PanelKind::Equalizer,
        PanelKind::PlaybackButtons,
        PanelKind::SeekSlider,
        PanelKind::PlaybackOptions,
        PanelKind::SeekBar,
        PanelKind::Transport,
        PanelKind::StatusBar,
        PanelKind::Empty,
    ];

    /// Stable id shared with Slint (and the saved layout).
    pub fn id(self) -> &'static str {
        match self {
            Self::Playlist => "playlist",
            Self::Waveform => "waveform",
            Self::Spectrum => "spectrum",
            Self::Transport => "transport",
            Self::StatusBar => "status-bar",
            Self::AlbumArt => "album-art",
            Self::NowPlaying => "now-playing",
            Self::PlaybackButtons => "playback-buttons",
            Self::SeekBar => "seek-bar",
            Self::SeekSlider => "seek-slider",
            Self::PlaybackOptions => "playback-options",
            Self::Info => "info",
            Self::Lyrics => "lyrics",
            Self::Biography => "biography",
            Self::Library => "library",
            Self::Equalizer => "equalizer",
            Self::Empty => "empty",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Playlist => "Playlist",
            Self::Waveform => "Waveform seekbar",
            Self::Spectrum => "Spectrum analyzer",
            Self::Transport => "Playback controls",
            Self::StatusBar => "Status bar",
            Self::AlbumArt => "Album art",
            Self::NowPlaying => "Now playing",
            Self::PlaybackButtons => "Playback buttons",
            Self::SeekBar => "Seek bar (with controls)",
            Self::SeekSlider => "Seek slider",
            Self::PlaybackOptions => "Playback options",
            Self::Info => "Track info",
            Self::Lyrics => "Lyrics",
            Self::Biography => "Biography",
            Self::Library => "Library",
            Self::Equalizer => "Vintage equalizer",
            Self::Empty => "Empty panel",
        }
    }

    /// Short tab caption, like the reference UI's "INF / BIO / LYR / ART /
    /// LIB" tab strip.
    pub fn tab_label(self) -> &'static str {
        match self {
            Self::Info => "INF",
            Self::Biography => "BIO",
            Self::Lyrics => "LYR",
            Self::AlbumArt => "ART",
            Self::Library => "LIB",
            Self::Playlist => "PLS",
            Self::NowPlaying => "NOW",
            Self::Waveform => "WAV",
            Self::Spectrum => "SPC",
            Self::Equalizer => "EQ",
            Self::PlaybackButtons
            | Self::SeekBar
            | Self::SeekSlider
            | Self::PlaybackOptions
            | Self::Transport => "CTL",
            Self::StatusBar => "STS",
            Self::Empty => "---",
        }
    }

    /// The panel's natural height when stacked vertically; `None` for
    /// panels that should take a share of whatever space is left.
    fn natural_height(self) -> Option<f32> {
        match self {
            Self::Waveform => Some(80.0),
            Self::Spectrum => Some(150.0),
            Self::Equalizer => Some(190.0),
            Self::Transport => Some(32.0),
            Self::StatusBar => Some(22.0),
            Self::NowPlaying | Self::PlaybackButtons => Some(28.0),
            Self::SeekBar => Some(26.0),
            Self::SeekSlider | Self::PlaybackOptions => Some(22.0),
            Self::Playlist
            | Self::AlbumArt
            | Self::Info
            | Self::Lyrics
            | Self::Biography
            | Self::Library
            | Self::Empty => None,
        }
    }

    /// Size a panel gets when inserted into a split along `axis`.
    fn default_size(self, axis: Axis) -> Size {
        match (axis, self.natural_height()) {
            (Axis::Vertical, Some(height)) => Size::Fixed(height),
            _ => Size::Weight(1.0),
        }
    }
}

/// Direction children of a split are laid out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Axis {
    /// Side by side, left to right.
    Horizontal,
    /// Stacked, top to bottom.
    Vertical,
}

/// How much room a split's child takes along the split's axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Size {
    /// Exactly this many logical pixels (scaled down if they don't fit).
    Fixed(f32),
    /// A share of the space the fixed children leave over.
    Weight(f32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Node {
    Panel(PanelKind),
    Split { axis: Axis, children: Vec<Child> },
    /// Several nodes sharing one area, one shown at a time under a tab strip.
    Tabs { tabs: Vec<Node>, active: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Child {
    pub size: Size,
    pub node: Node,
}

/// Position of a node in the tree: the child index taken at each split,
/// from the root.
pub type Path = Vec<usize>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A placed panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub path: Path,
    pub kind: PanelKind,
    pub rect: Rect,
}

/// The draggable boundary between `split`'s children `index` and
/// `index + 1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub split: Path,
    pub index: usize,
    pub axis: Axis,
    pub rect: Rect,
}

/// A tab group's strip of tab buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct TabStrip {
    pub path: Path,
    pub rect: Rect,
    pub labels: Vec<&'static str>,
    pub active: usize,
}

#[derive(Debug, Default, PartialEq)]
pub struct Placement {
    pub slots: Vec<Slot>,
    pub dividers: Vec<Divider>,
    pub tab_strips: Vec<TabStrip>,
}

/// The whole-window layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub root: Node,
}

impl Layout {
    /// The default window, following the reference screenshots (issues #1
    /// and #10), top to bottom:
    /// - now-playing line, playback buttons at its right;
    /// - a tab group (INF / BIO / LYR / ART / LIB) beside the playlist;
    /// - seek slider over waveform over spectrum, with the playback options
    ///   over the album art to their right;
    /// - the status bar.
    ///
    /// `show_waveform`/`show_spectrum` carry over the visibility toggles
    /// that existed before panels did.
    pub fn default_with(show_waveform: bool, show_spectrum: bool) -> Self {
        let panel = |kind: PanelKind| Child {
            size: kind.default_size(Axis::Vertical),
            node: Node::Panel(kind),
        };
        let weighted = |node: Node| Child {
            size: Size::Weight(1.0),
            node,
        };
        // Left column: seek slider, waveform and spectrum -- one width, so
        // the slider and the waveform line up. Right column: the playback
        // options above the album art (issue #10).
        let mut left = vec![panel(PanelKind::SeekSlider)];
        if show_waveform {
            left.push(panel(PanelKind::Waveform));
        }
        if show_spectrum {
            left.push(panel(PanelKind::Spectrum));
        }
        let row_height: f32 = left
            .iter()
            .map(|child| match child.size {
                Size::Fixed(px) => px,
                Size::Weight(_) => 0.0,
            })
            .sum::<f32>()
            .max(DEFAULT_ALBUM_ART_SIZE);
        let right = Node::Split {
            axis: Axis::Vertical,
            children: vec![
                panel(PanelKind::PlaybackOptions),
                weighted(Node::Panel(PanelKind::AlbumArt)),
            ],
        };
        let bottom_row = Node::Split {
            axis: Axis::Horizontal,
            children: vec![
                weighted(Node::Split {
                    axis: Axis::Vertical,
                    children: left,
                }),
                Child {
                    size: Size::Fixed(row_height),
                    node: right,
                },
            ],
        };
        let top_row = Node::Split {
            axis: Axis::Horizontal,
            children: vec![
                weighted(Node::Panel(PanelKind::NowPlaying)),
                Child {
                    size: Size::Fixed(DEFAULT_PLAYBACK_BUTTONS_WIDTH),
                    node: Node::Panel(PanelKind::PlaybackButtons),
                },
            ],
        };
        let side_tabs = Node::Tabs {
            tabs: [
                PanelKind::Info,
                PanelKind::Biography,
                PanelKind::Lyrics,
                PanelKind::AlbumArt,
                PanelKind::Library,
            ]
            .into_iter()
            .map(Node::Panel)
            .collect(),
            // LYR, as in the reference.
            active: 2,
        };
        let middle = Node::Split {
            axis: Axis::Horizontal,
            children: vec![weighted(side_tabs), weighted(Node::Panel(PanelKind::Playlist))],
        };
        Self {
            root: Node::Split {
                axis: Axis::Vertical,
                children: vec![
                    Child {
                        size: Size::Fixed(28.0),
                        node: top_row,
                    },
                    weighted(middle),
                    Child {
                        size: Size::Fixed(row_height),
                        node: bottom_row,
                    },
                    panel(PanelKind::StatusBar),
                ],
            },
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("layout always serializes")
    }

    /// `None` if `json` isn't a valid saved layout (e.g. from a newer
    /// build); callers fall back to the default.
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    #[cfg(test)]
    pub fn contains(&self, kind: PanelKind) -> bool {
        fn walk(node: &Node, kind: PanelKind) -> bool {
            match node {
                Node::Panel(k) => *k == kind,
                Node::Split { children, .. } => children.iter().any(|c| walk(&c.node, kind)),
                Node::Tabs { tabs, .. } => tabs.iter().any(|tab| walk(tab, kind)),
            }
        }
        walk(&self.root, kind)
    }

    /// Places every panel and divider inside a `width` x `height` area.
    pub fn place(&self, width: f32, height: f32) -> Placement {
        let mut placement = Placement::default();
        place_node(
            &self.root,
            Rect {
                x: 0.0,
                y: 0.0,
                width: width.max(0.0),
                height: height.max(0.0),
            },
            &mut Vec::new(),
            &mut placement,
        );
        placement
    }

    fn node_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let mut node = &mut self.root;
        for &i in path {
            node = match node {
                Node::Split { children, .. } => &mut children.get_mut(i)?.node,
                Node::Tabs { tabs, .. } => tabs.get_mut(i)?,
                Node::Panel(_) => return None,
            };
        }
        Some(node)
    }

    pub fn replace(&mut self, path: &[usize], kind: PanelKind) {
        if let Some(node @ Node::Panel(_)) = self.node_mut(path) {
            *node = Node::Panel(kind);
        }
    }

    /// Splits the panel at `path` in two along `axis`: it keeps its kind
    /// and gets an empty neighbour after it, each taking half.
    pub fn split(&mut self, path: &[usize], axis: Axis) {
        let Some(node) = self.node_mut(path) else {
            return;
        };
        let Node::Panel(kind) = *node else {
            return;
        };
        *node = Node::Split {
            axis,
            children: vec![
                Child {
                    size: Size::Weight(1.0),
                    node: Node::Panel(kind),
                },
                Child {
                    size: Size::Weight(1.0),
                    node: Node::Panel(PanelKind::Empty),
                },
            ],
        };
    }

    /// Removes the panel at `path`. A split left with a single child is
    /// replaced by that child; removing the last panel leaves an empty one.
    pub fn remove(&mut self, path: &[usize]) {
        let Some((&index, parent_path)) = path.split_last() else {
            self.root = Node::Panel(PanelKind::Empty);
            return;
        };
        if let Some(Node::Tabs { tabs, active }) = self.node_mut(parent_path) {
            if index >= tabs.len() {
                return;
            }
            tabs.remove(index);
            if tabs.is_empty() {
                if let Some(parent) = self.node_mut(parent_path) {
                    *parent = Node::Panel(PanelKind::Empty);
                }
            } else {
                // Tabs after the removed one shift left; removing the one
                // showing shows the tab before it.
                if *active > index || (*active == index && index > 0) {
                    *active -= 1;
                }
                *active = (*active).min(tabs.len() - 1);
            }
            return;
        }
        let Some(Node::Split { children, .. }) = self.node_mut(parent_path) else {
            return;
        };
        if index >= children.len() {
            return;
        }
        children.remove(index);
        if children.len() == 1 {
            let only = children.remove(0).node;
            if let Some(parent) = self.node_mut(parent_path) {
                *parent = only;
            }
        }
    }

    /// Moves the boundary of `divider` by `delta` px along its axis,
    /// starting from the child sizes captured in `start` (see
    /// `drag_start`), so a drag is applied relative to where it began
    /// rather than accumulating rounding.
    pub fn drag(&mut self, start: &DragStart, delta: f32) {
        let Some(Node::Split { children, .. }) = self.node_mut(&start.split) else {
            return;
        };
        let i = start.index;
        if i + 1 >= children.len() {
            return;
        }
        let total = start.before_px + start.after_px;
        let before = (start.before_px + delta).clamp(MIN_PANEL_SIZE, (total - MIN_PANEL_SIZE).max(MIN_PANEL_SIZE));
        let after = total - before;
        match (children[i].size, children[i + 1].size) {
            (Size::Weight(a), Size::Weight(b)) => {
                let weight = a + b;
                children[i].size = Size::Weight(weight * before / total.max(1.0));
                children[i + 1].size = Size::Weight(weight * after / total.max(1.0));
            }
            // A weighted neighbour absorbs the change by itself.
            (Size::Fixed(_), Size::Weight(_)) => children[i].size = Size::Fixed(before),
            (Size::Weight(_), Size::Fixed(_)) => children[i + 1].size = Size::Fixed(after),
            (Size::Fixed(_), Size::Fixed(_)) => {
                children[i].size = Size::Fixed(before);
                children[i + 1].size = Size::Fixed(after);
            }
        }
    }
}

/// Where a dragged panel lands relative to the panel it's dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    /// Trade places with the target.
    Swap,
    /// Sit beside the target on this side, sharing its space.
    Beside(Side),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn axis(self) -> Axis {
        match self {
            Side::Left | Side::Right => Axis::Horizontal,
            Side::Top | Side::Bottom => Axis::Vertical,
        }
    }

    /// Whether the dropped panel goes before the target along the axis.
    fn before(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

/// Which way a panel can be shifted among its siblings (the "Move
/// Left/Right/Up/Down" menu items).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SiblingMoves {
    /// The axis of the split holding the panel; `None` for a lone panel.
    pub axis: Option<Axis>,
    pub can_move_back: bool,
    pub can_move_forward: bool,
}

impl Layout {
    fn node(&self, path: &[usize]) -> Option<&Node> {
        let mut node = &self.root;
        for &i in path {
            node = match node {
                Node::Split { children, .. } => &children.get(i)?.node,
                Node::Tabs { tabs, .. } => tabs.get(i)?,
                Node::Panel(_) => return None,
            };
        }
        Some(node)
    }

    pub fn sibling_moves(&self, path: &[usize]) -> SiblingMoves {
        let Some((&index, parent)) = path.split_last() else {
            return SiblingMoves::default();
        };
        match self.node(parent) {
            Some(Node::Split { axis, children }) => SiblingMoves {
                axis: Some(*axis),
                can_move_back: index > 0,
                can_move_forward: index + 1 < children.len(),
            },
            // Tabs read left to right.
            Some(Node::Tabs { tabs, .. }) => SiblingMoves {
                axis: Some(Axis::Horizontal),
                can_move_back: index > 0,
                can_move_forward: index + 1 < tabs.len(),
            },
            _ => SiblingMoves::default(),
        }
    }

    /// Swaps the node at `path` with its previous (`forward == false`) or
    /// next sibling, sizes included -- the panel moves, its neighbour takes
    /// its old spot.
    pub fn shift(&mut self, path: &[usize], forward: bool) {
        let Some((&index, parent)) = path.split_last() else {
            return;
        };
        let other = if forward { index + 1 } else { index.wrapping_sub(1) };
        match self.node_mut(parent) {
            Some(Node::Split { children, .. }) if index < children.len() && other < children.len() => {
                children.swap(index, other);
            }
            Some(Node::Tabs { tabs, active }) if index < tabs.len() && other < tabs.len() => {
                tabs.swap(index, other);
                // The tab that was showing stays showing.
                if *active == index {
                    *active = other;
                } else if *active == other {
                    *active = index;
                }
            }
            _ => {}
        }
    }

    /// Drags tab `from` of the tab group at `path` so it lands before what
    /// is now tab `to` (`to == len` puts it last), and shows it.
    pub fn move_tab(&mut self, path: &[usize], from: usize, to: usize) {
        let Some(Node::Tabs { tabs, active }) = self.node_mut(path) else {
            return;
        };
        if from >= tabs.len() || to > tabs.len() {
            return;
        }
        let tab = tabs.remove(from);
        // Removing `from` shifts everything after it one to the left.
        let to = if to > from { to - 1 } else { to };
        tabs.insert(to, tab);
        *active = to;
    }

    /// Shows tab `index` of the tab group at `path`.
    pub fn set_active_tab(&mut self, path: &[usize], index: usize) {
        if let Some(Node::Tabs { tabs, active }) = self.node_mut(path) {
            if index < tabs.len() {
                *active = index;
            }
        }
    }

    /// "Add Tab": inside a tab group, adds an empty tab after the panel at
    /// `path` and shows it; anywhere else, turns that panel into a tab
    /// group of itself plus an empty tab.
    pub fn add_tab(&mut self, path: &[usize]) {
        if let Some((&index, parent)) = path.split_last() {
            if let Some(Node::Tabs { tabs, active }) = self.node_mut(parent) {
                tabs.insert(index + 1, Node::Panel(PanelKind::Empty));
                *active = index + 1;
                return;
            }
        }
        if let Some(node @ Node::Panel(_)) = self.node_mut(path) {
            let panel = std::mem::replace(node, Node::Panel(PanelKind::Empty));
            *node = Node::Tabs {
                tabs: vec![panel, Node::Panel(PanelKind::Empty)],
                active: 1,
            };
        }
    }

    /// Drag-and-drop: moves the panel at `from` relative to the panel at
    /// `onto`.
    pub fn drop_panel(&mut self, from: &[usize], onto: &[usize], drop: Drop) {
        if from == onto {
            return;
        }
        let (Some(&Node::Panel(moved)), Some(&Node::Panel(target))) = (self.node(from), self.node(onto))
        else {
            return;
        };
        let side = match drop {
            Drop::Swap => {
                self.replace(from, target);
                self.replace(onto, moved);
                return;
            }
            Drop::Beside(side) => side,
        };

        // Removing `from` can collapse splits and shift `onto`'s path, so
        // tag the target with a marker no real layout contains (a split
        // with no children), remove, then find the marker again.
        let Some(target_node) = self.node_mut(onto) else {
            return;
        };
        *target_node = Node::Split {
            axis: Axis::Horizontal,
            children: Vec::new(),
        };
        self.remove(from);
        let Some(marker) = find_path(&self.root, &mut Vec::new(), &|node| {
            matches!(node, Node::Split { children, .. } if children.is_empty())
        }) else {
            return;
        };

        let axis = side.axis();
        let moved_child = Child {
            size: moved.default_size(axis),
            node: Node::Panel(moved),
        };
        // Already in a split running the right way: become a sibling, so a
        // stack stays one flat stack instead of nesting.
        if let Some((&index, parent)) = marker.split_last() {
            if let Some(Node::Split {
                axis: parent_axis,
                children,
            }) = self.node_mut(parent)
            {
                if *parent_axis == axis {
                    children[index].node = Node::Panel(target);
                    let at = if side.before() { index } else { index + 1 };
                    children.insert(at, moved_child);
                    return;
                }
            }
        }
        let target_child = Child {
            size: Size::Weight(1.0),
            node: Node::Panel(target),
        };
        let children = if side.before() {
            vec![moved_child, target_child]
        } else {
            vec![target_child, moved_child]
        };
        if let Some(node) = self.node_mut(&marker) {
            *node = Node::Split { axis, children };
        }
    }
}

fn find_path(node: &Node, path: &mut Path, pred: &dyn Fn(&Node) -> bool) -> Option<Path> {
    if pred(node) {
        return Some(path.clone());
    }
    let children: Vec<&Node> = match node {
        Node::Split { children, .. } => children.iter().map(|c| &c.node).collect(),
        Node::Tabs { tabs, .. } => tabs.iter().collect(),
        Node::Panel(_) => Vec::new(),
    };
    for (i, child) in children.into_iter().enumerate() {
        path.push(i);
        if let Some(found) = find_path(child, path, pred) {
            return Some(found);
        }
        path.pop();
    }
    None
}

/// Which drop a pointer at (`x`, `y`) inside `rect` means: the outer
/// quarter on any side places beside on that side, the middle swaps.
pub fn drop_at(rect: &Rect, x: f32, y: f32) -> Drop {
    let rx = ((x - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0);
    let ry = ((y - rect.y) / rect.height.max(1.0)).clamp(0.0, 1.0);
    let edges = [
        (rx, Side::Left),
        (1.0 - rx, Side::Right),
        (ry, Side::Top),
        (1.0 - ry, Side::Bottom),
    ];
    let (distance, side) = edges
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("four edges");
    if distance < 0.25 {
        Drop::Beside(side)
    } else {
        Drop::Swap
    }
}

/// The part of `rect` a drop would occupy, for the drag preview.
pub fn drop_preview(rect: &Rect, drop: Drop) -> Rect {
    let half_w = rect.width / 2.0;
    let half_h = rect.height / 2.0;
    match drop {
        Drop::Swap => *rect,
        Drop::Beside(Side::Left) => Rect { width: half_w, ..*rect },
        Drop::Beside(Side::Right) => Rect {
            x: rect.x + half_w,
            width: half_w,
            ..*rect
        },
        Drop::Beside(Side::Top) => Rect { height: half_h, ..*rect },
        Drop::Beside(Side::Bottom) => Rect {
            y: rect.y + half_h,
            height: half_h,
            ..*rect
        },
    }
}

/// The two children's on-screen sizes when a divider drag starts.
#[derive(Debug, Clone, PartialEq)]
pub struct DragStart {
    pub split: Path,
    pub index: usize,
    pub before_px: f32,
    pub after_px: f32,
}

impl DragStart {
    /// Captures the sizes either side of `divider` from the current
    /// placement.
    pub fn new(divider: &Divider, placement: &Placement) -> Option<Self> {
        let child_extent = |child: usize| {
            let mut prefix = divider.split.clone();
            prefix.push(child);
            child_rect_extent(placement, &prefix, divider.axis)
        };
        Some(Self {
            split: divider.split.clone(),
            index: divider.index,
            before_px: child_extent(divider.index)?,
            after_px: child_extent(divider.index + 1)?,
        })
    }
}

/// The full extent along `axis` covered by the slots under `prefix` (a
/// split's child may itself be split, so it can span several slots).
fn child_rect_extent(placement: &Placement, prefix: &[usize], axis: Axis) -> Option<f32> {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for slot in placement.slots.iter().filter(|s| s.path.starts_with(prefix)) {
        let (start, len) = match axis {
            Axis::Horizontal => (slot.rect.x, slot.rect.width),
            Axis::Vertical => (slot.rect.y, slot.rect.height),
        };
        lo = lo.min(start);
        hi = hi.max(start + len);
    }
    (hi >= lo).then_some(hi - lo)
}

fn place_node(node: &Node, rect: Rect, path: &mut Path, out: &mut Placement) {
    let (axis, children) = match node {
        Node::Panel(kind) => {
            out.slots.push(Slot {
                path: path.clone(),
                kind: *kind,
                rect,
            });
            return;
        }
        Node::Split { axis, children } => (*axis, children),
        Node::Tabs { tabs, active } => {
            let active = (*active).min(tabs.len().saturating_sub(1));
            let strip_height = TAB_STRIP_HEIGHT.min(rect.height);
            out.tab_strips.push(TabStrip {
                path: path.clone(),
                rect: Rect {
                    height: strip_height,
                    ..rect
                },
                labels: tabs
                    .iter()
                    .map(|tab| match tab {
                        Node::Panel(kind) => kind.tab_label(),
                        _ => "···",
                    })
                    .collect(),
                active,
            });
            if let Some(tab) = tabs.get(active) {
                path.push(active);
                place_node(
                    tab,
                    Rect {
                        y: rect.y + strip_height,
                        height: rect.height - strip_height,
                        ..rect
                    },
                    path,
                    out,
                );
                path.pop();
            }
            return;
        }
    };

    let length = match axis {
        Axis::Horizontal => rect.width,
        Axis::Vertical => rect.height,
    };
    let fixed: f32 = children
        .iter()
        .filter_map(|c| match c.size {
            Size::Fixed(px) => Some(px.max(0.0)),
            Size::Weight(_) => None,
        })
        .sum();
    let weights: f32 = children
        .iter()
        .filter_map(|c| match c.size {
            Size::Weight(w) => Some(w.max(0.0)),
            Size::Fixed(_) => None,
        })
        .sum();
    // Fixed children shrink proportionally once they alone overflow.
    let fixed_scale = if fixed > length && fixed > 0.0 { length / fixed } else { 1.0 };
    let flexible = (length - fixed * fixed_scale).max(0.0);

    let mut offset = 0.0;
    for (i, child) in children.iter().enumerate() {
        let extent = match child.size {
            Size::Fixed(px) => px.max(0.0) * fixed_scale,
            Size::Weight(w) if weights > 0.0 => flexible * w.max(0.0) / weights,
            Size::Weight(_) => 0.0,
        };
        let child_rect = match axis {
            Axis::Horizontal => Rect {
                x: rect.x + offset,
                width: extent,
                ..rect
            },
            Axis::Vertical => Rect {
                y: rect.y + offset,
                height: extent,
                ..rect
            },
        };
        path.push(i);
        place_node(&child.node, child_rect, path, out);
        path.pop();
        offset += extent;

        if i + 1 < children.len() {
            let half = DIVIDER_THICKNESS / 2.0;
            out.dividers.push(Divider {
                split: path.clone(),
                index: i,
                axis,
                rect: match axis {
                    Axis::Horizontal => Rect {
                        x: rect.x + offset - half,
                        width: DIVIDER_THICKNESS,
                        ..rect
                    },
                    Axis::Vertical => Rect {
                        y: rect.y + offset - half,
                        height: DIVIDER_THICKNESS,
                        ..rect
                    },
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_kinds(placement: &Placement) -> Vec<PanelKind> {
        placement.slots.iter().map(|s| s.kind).collect()
    }

    /// The flat stack the window had before issue #10's default -- a fixed
    /// fixture, so these tests exercise the layout operations rather than
    /// whatever the default happens to be.
    fn classic() -> Layout {
        let panel = |kind: PanelKind| Child {
            size: kind.default_size(Axis::Vertical),
            node: Node::Panel(kind),
        };
        Layout {
            root: Node::Split {
                axis: Axis::Vertical,
                children: vec![
                    panel(PanelKind::Playlist),
                    panel(PanelKind::Waveform),
                    panel(PanelKind::Spectrum),
                    panel(PanelKind::Transport),
                    panel(PanelKind::StatusBar),
                ],
            },
        }
    }

    #[test]
    fn default_layout_follows_the_reference_screenshot() {
        let placed = Layout::default_with(true, true).place(1400.0, 700.0);
        assert_eq!(
            slot_kinds(&placed),
            [
                PanelKind::NowPlaying,
                PanelKind::PlaybackButtons,
                PanelKind::Lyrics,
                PanelKind::Playlist,
                PanelKind::SeekSlider,
                PanelKind::Waveform,
                PanelKind::Spectrum,
                PanelKind::PlaybackOptions,
                PanelKind::AlbumArt,
                PanelKind::StatusBar
            ]
        );
        let rect = |kind| placed.slots.iter().find(|s| s.kind == kind).unwrap().rect;
        // Buttons at the top right, beside the now-playing line.
        let buttons = rect(PanelKind::PlaybackButtons);
        assert_eq!((buttons.x + buttons.width, buttons.y), (1400.0, 0.0));
        // Tabs (strip + lyrics) on the left half, playlist on the right.
        let strip = &placed.tab_strips[0];
        assert_eq!(strip.labels, ["INF", "BIO", "LYR", "ART", "LIB"]);
        assert_eq!(strip.active, 2);
        assert_eq!(rect(PanelKind::Lyrics).y, 28.0 + TAB_STRIP_HEIGHT);
        assert_eq!(rect(PanelKind::Playlist).x, 700.0);
        // The seek slider sits right above the waveform with the same x
        // range, so positions in the two line up.
        let (slider, waveform) = (rect(PanelKind::SeekSlider), rect(PanelKind::Waveform));
        assert_eq!((slider.x, slider.width), (waveform.x, waveform.width));
        assert_eq!(slider.y + slider.height, waveform.y);
        // Playback options above the cover, at the right of that column.
        let (options, art) = (rect(PanelKind::PlaybackOptions), rect(PanelKind::AlbumArt));
        assert_eq!((options.x, options.y), (art.x, slider.y));
        assert_eq!(options.y + options.height, art.y);
        assert_eq!(art.x + art.width, 1400.0);
        assert_eq!(art.width, 22.0 + 80.0 + 150.0);
        assert_eq!(rect(PanelKind::Playlist).height, 700.0 - 28.0 - 252.0 - 22.0);

        // Old visibility toggles still carry over.
        let without = Layout::default_with(false, true);
        assert!(!without.contains(PanelKind::Waveform));
        assert!(without.contains(PanelKind::Spectrum));
    }

    #[test]
    fn tab_groups_switch_add_move_and_remove_tabs() {
        let mut layout = Layout::default_with(true, true);
        let tabs_path = [1, 0];
        layout.set_active_tab(&tabs_path, 0);
        assert_eq!(layout.place(1400.0, 700.0).slots[2].kind, PanelKind::Info);
        // Moving the showing tab keeps it showing.
        layout.shift(&[1, 0, 0], true);
        let placed = layout.place(1400.0, 700.0);
        assert_eq!(placed.tab_strips[0].labels[..2], ["BIO", "INF"]);
        assert_eq!(placed.tab_strips[0].active, 1);
        // Add a tab after it: empty and shown.
        layout.add_tab(&[1, 0, 1]);
        let placed = layout.place(1400.0, 700.0);
        assert_eq!(placed.tab_strips[0].labels.len(), 6);
        assert_eq!(placed.slots[2].kind, PanelKind::Empty);
        // Remove it again: the tab before it shows.
        layout.remove(&[1, 0, 2]);
        let placed = layout.place(1400.0, 700.0);
        assert_eq!(placed.tab_strips[0].labels.len(), 5);
        assert_eq!(placed.slots[2].kind, PanelKind::Info);
        // Dragging tabs reorders them; the dragged tab shows.
        let mut dragged = Layout::default_with(true, true);
        dragged.move_tab(&tabs_path, 0, 3); // INF to before ART
        let placed = dragged.place(1400.0, 700.0);
        assert_eq!(placed.tab_strips[0].labels, ["BIO", "LYR", "INF", "ART", "LIB"]);
        assert_eq!(placed.tab_strips[0].active, 2);
        dragged.move_tab(&tabs_path, 4, 0); // LIB to the front
        assert_eq!(dragged.place(1400.0, 700.0).tab_strips[0].labels[0], "LIB");
        dragged.move_tab(&tabs_path, 1, 5); // BIO to the end
        assert_eq!(dragged.place(1400.0, 700.0).tab_strips[0].labels[4], "BIO");

        // "Add Tab" on a plain panel turns it into a tab group.
        layout.add_tab(&[1, 1]);
        let placed = layout.place(1400.0, 700.0);
        assert_eq!(placed.tab_strips.len(), 2);
        assert_eq!(placed.tab_strips[1].labels, ["PLS", "---"]);
        // Removing every tab leaves an empty panel.
        for _ in 0..2 {
            layout.remove(&[1, 1, 0]);
        }
        assert!(matches!(layout.place(1400.0, 700.0).slots[3].kind, PanelKind::Empty));
        // Tab groups survive saving.
        assert_eq!(Layout::from_json(&layout.to_json()), Some(layout));
    }

    #[test]
    fn fixed_panels_shrink_when_the_window_is_too_small() {
        let placed = classic().place(500.0, 95.0);
        let total: f32 = placed.slots.iter().map(|s| s.rect.height).sum();
        assert!((total - 95.0).abs() < 0.01);
        assert_eq!(placed.slots[0].rect.height, 0.0);
    }

    #[test]
    fn split_remove_and_replace_reshape_the_tree() {
        let mut layout = classic();
        // Put a side panel left of the playlist: split it horizontally.
        layout.split(&[0], Axis::Horizontal);
        let placed = layout.place(1000.0, 700.0);
        assert_eq!(placed.slots[0].kind, PanelKind::Playlist);
        assert_eq!(placed.slots[1].kind, PanelKind::Empty);
        assert_eq!(placed.slots[0].rect.width, 500.0);
        assert_eq!(placed.slots[1].rect.x, 500.0);

        layout.replace(&[0, 1], PanelKind::Spectrum);
        assert_eq!(layout.place(1000.0, 700.0).slots[1].kind, PanelKind::Spectrum);

        // Removing one side collapses the split back into a single panel.
        layout.remove(&[0, 1]);
        assert_eq!(layout, {
            let mut expected = classic();
            expected.replace(&[0], PanelKind::Playlist);
            expected
        });

        let mut single = Layout {
            root: Node::Panel(PanelKind::Playlist),
        };
        single.remove(&[]);
        assert_eq!(single.root, Node::Panel(PanelKind::Empty));
    }

    #[test]
    fn dragging_a_divider_resizes_both_neighbours() {
        let mut layout = classic();
        layout.split(&[0], Axis::Horizontal);
        let placed = layout.place(1000.0, 700.0);
        // The vertical divider between the two halves of the old playlist.
        let divider = placed
            .dividers
            .iter()
            .find(|d| d.axis == Axis::Horizontal)
            .unwrap()
            .clone();
        let start = DragStart::new(&divider, &placed).unwrap();
        assert_eq!((start.before_px, start.after_px), (500.0, 500.0));

        layout.drag(&start, -200.0);
        let placed = layout.place(1000.0, 700.0);
        assert_eq!(placed.slots[0].rect.width, 300.0);
        assert_eq!(placed.slots[1].rect.width, 700.0);
        // Weighted sizes stay proportional when the window resizes.
        let wider = layout.place(2000.0, 700.0);
        assert_eq!(wider.slots[0].rect.width, 600.0);

        // Dragging a fixed panel's edge changes its pixel size, never past
        // the minimum.
        let placed = layout.place(1000.0, 700.0);
        let above_waveform = placed
            .dividers
            .iter()
            .find(|d| d.axis == Axis::Vertical && d.index == 0)
            .unwrap()
            .clone();
        let start = DragStart::new(&above_waveform, &placed).unwrap();
        layout.drag(&start, -52.0);
        let placed = layout.place(1000.0, 700.0);
        let waveform = placed.slots.iter().find(|s| s.kind == PanelKind::Waveform).unwrap();
        assert_eq!(waveform.rect.height, 132.0);
        layout.drag(&start, 1000.0);
        let placed = layout.place(1000.0, 700.0);
        let waveform = placed.slots.iter().find(|s| s.kind == PanelKind::Waveform).unwrap();
        assert_eq!(waveform.rect.height, MIN_PANEL_SIZE);
    }

    fn kinds_in_order(layout: &Layout) -> Vec<PanelKind> {
        slot_kinds(&layout.place(1000.0, 700.0))
    }

    #[test]
    fn shifting_moves_a_panel_among_its_siblings() {
        let mut layout = classic();
        assert_eq!(
            layout.sibling_moves(&[0]),
            SiblingMoves { axis: Some(Axis::Vertical), can_move_back: false, can_move_forward: true }
        );
        // Move the waveform below the spectrum; its fixed size goes with it.
        layout.shift(&[1], true);
        assert_eq!(
            kinds_in_order(&layout)[..3],
            [PanelKind::Playlist, PanelKind::Spectrum, PanelKind::Waveform]
        );
        let placed = layout.place(1000.0, 700.0);
        assert_eq!(placed.slots[2].rect.height, 80.0);
        // Ends of the split can't move further.
        layout.shift(&[0], false);
        layout.shift(&[4], true);
        assert_eq!(kinds_in_order(&layout)[0], PanelKind::Playlist);
        assert_eq!(Layout { root: Node::Panel(PanelKind::Playlist) }.sibling_moves(&[]), SiblingMoves::default());
    }

    #[test]
    fn dropping_in_the_middle_swaps_panels() {
        let mut layout = classic();
        layout.drop_panel(&[3], &[0], Drop::Swap);
        let kinds = kinds_in_order(&layout);
        assert_eq!(kinds[0], PanelKind::Transport);
        assert_eq!(kinds[3], PanelKind::Playlist);
    }

    #[test]
    fn dropping_on_an_edge_places_beside_and_collapses_the_old_spot() {
        // Drag the spectrum to the left of the playlist: the playlist's
        // slot becomes a side-by-side split, the spectrum's old row goes.
        let mut layout = classic();
        layout.drop_panel(&[2], &[0], Drop::Beside(Side::Left));
        let placed = layout.place(1000.0, 700.0);
        assert_eq!(
            slot_kinds(&placed),
            [PanelKind::Spectrum, PanelKind::Playlist, PanelKind::Waveform, PanelKind::Transport, PanelKind::StatusBar]
        );
        assert_eq!(placed.slots[0].rect.x, 0.0);
        assert_eq!(placed.slots[1].rect.x, 500.0);
        assert_eq!(placed.slots[0].rect.y, placed.slots[1].rect.y);

        // Dropping along the stack's own direction inserts a sibling
        // instead of nesting: status bar to the top of the playlist.
        let mut layout = classic();
        layout.drop_panel(&[4], &[0], Drop::Beside(Side::Top));
        assert_eq!(
            kinds_in_order(&layout),
            [PanelKind::StatusBar, PanelKind::Playlist, PanelKind::Waveform, PanelKind::Spectrum, PanelKind::Transport]
        );
        let Node::Split { children, .. } = &layout.root else { panic!() };
        assert_eq!(children.len(), 5, "still one flat stack");
        assert_eq!(children[0].size, Size::Fixed(22.0));

        // Moving a panel out of a two-way split collapses that split: put
        // the spectrum right of the playlist, then below the transport
        // (now at index 2, one row having been folded into the split).
        let mut layout = classic();
        layout.drop_panel(&[2], &[0], Drop::Beside(Side::Right));
        layout.drop_panel(&[0, 1], &[2], Drop::Beside(Side::Bottom));
        let fixed = |px, kind| Child { size: Size::Fixed(px), node: Node::Panel(kind) };
        assert_eq!(
            layout.root,
            Node::Split {
                axis: Axis::Vertical,
                children: vec![
                    Child { size: Size::Weight(1.0), node: Node::Panel(PanelKind::Playlist) },
                    fixed(80.0, PanelKind::Waveform),
                    fixed(32.0, PanelKind::Transport),
                    fixed(150.0, PanelKind::Spectrum),
                    fixed(22.0, PanelKind::StatusBar),
                ],
            }
        );
    }

    #[test]
    fn drop_zones_follow_the_pointer() {
        let rect = Rect { x: 100.0, y: 0.0, width: 400.0, height: 200.0 };
        assert_eq!(drop_at(&rect, 300.0, 100.0), Drop::Swap);
        assert_eq!(drop_at(&rect, 110.0, 100.0), Drop::Beside(Side::Left));
        assert_eq!(drop_at(&rect, 490.0, 100.0), Drop::Beside(Side::Right));
        assert_eq!(drop_at(&rect, 300.0, 5.0), Drop::Beside(Side::Top));
        assert_eq!(drop_at(&rect, 300.0, 195.0), Drop::Beside(Side::Bottom));
        assert_eq!(
            drop_preview(&rect, Drop::Beside(Side::Right)),
            Rect { x: 300.0, y: 0.0, width: 200.0, height: 200.0 }
        );
    }

    #[test]
    fn layouts_round_trip_through_json_and_reject_garbage() {
        let mut layout = classic();
        layout.split(&[0], Axis::Horizontal);
        layout.replace(&[0, 1], PanelKind::Waveform);
        assert_eq!(Layout::from_json(&layout.to_json()), Some(layout));
        assert_eq!(Layout::from_json("{\"root\": 42}"), None);
        assert_eq!(PanelKind::from_id("status-bar"), Some(PanelKind::StatusBar));
        assert_eq!(PanelKind::from_id("nope"), None);
    }
}
