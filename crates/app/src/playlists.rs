//! Several named playlists (issue #33): the one shown is the "active" one,
//! and every change to it is saved under its id (see `wiring`, which saves
//! each `PlaylistUpdated` to the playlist it names). Switching never
//! interrupts playback -- the song playing carries on, and the next one
//! comes from the new playlist. File > New Playlist, the summary bar's
//! menu and the Playlist Manager window all drive this.

use crate::settings::Settings;
use crate::store::{PlaylistInfo, Store};
use engine::{EngineCommand, PlaybackEngine};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use ui::{MainWindow, PlaylistChoice, PlaylistManagerRow, PlaylistManagerWindow};

/// `base`, or `base 2`, `base 3`... -- the first not in `taken`.
pub fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.contains(name))
        .expect("an unused number exists")
}

pub struct Playlists {
    main: slint::Weak<MainWindow>,
    manager: PlaylistManagerWindow,
    engine: Rc<PlaybackEngine>,
    store: Rc<Store>,
    settings: Rc<RefCell<Settings>>,
    list: RefCell<Vec<PlaylistInfo>>,
    active: Cell<i64>,
    /// A cut or copied playlist: name and paths, for Paste.
    clipboard: RefCell<Option<(String, Vec<PathBuf>)>>,
}

impl Playlists {
    /// Shows the saved active playlist (restoring it at startup); returns
    /// how many tracks it has, for the loading indicator.
    pub fn wire(
        main: &MainWindow,
        manager: &PlaylistManagerWindow,
        engine: Rc<PlaybackEngine>,
        store: Rc<Store>,
        settings: Rc<RefCell<Settings>>,
    ) -> (Rc<Self>, usize) {
        let list = store.playlists();
        let saved = settings.borrow().active_playlist;
        let active = list.iter().find(|p| p.id == saved).unwrap_or(&list[0]).id;
        let playlists = Rc::new(Self {
            main: main.as_weak(),
            manager: manager.clone_strong(),
            engine,
            store,
            settings,
            list: RefCell::new(list),
            active: Cell::new(active),
            clipboard: RefCell::new(None),
        });
        let paths = playlists.store.playlist_paths(active, false);
        let count = paths.len();
        playlists.engine.send(EngineCommand::SwitchPlaylist { id: active, paths });
        playlists.refresh();

        macro_rules! on {
            ($window:ident . $callback:ident, |$p:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(&playlists);
                $window.$callback(move |$($arg),*| {
                    if let Some($p) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(main.on_playlist_chosen, |p, index| p.activate_index(index));
        on!(main.on_new_playlist_clicked, |p| p.new_playlist());
        on!(main.on_playlist_manager_clicked, |p| p.open_manager());
        on!(manager.on_activate, |p, index| p.activate_index(index));
        on!(manager.on_rename, |p, index, name| p.rename(index, &name));
        on!(manager.on_remove, |p, index| p.remove(index));
        on!(manager.on_create_new, |p| p.new_playlist());
        on!(manager.on_cut, |p, index| {
            if p.copy(index) {
                p.remove(index);
            }
        });
        on!(manager.on_copy, |p, index| {
            p.copy(index);
        });
        on!(manager.on_paste, |p| p.paste());
        (playlists, count)
    }

    /// Re-reads the list (names, counts) into the menu and the manager.
    pub fn refresh(&self) {
        let list = self.store.playlists();
        let active = self.active.get();
        if let Some(main) = self.main.upgrade() {
            // A fresh model each time: activating a checkable menu item
            // toggles its own check mark (see `show_sort_choices`).
            main.set_playlist_choices(ModelRc::from(Rc::new(VecModel::from(
                list.iter()
                    .map(|p| PlaylistChoice { title: p.name.as_str().into(), checked: p.id == active })
                    .collect::<Vec<_>>(),
            ))));
            let name = list.iter().find(|p| p.id == active).map(|p| p.name.clone()).unwrap_or_default();
            main.set_playlist_name(name.into());
        }
        self.manager.set_rows(ModelRc::from(Rc::new(VecModel::from(
            list.iter()
                .map(|p| PlaylistManagerRow {
                    name: p.name.as_str().into(),
                    count: p.count.to_string().into(),
                    active: p.id == active,
                })
                .collect::<Vec<_>>(),
        ))));
        self.manager.set_can_paste(self.clipboard.borrow().is_some());
        if self.manager.get_selected() >= list.len() as i32 {
            self.manager.set_selected(list.len() as i32 - 1);
        }
        *self.list.borrow_mut() = list;
    }

    fn id_at(&self, index: i32) -> Option<i64> {
        usize::try_from(index).ok().and_then(|i| self.list.borrow().get(i).map(|p| p.id))
    }

    fn activate_index(&self, index: i32) {
        if let Some(id) = self.id_at(index) {
            self.activate(id);
        }
    }

    pub fn activate(&self, id: i64) {
        if id == self.active.get() {
            return;
        }
        self.active.set(id);
        self.settings.borrow_mut().active_playlist = id;
        self.settings.borrow().save(&self.store);
        let paths = self.store.playlist_paths(id, false);
        self.engine.send(EngineCommand::SwitchPlaylist { id, paths });
        self.refresh();
    }

    fn names(&self) -> Vec<String> {
        self.list.borrow().iter().map(|p| p.name.clone()).collect()
    }

    /// An empty playlist, shown at once -- Open Files/Folder then fill it.
    pub fn new_playlist(&self) {
        let name = unique_name("New Playlist", &self.names());
        let id = self.store.create_playlist(&name, &[]);
        self.refresh();
        self.activate(id);
        let index = self.list.borrow().iter().position(|p| p.id == id);
        if let Some(index) = index {
            self.manager.set_selected(index as i32);
        }
    }

    fn rename(&self, index: i32, name: &str) {
        let name = name.trim();
        if let (Some(id), false) = (self.id_at(index), name.is_empty()) {
            self.store.rename_playlist(id, name);
            self.refresh();
        }
    }

    /// Deletes a playlist -- never the last one. Removing the active one
    /// shows its neighbour first.
    fn remove(&self, index: i32) {
        let Some(id) = self.id_at(index) else { return };
        let neighbour = {
            let list = self.list.borrow();
            if list.len() < 2 {
                return;
            }
            let i = index as usize;
            list.get(i + 1).or_else(|| list.get(i.wrapping_sub(1))).map(|p| p.id)
        };
        if id == self.active.get() {
            if let Some(neighbour) = neighbour {
                self.activate(neighbour);
            }
        }
        self.store.delete_playlist(id);
        self.refresh();
    }

    /// Puts the playlist on the internal clipboard; `false` if there's none.
    fn copy(&self, index: i32) -> bool {
        let Some(info) = usize::try_from(index).ok().and_then(|i| self.list.borrow().get(i).cloned()) else {
            return false;
        };
        let paths = self.store.playlist_paths(info.id, true);
        *self.clipboard.borrow_mut() = Some((info.name, paths));
        self.manager.set_can_paste(true);
        true
    }

    fn paste(&self) {
        let Some((name, paths)) = self.clipboard.borrow().clone() else { return };
        let name = unique_name(&name, &self.names());
        self.store.create_playlist(&name, &paths);
        self.refresh();
    }

    fn open_manager(&self) {
        self.refresh();
        let active = self.list.borrow().iter().position(|p| p.id == self.active.get());
        self.manager.set_selected(active.map_or(-1, |i| i as i32));
        self.manager.set_renaming(-1);
        let _ = self.manager.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_names_count_up_past_taken_ones() {
        let taken = vec!["New Playlist".to_string(), "New Playlist 2".to_string(), "Chill".to_string()];
        assert_eq!(unique_name("New Playlist", &taken), "New Playlist 3");
        assert_eq!(unique_name("Trance", &taken), "Trance");
        assert_eq!(unique_name("Chill", &taken), "Chill 2");
    }
}
