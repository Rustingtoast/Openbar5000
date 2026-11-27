//! The colour picker window (`ColorPickerWindow`): any colour, by eye or by
//! hex code, for whatever opened it -- one at a time; opening it again
//! for something else drops the first request.

use crate::theme::parse_hex_color;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use ui::ColorPickerWindow;

/// What a request does with the colour picked.
type OnPick = Box<dyn Fn(slint::Color)>;

pub struct ColorPicker {
    window: ColorPickerWindow,
    /// What to do with the colour picked, for the request open.
    on_pick: RefCell<Option<OnPick>>,
}

impl ColorPicker {
    pub fn new() -> Rc<Self> {
        let picker = Rc::new(Self {
            window: ColorPickerWindow::new().expect("failed to create the colour picker window"),
            on_pick: RefCell::new(None),
        });

        let window = picker.window.as_weak();
        picker.window.on_hex_edited(move |text| {
            if let (Some(window), Some(color)) = (window.upgrade(), parse_hex_color(&text)) {
                window.invoke_take_color(color);
            }
        });
        let weak = Rc::downgrade(&picker);
        picker.window.on_accepted(move |color| {
            if let Some(picker) = weak.upgrade() {
                let on_pick = picker.on_pick.borrow_mut().take();
                picker.close();
                if let Some(on_pick) = on_pick {
                    on_pick(color);
                }
            }
        });
        let weak = Rc::downgrade(&picker);
        picker.window.on_cancelled(move || {
            if let Some(picker) = weak.upgrade() {
                picker.on_pick.borrow_mut().take();
                picker.close();
            }
        });
        picker
    }

    /// For the theme (see `theme::ThemedWindows`).
    pub fn window(&self) -> &ColorPickerWindow {
        &self.window
    }

    /// Opens the picker on `color`, titled `subject`; OK hands the colour
    /// picked to `on_pick`.
    pub fn open(
        &self,
        subject: &str,
        color: slint::Color,
        on_pick: impl Fn(slint::Color) + 'static,
    ) {
        *self.on_pick.borrow_mut() = Some(Box::new(on_pick));
        self.window.set_subject(subject.into());
        self.window.invoke_open_with(color);
        crate::platform::show_in_front(self.window.window());
    }

    /// Closes it, without picking, if it's open (its owner closing).
    pub fn close(&self) {
        self.on_pick.borrow_mut().take();
        let _ = self.window.hide();
    }
}
