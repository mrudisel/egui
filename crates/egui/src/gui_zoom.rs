//! Helpers for zooming the whole GUI of an app (changing [`Context::pixels_per_point`].
//!
use crate::*;

/// The suggested keyboard shortcuts for global gui zooming.
pub mod kb_shortcuts {
    use super::*;

    pub const ZOOM_IN: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND, Key::PlusEquals);
    pub const ZOOM_OUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
    pub const ZOOM_RESET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);
}

/// Let the user scale the GUI (change [`Context::zoom_factor`]) by pressing
/// Cmd+Plus, Cmd+Minus or Cmd+0, just like in a browser.
///
/// By default, [`crate::Context`] calls this function at the end of each frame,
/// controllable by [`crate::Options::zoom_with_keyboard`].
pub(crate) fn zoom_with_keyboard(ctx: &Context) {
    if ctx.input_mut(|i| i.consume_shortcut(&kb_shortcuts::ZOOM_RESET)) {
        ctx.set_zoom_factor(1.0);
    } else {
        if ctx.input_mut(|i| i.consume_shortcut(&kb_shortcuts::ZOOM_IN)) {
            zoom_in(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&kb_shortcuts::ZOOM_OUT)) {
            zoom_out(ctx);
        }
    }
}

const MIN_ZOOM_FACTOR: f32 = 0.2;
const MAX_ZOOM_FACTOR: f32 = 5.0;

/// Make everything larger by increasing [`Context::zoom_factor`].
pub fn zoom_in(ctx: &Context) {
    let mut zoom_factor = ctx.zoom_factor();
    zoom_factor += 0.1;
    zoom_factor = zoom_factor.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
    zoom_factor = (zoom_factor * 10.).round() / 10.;
    ctx.set_zoom_factor(zoom_factor);
}

/// Make everything smaller by decreasing [`Context::zoom_factor`].
pub fn zoom_out(ctx: &Context) {
    let mut zoom_factor = ctx.zoom_factor();
    zoom_factor -= 0.1;
    zoom_factor = zoom_factor.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
    zoom_factor = (zoom_factor * 10.).round() / 10.;
    ctx.set_zoom_factor(zoom_factor);
}

/// Show buttons for zooming the ui.
///
/// This is meant to be called from within a menu (See [`Ui::menu_button`]).
pub fn zoom_menu_buttons(ui: &mut Ui) {
    if ui
        .add_enabled(
            ui.ctx().zoom_factor() < MAX_ZOOM_FACTOR,
            Button::new("Zoom In").shortcut_text(ui.ctx().format_shortcut(&kb_shortcuts::ZOOM_IN)),
        )
        .clicked()
    {
        zoom_in(ui.ctx());
        ui.close_menu();
    }

    if ui
        .add_enabled(
            ui.ctx().zoom_factor() > MIN_ZOOM_FACTOR,
            Button::new("Zoom Out")
                .shortcut_text(ui.ctx().format_shortcut(&kb_shortcuts::ZOOM_OUT)),
        )
        .clicked()
    {
        zoom_out(ui.ctx());
        ui.close_menu();
    }

    if ui
        .add_enabled(
            ui.ctx().zoom_factor() != 1.0,
            Button::new("Reset Zoom")
                .shortcut_text(ui.ctx().format_shortcut(&kb_shortcuts::ZOOM_RESET)),
        )
        .clicked()
    {
        ui.ctx().set_zoom_factor(1.0);
        ui.close_menu();
    }
}
