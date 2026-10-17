//! Preferences > Output: which device plays the audio. The default follows
//! the system's default output (and its changes); picking a device sticks
//! to it, falling back to the default while it's unplugged. The switching
//! itself happens in the engine (`EngineCommand::SetOutputDevice`).

use crate::settings::Settings;
use crate::store::Store;
use engine::{EngineCommand, OutputDeviceInfo, PlaybackEngine};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use ui::SettingsWindow;

/// What the dropdown offers, in order: the system default first.
#[derive(Debug, Clone, PartialEq)]
enum Choice {
    SystemDefault,
    Device(OutputDeviceInfo),
    /// The saved device, currently not connected.
    Missing { id: String, name: String },
}

/// The dropdown's entries for `devices`, with the saved choice
/// (`selected`) kept even when it isn't connected, and which one is
/// selected.
fn choices(
    devices: Vec<OutputDeviceInfo>,
    default_name: Option<&str>,
    selected: Option<(&str, &str)>,
) -> (Vec<Choice>, Vec<String>, usize) {
    let mut choices = vec![Choice::SystemDefault];
    choices.extend(devices.into_iter().map(Choice::Device));
    let index = match selected {
        None => 0,
        Some((id, name)) => match choices
            .iter()
            .position(|c| matches!(c, Choice::Device(d) if d.id == id))
        {
            Some(index) => index,
            None => {
                choices.push(Choice::Missing { id: id.to_string(), name: name.to_string() });
                choices.len() - 1
            }
        },
    };
    let labels = choices
        .iter()
        .map(|choice| match choice {
            Choice::SystemDefault => match default_name {
                Some(name) => format!("System default ({name})"),
                None => "System default".to_string(),
            },
            Choice::Device(device) => device.name.clone(),
            Choice::Missing { name, .. } => format!("{name} (not connected)"),
        })
        .collect();
    (choices, labels, index)
}

pub struct AudioOutput {
    preferences: slint::Weak<SettingsWindow>,
    engine: Rc<PlaybackEngine>,
    store: Rc<Store>,
    settings: Rc<RefCell<Settings>>,
    choices: RefCell<Vec<Choice>>,
}

impl AudioOutput {
    pub fn wire(
        preferences: &SettingsWindow,
        engine: Rc<PlaybackEngine>,
        store: Rc<Store>,
        settings: Rc<RefCell<Settings>>,
    ) -> Rc<Self> {
        let output = Rc::new(Self {
            preferences: preferences.as_weak(),
            engine,
            store,
            settings,
            choices: RefCell::new(Vec::new()),
        });
        let saved = output.settings.borrow().output_device.clone();
        output.engine.send(EngineCommand::SetOutputDevice(saved));
        output.refresh();

        let weak = Rc::downgrade(&output);
        preferences.on_output_device_chosen(move |index| {
            if let Some(output) = weak.upgrade() {
                output.choose(index);
            }
        });
        let weak = Rc::downgrade(&output);
        preferences.on_output_refresh_clicked(move || {
            if let Some(output) = weak.upgrade() {
                output.refresh();
            }
        });
        output
    }

    /// Re-lists the devices (also done whenever Preferences opens, so
    /// newly plugged-in ones show up).
    pub fn refresh(&self) {
        let settings = self.settings.borrow();
        let selected = settings
            .output_device
            .as_deref()
            .map(|id| (id, settings.output_device_name.as_str()));
        let default_name = engine::default_output_device().map(|d| d.name);
        let (choices, labels, index) = choices(engine::output_devices(), default_name.as_deref(), selected);
        drop(settings);
        *self.choices.borrow_mut() = choices;
        if let Some(preferences) = self.preferences.upgrade() {
            preferences.set_output_devices(ModelRc::from(Rc::new(VecModel::from(
                labels.into_iter().map(SharedString::from).collect::<Vec<_>>(),
            ))));
            preferences.set_output_device_index(index as i32);
        }
    }

    fn choose(&self, index: i32) {
        let Some(choice) = usize::try_from(index).ok().and_then(|i| self.choices.borrow().get(i).cloned())
        else {
            return;
        };
        let (id, name) = match choice {
            Choice::SystemDefault => (None, String::new()),
            Choice::Device(device) => (Some(device.id), device.name),
            // Already the saved choice.
            Choice::Missing { .. } => return,
        };
        {
            let mut settings = self.settings.borrow_mut();
            if settings.output_device == id {
                return;
            }
            settings.output_device = id.clone();
            settings.output_device_name = name;
            settings.save(&self.store);
        }
        self.engine.send(EngineCommand::SetOutputDevice(id));
        self.refresh();
    }

    /// The engine moved playback to another device.
    pub fn device_changed(&self, name: &str, fallback: bool) {
        let status = if fallback {
            format!(
                "Playing through {name}: {} isn\u{2019}t connected.",
                self.settings.borrow().output_device_name
            )
        } else {
            format!("Playing through {name}.")
        };
        if let Some(preferences) = self.preferences.upgrade() {
            preferences.set_output_status(status.into());
        }
        // The default's name in the list may be out of date.
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str) -> OutputDeviceInfo {
        OutputDeviceInfo { id: id.into(), name: name.into() }
    }

    #[test]
    fn the_system_default_comes_first_and_names_the_current_device() {
        let (_, labels, index) = choices(
            vec![device("a", "Speakers"), device("b", "Headphones")],
            Some("Speakers"),
            None,
        );
        assert_eq!(labels, ["System default (Speakers)", "Speakers", "Headphones"]);
        assert_eq!(index, 0);
    }

    #[test]
    fn a_chosen_device_is_selected_and_kept_while_unplugged() {
        let devices = vec![device("a", "Speakers"), device("b", "Headphones")];
        let (_, _, index) = choices(devices.clone(), Some("Speakers"), Some(("b", "Headphones")));
        assert_eq!(index, 2);

        let (choices, labels, index) = choices(vec![device("a", "Speakers")], None, Some(("b", "Headphones")));
        assert_eq!(labels.last().unwrap(), "Headphones (not connected)");
        assert_eq!(index, 2);
        assert!(matches!(&choices[2], Choice::Missing { id, .. } if id == "b"));
    }
}
