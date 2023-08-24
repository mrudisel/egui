//! [`egui`] bindings for [`winit`](https://github.com/rust-windowing/winit).
//!
//! The library translates winit events to egui, handled copy/paste,
//! updates the cursor, open links clicked in egui, etc.
//!
//! ## Feature flags
#![cfg_attr(feature = "document-features", doc = document_features::document_features!())]
//!

#![allow(clippy::manual_range_contains)]

use std::sync::Arc;

#[cfg(feature = "accesskit")]
pub use accesskit_winit;
pub use egui;
#[cfg(feature = "accesskit")]
use egui::accesskit;
use egui::{mutex::RwLock, Pos2, ViewportBuilder, ViewportCommand, ViewportId};
pub use winit;

pub mod clipboard;
mod window_settings;

pub use window_settings::WindowSettings;

use raw_window_handle::HasRawDisplayHandle;

pub fn native_pixels_per_point(window: &winit::window::Window) -> f32 {
    window.scale_factor() as f32
}

pub fn screen_size_in_pixels(window: &winit::window::Window) -> egui::Vec2 {
    let size = window.inner_size();
    egui::vec2(size.width as f32, size.height as f32)
}

// ----------------------------------------------------------------------------

#[must_use]
pub struct EventResponse {
    /// If true, egui consumed this event, i.e. wants exclusive use of this event
    /// (e.g. a mouse click on an egui window, or entering text into a text field).
    ///
    /// For instance, if you use egui for a game, you should only
    /// pass on the events to your game when [`Self::consumed`] is `false.
    ///
    /// Note that egui uses `tab` to move focus between elements, so this will always be `true` for tabs.
    pub consumed: bool,

    /// Do we need an egui refresh because of this event?
    pub repaint: bool,
}

// ----------------------------------------------------------------------------

/// Handles the integration between egui and winit.
pub struct State {
    start_time: instant::Instant,
    egui_input: egui::RawInput,
    pointer_pos_in_points: Option<egui::Pos2>,
    any_pointer_button_down: bool,
    current_cursor_icon: Option<egui::CursorIcon>,

    /// What egui uses.
    current_pixels_per_point: f32,

    clipboard: clipboard::Clipboard,

    /// If `true`, mouse inputs will be treated as touches.
    /// Useful for debugging touch support in egui.
    ///
    /// Creates duplicate touches, if real touch inputs are coming.
    simulate_touch_screen: bool,

    /// Is Some(…) when a touch is being translated to a pointer.
    ///
    /// Only one touch will be interpreted as pointer at any time.
    pointer_touch_id: Option<u64>,

    /// track ime state
    input_method_editor_started: bool,

    #[cfg(feature = "accesskit")]
    accesskit: Option<accesskit_winit::Adapter>,
}

impl State {
    /// Construct a new instance
    ///
    /// # Safety
    ///
    /// The returned `State` must not outlive the input `display_target`.
    pub fn new(display_target: &dyn HasRawDisplayHandle) -> Self {
        let egui_input = egui::RawInput {
            focused: false, // winit will tell us when we have focus
            ..Default::default()
        };

        Self {
            start_time: instant::Instant::now(),
            egui_input,
            pointer_pos_in_points: None,
            any_pointer_button_down: false,
            current_cursor_icon: None,
            current_pixels_per_point: 1.0,

            clipboard: clipboard::Clipboard::new(display_target),

            simulate_touch_screen: false,
            pointer_touch_id: None,

            input_method_editor_started: false,

            #[cfg(feature = "accesskit")]
            accesskit: None,
        }
    }

    #[cfg(feature = "accesskit")]
    pub fn init_accesskit<T: From<accesskit_winit::ActionRequestEvent> + Send>(
        &mut self,
        window: &winit::window::Window,
        event_loop_proxy: winit::event_loop::EventLoopProxy<T>,
        initial_tree_update_factory: impl 'static + FnOnce() -> accesskit::TreeUpdate + Send,
    ) {
        self.accesskit = Some(accesskit_winit::Adapter::new(
            window,
            initial_tree_update_factory,
            event_loop_proxy,
        ));
    }

    /// Call this once a graphics context has been created to update the maximum texture dimensions
    /// that egui will use.
    pub fn set_max_texture_side(&mut self, max_texture_side: usize) {
        self.egui_input.max_texture_side = Some(max_texture_side);
    }

    /// Call this when a new native Window is created for rendering to initialize the `pixels_per_point`
    /// for that window.
    ///
    /// In particular, on Android it is necessary to call this after each `Resumed` lifecycle
    /// event, each time a new native window is created.
    ///
    /// Once this has been initialized for a new window then this state will be maintained by handling
    /// [`winit::event::WindowEvent::ScaleFactorChanged`] events.
    pub fn set_pixels_per_point(&mut self, pixels_per_point: f32) {
        self.egui_input.pixels_per_point = Some(pixels_per_point);
        self.current_pixels_per_point = pixels_per_point;
    }

    /// The number of physical pixels per logical point,
    /// as configured on the current egui context (see [`egui::Context::pixels_per_point`]).
    #[inline]
    pub fn pixels_per_point(&self) -> f32 {
        self.current_pixels_per_point
    }

    /// The current input state.
    /// This is changed by [`Self::on_event`] and cleared by [`Self::take_egui_input`].
    #[inline]
    pub fn egui_input(&self) -> &egui::RawInput {
        &self.egui_input
    }

    /// Prepare for a new frame by extracting the accumulated input,
    /// as well as setting [the time](egui::RawInput::time) and [screen rectangle](egui::RawInput::screen_rect).
    pub fn take_egui_input(&mut self, window: &winit::window::Window) -> egui::RawInput {
        let pixels_per_point = self.pixels_per_point();

        self.egui_input.time = Some(self.start_time.elapsed().as_secs_f64());

        // On Windows, a minimized window will have 0 width and height.
        // See: https://github.com/rust-windowing/winit/issues/208
        // This solves an issue where egui window positions would be changed when minimizing on Windows.
        let screen_size_in_pixels = screen_size_in_pixels(window);
        let screen_size_in_points = screen_size_in_pixels / pixels_per_point;
        self.egui_input.screen_rect = if !window.is_minimized().unwrap_or_else(|| {
            eprintln!("Cannot determine the Viewport/native window minimized state");
            true
        }) {
            Some(egui::Rect::from_min_max(
                window
                    .outer_position()
                    .map(|pos| Pos2::new(pos.x as f32, pos.y as f32))
                    .unwrap_or(Pos2::ZERO),
                screen_size_in_points.to_pos2(),
            ))
        } else {
            None
        };

        self.egui_input.take()
    }

    /// Call this when there is a new event.
    ///
    /// The result can be found in [`Self::egui_input`] and be extracted with [`Self::take_egui_input`].
    pub fn on_event(
        &mut self,
        egui_ctx: &egui::Context,
        event: &winit::event::WindowEvent<'_>,
    ) -> EventResponse {
        use winit::event::WindowEvent;
        match event {
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let pixels_per_point = *scale_factor as f32;
                self.egui_input.pixels_per_point = Some(pixels_per_point);
                self.current_pixels_per_point = pixels_per_point;
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse_button_input(*state, *button);
                EventResponse {
                    repaint: true,
                    consumed: egui_ctx.wants_pointer_input(),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.on_mouse_wheel(*delta);
                EventResponse {
                    repaint: true,
                    consumed: egui_ctx.wants_pointer_input(),
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(*position);
                EventResponse {
                    repaint: true,
                    consumed: egui_ctx.is_using_pointer(),
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer_pos_in_points = None;
                self.egui_input.events.push(egui::Event::PointerGone);
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            // WindowEvent::TouchpadPressure {device_id, pressure, stage, ..  } => {} // TODO
            WindowEvent::Touch(touch) => {
                self.on_touch(touch);
                let consumed = match touch.phase {
                    winit::event::TouchPhase::Started
                    | winit::event::TouchPhase::Ended
                    | winit::event::TouchPhase::Cancelled => egui_ctx.wants_pointer_input(),
                    winit::event::TouchPhase::Moved => egui_ctx.is_using_pointer(),
                };
                EventResponse {
                    repaint: true,
                    consumed,
                }
            }
            WindowEvent::ReceivedCharacter(ch) => {
                // On Mac we get here when the user presses Cmd-C (copy), ctrl-W, etc.
                // We need to ignore these characters that are side-effects of commands.
                let is_mac_cmd = cfg!(target_os = "macos")
                    && (self.egui_input.modifiers.ctrl || self.egui_input.modifiers.mac_cmd);

                let consumed = if is_printable_char(*ch) && !is_mac_cmd {
                    self.egui_input
                        .events
                        .push(egui::Event::Text(ch.to_string()));
                    egui_ctx.wants_keyboard_input()
                } else {
                    false
                };
                EventResponse {
                    repaint: true,
                    consumed,
                }
            }
            WindowEvent::Ime(ime) => {
                // on Mac even Cmd-C is pressed during ime, a `c` is pushed to Preedit.
                // So no need to check is_mac_cmd.
                //
                // How winit produce `Ime::Enabled` and `Ime::Disabled` differs in MacOS
                // and Windows.
                //
                // - On Windows, before and after each Commit will produce an Enable/Disabled
                // event.
                // - On MacOS, only when user explicit enable/disable ime. No Disabled
                // after Commit.
                //
                // We use input_method_editor_started to manually insert CompositionStart
                // between Commits.
                match ime {
                    winit::event::Ime::Enabled | winit::event::Ime::Disabled => (),
                    winit::event::Ime::Commit(text) => {
                        self.input_method_editor_started = false;
                        self.egui_input
                            .events
                            .push(egui::Event::CompositionEnd(text.clone()));
                    }
                    winit::event::Ime::Preedit(text, ..) => {
                        if !self.input_method_editor_started {
                            self.input_method_editor_started = true;
                            self.egui_input.events.push(egui::Event::CompositionStart);
                        }
                        self.egui_input
                            .events
                            .push(egui::Event::CompositionUpdate(text.clone()));
                    }
                };

                EventResponse {
                    repaint: true,
                    consumed: egui_ctx.wants_keyboard_input(),
                }
            }
            WindowEvent::KeyboardInput { input, .. } => {
                self.on_keyboard_input(input);
                // When pressing the Tab key, egui focuses the first focusable element, hence Tab always consumes.
                let consumed = egui_ctx.wants_keyboard_input()
                    || input.virtual_keycode == Some(winit::event::VirtualKeyCode::Tab);
                EventResponse {
                    repaint: true,
                    consumed,
                }
            }
            WindowEvent::Focused(focused) => {
                self.egui_input.focused = *focused;
                // We will not be given a KeyboardInput event when the modifiers are released while
                // the window does not have focus. Unset all modifier state to be safe.
                self.egui_input.modifiers = egui::Modifiers::default();
                self.egui_input
                    .events
                    .push(egui::Event::WindowFocused(*focused));
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::HoveredFile(path) => {
                self.egui_input.hovered_files.push(egui::HoveredFile {
                    path: Some(path.clone()),
                    ..Default::default()
                });
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::HoveredFileCancelled => {
                self.egui_input.hovered_files.clear();
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.egui_input.hovered_files.clear();
                self.egui_input.dropped_files.push(egui::DroppedFile {
                    path: Some(path.clone()),
                    ..Default::default()
                });
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }
            WindowEvent::ModifiersChanged(state) => {
                self.egui_input.modifiers.alt = state.alt();
                self.egui_input.modifiers.ctrl = state.ctrl();
                self.egui_input.modifiers.shift = state.shift();
                self.egui_input.modifiers.mac_cmd = cfg!(target_os = "macos") && state.logo();
                self.egui_input.modifiers.command = if cfg!(target_os = "macos") {
                    state.logo()
                } else {
                    state.ctrl()
                };
                EventResponse {
                    repaint: true,
                    consumed: false,
                }
            }

            // Things that may require repaint:
            WindowEvent::CloseRequested => {
                self.egui_input
                    .events
                    .push(egui::Event::WindowEvent(egui::WindowEvent::CloseRequested));
                EventResponse {
                    consumed: true,
                    repaint: true,
                }
            }
            WindowEvent::CursorEntered { .. }
            | WindowEvent::Destroyed
            | WindowEvent::Occluded(_)
            | WindowEvent::Resized(_)
            | WindowEvent::Moved(_)
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::TouchpadPressure { .. } => EventResponse {
                repaint: true,
                consumed: false,
            },

            // Things we completely ignore:
            WindowEvent::AxisMotion { .. }
            | WindowEvent::SmartMagnify { .. }
            | WindowEvent::TouchpadRotate { .. } => EventResponse {
                repaint: false,
                consumed: false,
            },

            WindowEvent::TouchpadMagnify { delta, .. } => {
                // Positive delta values indicate magnification (zooming in).
                // Negative delta values indicate shrinking (zooming out).
                let zoom_factor = (*delta as f32).exp();
                self.egui_input.events.push(egui::Event::Zoom(zoom_factor));
                EventResponse {
                    repaint: true,
                    consumed: egui_ctx.wants_pointer_input(),
                }
            }
        }
    }

    /// Call this when there is a new [`accesskit::ActionRequest`].
    ///
    /// The result can be found in [`Self::egui_input`] and be extracted with [`Self::take_egui_input`].
    #[cfg(feature = "accesskit")]
    pub fn on_accesskit_action_request(&mut self, request: accesskit::ActionRequest) {
        self.egui_input
            .events
            .push(egui::Event::AccessKitActionRequest(request));
    }

    fn on_mouse_button_input(
        &mut self,
        state: winit::event::ElementState,
        button: winit::event::MouseButton,
    ) {
        if let Some(pos) = self.pointer_pos_in_points {
            if let Some(button) = translate_mouse_button(button) {
                let pressed = state == winit::event::ElementState::Pressed;

                self.egui_input.events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: self.egui_input.modifiers,
                });

                if self.simulate_touch_screen {
                    if pressed {
                        self.any_pointer_button_down = true;

                        self.egui_input.events.push(egui::Event::Touch {
                            device_id: egui::TouchDeviceId(0),
                            id: egui::TouchId(0),
                            phase: egui::TouchPhase::Start,
                            pos,
                            force: None,
                        });
                    } else {
                        self.any_pointer_button_down = false;

                        self.egui_input.events.push(egui::Event::PointerGone);

                        self.egui_input.events.push(egui::Event::Touch {
                            device_id: egui::TouchDeviceId(0),
                            id: egui::TouchId(0),
                            phase: egui::TouchPhase::End,
                            pos,
                            force: None,
                        });
                    };
                }
            }
        }
    }

    fn on_cursor_moved(&mut self, pos_in_pixels: winit::dpi::PhysicalPosition<f64>) {
        let pos_in_points = egui::pos2(
            pos_in_pixels.x as f32 / self.pixels_per_point(),
            pos_in_pixels.y as f32 / self.pixels_per_point(),
        );
        self.pointer_pos_in_points = Some(pos_in_points);

        if self.simulate_touch_screen {
            if self.any_pointer_button_down {
                self.egui_input
                    .events
                    .push(egui::Event::PointerMoved(pos_in_points));

                self.egui_input.events.push(egui::Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(0),
                    phase: egui::TouchPhase::Move,
                    pos: pos_in_points,
                    force: None,
                });
            }
        } else {
            self.egui_input
                .events
                .push(egui::Event::PointerMoved(pos_in_points));
        }
    }

    fn on_touch(&mut self, touch: &winit::event::Touch) {
        // Emit touch event
        self.egui_input.events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(egui::epaint::util::hash(touch.device_id)),
            id: egui::TouchId::from(touch.id),
            phase: match touch.phase {
                winit::event::TouchPhase::Started => egui::TouchPhase::Start,
                winit::event::TouchPhase::Moved => egui::TouchPhase::Move,
                winit::event::TouchPhase::Ended => egui::TouchPhase::End,
                winit::event::TouchPhase::Cancelled => egui::TouchPhase::Cancel,
            },
            pos: egui::pos2(
                touch.location.x as f32 / self.pixels_per_point(),
                touch.location.y as f32 / self.pixels_per_point(),
            ),
            force: match touch.force {
                Some(winit::event::Force::Normalized(force)) => Some(force as f32),
                Some(winit::event::Force::Calibrated {
                    force,
                    max_possible_force,
                    ..
                }) => Some((force / max_possible_force) as f32),
                None => None,
            },
        });
        // If we're not yet translating a touch or we're translating this very
        // touch …
        if self.pointer_touch_id.is_none() || self.pointer_touch_id.unwrap() == touch.id {
            // … emit PointerButton resp. PointerMoved events to emulate mouse
            match touch.phase {
                winit::event::TouchPhase::Started => {
                    self.pointer_touch_id = Some(touch.id);
                    // First move the pointer to the right location
                    self.on_cursor_moved(touch.location);
                    self.on_mouse_button_input(
                        winit::event::ElementState::Pressed,
                        winit::event::MouseButton::Left,
                    );
                }
                winit::event::TouchPhase::Moved => {
                    self.on_cursor_moved(touch.location);
                }
                winit::event::TouchPhase::Ended => {
                    self.pointer_touch_id = None;
                    self.on_mouse_button_input(
                        winit::event::ElementState::Released,
                        winit::event::MouseButton::Left,
                    );
                    // The pointer should vanish completely to not get any
                    // hover effects
                    self.pointer_pos_in_points = None;
                    self.egui_input.events.push(egui::Event::PointerGone);
                }
                winit::event::TouchPhase::Cancelled => {
                    self.pointer_touch_id = None;
                    self.pointer_pos_in_points = None;
                    self.egui_input.events.push(egui::Event::PointerGone);
                }
            }
        }
    }

    fn on_mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        {
            let (unit, delta) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    (egui::MouseWheelUnit::Line, egui::vec2(x, y))
                }
                winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition {
                    x,
                    y,
                }) => (
                    egui::MouseWheelUnit::Point,
                    egui::vec2(x as f32, y as f32) / self.pixels_per_point(),
                ),
            };
            let modifiers = self.egui_input.modifiers;
            self.egui_input.events.push(egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
            });
        }
        let delta = match delta {
            winit::event::MouseScrollDelta::LineDelta(x, y) => {
                let points_per_scroll_line = 50.0; // Scroll speed decided by consensus: https://github.com/emilk/egui/issues/461
                egui::vec2(x, y) * points_per_scroll_line
            }
            winit::event::MouseScrollDelta::PixelDelta(delta) => {
                egui::vec2(delta.x as f32, delta.y as f32) / self.pixels_per_point()
            }
        };

        if self.egui_input.modifiers.ctrl || self.egui_input.modifiers.command {
            // Treat as zoom instead:
            let factor = (delta.y / 200.0).exp();
            self.egui_input.events.push(egui::Event::Zoom(factor));
        } else if self.egui_input.modifiers.shift {
            // Treat as horizontal scrolling.
            // Note: one Mac we already get horizontal scroll events when shift is down.
            self.egui_input
                .events
                .push(egui::Event::Scroll(egui::vec2(delta.x + delta.y, 0.0)));
        } else {
            self.egui_input.events.push(egui::Event::Scroll(delta));
        }
    }

    fn on_keyboard_input(&mut self, input: &winit::event::KeyboardInput) {
        if let Some(keycode) = input.virtual_keycode {
            let pressed = input.state == winit::event::ElementState::Pressed;

            if pressed {
                // VirtualKeyCode::Paste etc in winit are broken/untrustworthy,
                // so we detect these things manually:
                if is_cut_command(self.egui_input.modifiers, keycode) {
                    self.egui_input.events.push(egui::Event::Cut);
                } else if is_copy_command(self.egui_input.modifiers, keycode) {
                    self.egui_input.events.push(egui::Event::Copy);
                } else if is_paste_command(self.egui_input.modifiers, keycode) {
                    if let Some(contents) = self.clipboard.get() {
                        let contents = contents.replace("\r\n", "\n");
                        if !contents.is_empty() {
                            self.egui_input.events.push(egui::Event::Paste(contents));
                        }
                    }
                }
            }

            if let Some(key) = translate_virtual_key_code(keycode) {
                self.egui_input.events.push(egui::Event::Key {
                    key,
                    pressed,
                    repeat: false, // egui will fill this in for us!
                    modifiers: self.egui_input.modifiers,
                });
            }
        }
    }

    /// Call with the output given by `egui`.
    ///
    /// This will, if needed:
    /// * update the cursor
    /// * copy text to the clipboard
    /// * open any clicked urls
    /// * update the IME
    /// *
    pub fn handle_platform_output(
        &mut self,
        window: &winit::window::Window,
        egui_ctx: &egui::Context,
        platform_output: egui::PlatformOutput,
    ) {
        let egui::PlatformOutput {
            cursor_icon,
            open_url,
            copied_text,
            events: _,                    // handled above
            mutable_text_under_cursor: _, // only used in eframe web
            text_cursor_pos,
            #[cfg(feature = "accesskit")]
            accesskit_update,
        } = platform_output;
        self.current_pixels_per_point = egui_ctx.pixels_per_point(); // someone can have changed it to scale the UI

        self.set_cursor_icon(window, cursor_icon);

        if let Some(open_url) = open_url {
            open_url_in_browser(&open_url.url);
        }

        if !copied_text.is_empty() {
            self.clipboard.set(copied_text);
        }

        if let Some(egui::Pos2 { x, y }) = text_cursor_pos {
            window.set_ime_position(winit::dpi::LogicalPosition { x, y });
        }

        #[cfg(feature = "accesskit")]
        if let Some(accesskit) = self.accesskit.as_ref() {
            if let Some(update) = accesskit_update {
                accesskit.update_if_active(|| update);
            }
        }
    }

    fn set_cursor_icon(&mut self, window: &winit::window::Window, cursor_icon: egui::CursorIcon) {
        if self.current_cursor_icon == Some(cursor_icon) {
            // Prevent flickering near frame boundary when Windows OS tries to control cursor icon for window resizing.
            // On other platforms: just early-out to save CPU.
            return;
        }

        let is_pointer_in_window = self.pointer_pos_in_points.is_some();
        if is_pointer_in_window {
            self.current_cursor_icon = Some(cursor_icon);

            if let Some(winit_cursor_icon) = translate_cursor(cursor_icon) {
                window.set_cursor_visible(true);
                window.set_cursor_icon(winit_cursor_icon);
            } else {
                window.set_cursor_visible(false);
            }
        } else {
            // Remember to set the cursor again once the cursor returns to the screen:
            self.current_cursor_icon = None;
        }
    }
}

fn open_url_in_browser(_url: &str) {
    #[cfg(feature = "webbrowser")]
    if let Err(err) = webbrowser::open(_url) {
        log::warn!("Failed to open url: {}", err);
    }

    #[cfg(not(feature = "webbrowser"))]
    {
        log::warn!("Cannot open url - feature \"links\" not enabled.");
    }
}

/// Winit sends special keys (backspace, delete, F1, …) as characters.
/// Ignore those.
/// We also ignore '\r', '\n', '\t'.
/// Newlines are handled by the `Key::Enter` event.
fn is_printable_char(chr: char) -> bool {
    let is_in_private_use_area = '\u{e000}' <= chr && chr <= '\u{f8ff}'
        || '\u{f0000}' <= chr && chr <= '\u{ffffd}'
        || '\u{100000}' <= chr && chr <= '\u{10fffd}';

    !is_in_private_use_area && !chr.is_ascii_control()
}

fn is_cut_command(modifiers: egui::Modifiers, keycode: winit::event::VirtualKeyCode) -> bool {
    (modifiers.command && keycode == winit::event::VirtualKeyCode::X)
        || (cfg!(target_os = "windows")
            && modifiers.shift
            && keycode == winit::event::VirtualKeyCode::Delete)
}

fn is_copy_command(modifiers: egui::Modifiers, keycode: winit::event::VirtualKeyCode) -> bool {
    (modifiers.command && keycode == winit::event::VirtualKeyCode::C)
        || (cfg!(target_os = "windows")
            && modifiers.ctrl
            && keycode == winit::event::VirtualKeyCode::Insert)
}

fn is_paste_command(modifiers: egui::Modifiers, keycode: winit::event::VirtualKeyCode) -> bool {
    (modifiers.command && keycode == winit::event::VirtualKeyCode::V)
        || (cfg!(target_os = "windows")
            && modifiers.shift
            && keycode == winit::event::VirtualKeyCode::Insert)
}

fn translate_mouse_button(button: winit::event::MouseButton) -> Option<egui::PointerButton> {
    match button {
        winit::event::MouseButton::Left => Some(egui::PointerButton::Primary),
        winit::event::MouseButton::Right => Some(egui::PointerButton::Secondary),
        winit::event::MouseButton::Middle => Some(egui::PointerButton::Middle),
        winit::event::MouseButton::Other(1) => Some(egui::PointerButton::Extra1),
        winit::event::MouseButton::Other(2) => Some(egui::PointerButton::Extra2),
        winit::event::MouseButton::Other(_) => None,
    }
}

fn translate_virtual_key_code(key: winit::event::VirtualKeyCode) -> Option<egui::Key> {
    use egui::Key;
    use winit::event::VirtualKeyCode;

    Some(match key {
        VirtualKeyCode::Down => Key::ArrowDown,
        VirtualKeyCode::Left => Key::ArrowLeft,
        VirtualKeyCode::Right => Key::ArrowRight,
        VirtualKeyCode::Up => Key::ArrowUp,

        VirtualKeyCode::Escape => Key::Escape,
        VirtualKeyCode::Tab => Key::Tab,
        VirtualKeyCode::Back => Key::Backspace,
        VirtualKeyCode::Return => Key::Enter,
        VirtualKeyCode::Space => Key::Space,

        VirtualKeyCode::Insert => Key::Insert,
        VirtualKeyCode::Delete => Key::Delete,
        VirtualKeyCode::Home => Key::Home,
        VirtualKeyCode::End => Key::End,
        VirtualKeyCode::PageUp => Key::PageUp,
        VirtualKeyCode::PageDown => Key::PageDown,

        VirtualKeyCode::Minus => Key::Minus,
        // Using Mac the key with the Plus sign on it is reported as the Equals key
        // (with both English and Swedish keyboard).
        VirtualKeyCode::Equals => Key::PlusEquals,

        VirtualKeyCode::Key0 | VirtualKeyCode::Numpad0 => Key::Num0,
        VirtualKeyCode::Key1 | VirtualKeyCode::Numpad1 => Key::Num1,
        VirtualKeyCode::Key2 | VirtualKeyCode::Numpad2 => Key::Num2,
        VirtualKeyCode::Key3 | VirtualKeyCode::Numpad3 => Key::Num3,
        VirtualKeyCode::Key4 | VirtualKeyCode::Numpad4 => Key::Num4,
        VirtualKeyCode::Key5 | VirtualKeyCode::Numpad5 => Key::Num5,
        VirtualKeyCode::Key6 | VirtualKeyCode::Numpad6 => Key::Num6,
        VirtualKeyCode::Key7 | VirtualKeyCode::Numpad7 => Key::Num7,
        VirtualKeyCode::Key8 | VirtualKeyCode::Numpad8 => Key::Num8,
        VirtualKeyCode::Key9 | VirtualKeyCode::Numpad9 => Key::Num9,

        VirtualKeyCode::A => Key::A,
        VirtualKeyCode::B => Key::B,
        VirtualKeyCode::C => Key::C,
        VirtualKeyCode::D => Key::D,
        VirtualKeyCode::E => Key::E,
        VirtualKeyCode::F => Key::F,
        VirtualKeyCode::G => Key::G,
        VirtualKeyCode::H => Key::H,
        VirtualKeyCode::I => Key::I,
        VirtualKeyCode::J => Key::J,
        VirtualKeyCode::K => Key::K,
        VirtualKeyCode::L => Key::L,
        VirtualKeyCode::M => Key::M,
        VirtualKeyCode::N => Key::N,
        VirtualKeyCode::O => Key::O,
        VirtualKeyCode::P => Key::P,
        VirtualKeyCode::Q => Key::Q,
        VirtualKeyCode::R => Key::R,
        VirtualKeyCode::S => Key::S,
        VirtualKeyCode::T => Key::T,
        VirtualKeyCode::U => Key::U,
        VirtualKeyCode::V => Key::V,
        VirtualKeyCode::W => Key::W,
        VirtualKeyCode::X => Key::X,
        VirtualKeyCode::Y => Key::Y,
        VirtualKeyCode::Z => Key::Z,

        VirtualKeyCode::F1 => Key::F1,
        VirtualKeyCode::F2 => Key::F2,
        VirtualKeyCode::F3 => Key::F3,
        VirtualKeyCode::F4 => Key::F4,
        VirtualKeyCode::F5 => Key::F5,
        VirtualKeyCode::F6 => Key::F6,
        VirtualKeyCode::F7 => Key::F7,
        VirtualKeyCode::F8 => Key::F8,
        VirtualKeyCode::F9 => Key::F9,
        VirtualKeyCode::F10 => Key::F10,
        VirtualKeyCode::F11 => Key::F11,
        VirtualKeyCode::F12 => Key::F12,
        VirtualKeyCode::F13 => Key::F13,
        VirtualKeyCode::F14 => Key::F14,
        VirtualKeyCode::F15 => Key::F15,
        VirtualKeyCode::F16 => Key::F16,
        VirtualKeyCode::F17 => Key::F17,
        VirtualKeyCode::F18 => Key::F18,
        VirtualKeyCode::F19 => Key::F19,
        VirtualKeyCode::F20 => Key::F20,

        _ => {
            return None;
        }
    })
}

fn translate_cursor(cursor_icon: egui::CursorIcon) -> Option<winit::window::CursorIcon> {
    match cursor_icon {
        egui::CursorIcon::None => None,

        egui::CursorIcon::Alias => Some(winit::window::CursorIcon::Alias),
        egui::CursorIcon::AllScroll => Some(winit::window::CursorIcon::AllScroll),
        egui::CursorIcon::Cell => Some(winit::window::CursorIcon::Cell),
        egui::CursorIcon::ContextMenu => Some(winit::window::CursorIcon::ContextMenu),
        egui::CursorIcon::Copy => Some(winit::window::CursorIcon::Copy),
        egui::CursorIcon::Crosshair => Some(winit::window::CursorIcon::Crosshair),
        egui::CursorIcon::Default => Some(winit::window::CursorIcon::Default),
        egui::CursorIcon::Grab => Some(winit::window::CursorIcon::Grab),
        egui::CursorIcon::Grabbing => Some(winit::window::CursorIcon::Grabbing),
        egui::CursorIcon::Help => Some(winit::window::CursorIcon::Help),
        egui::CursorIcon::Move => Some(winit::window::CursorIcon::Move),
        egui::CursorIcon::NoDrop => Some(winit::window::CursorIcon::NoDrop),
        egui::CursorIcon::NotAllowed => Some(winit::window::CursorIcon::NotAllowed),
        egui::CursorIcon::PointingHand => Some(winit::window::CursorIcon::Hand),
        egui::CursorIcon::Progress => Some(winit::window::CursorIcon::Progress),

        egui::CursorIcon::ResizeHorizontal => Some(winit::window::CursorIcon::EwResize),
        egui::CursorIcon::ResizeNeSw => Some(winit::window::CursorIcon::NeswResize),
        egui::CursorIcon::ResizeNwSe => Some(winit::window::CursorIcon::NwseResize),
        egui::CursorIcon::ResizeVertical => Some(winit::window::CursorIcon::NsResize),

        egui::CursorIcon::ResizeEast => Some(winit::window::CursorIcon::EResize),
        egui::CursorIcon::ResizeSouthEast => Some(winit::window::CursorIcon::SeResize),
        egui::CursorIcon::ResizeSouth => Some(winit::window::CursorIcon::SResize),
        egui::CursorIcon::ResizeSouthWest => Some(winit::window::CursorIcon::SwResize),
        egui::CursorIcon::ResizeWest => Some(winit::window::CursorIcon::WResize),
        egui::CursorIcon::ResizeNorthWest => Some(winit::window::CursorIcon::NwResize),
        egui::CursorIcon::ResizeNorth => Some(winit::window::CursorIcon::NResize),
        egui::CursorIcon::ResizeNorthEast => Some(winit::window::CursorIcon::NeResize),
        egui::CursorIcon::ResizeColumn => Some(winit::window::CursorIcon::ColResize),
        egui::CursorIcon::ResizeRow => Some(winit::window::CursorIcon::RowResize),

        egui::CursorIcon::Text => Some(winit::window::CursorIcon::Text),
        egui::CursorIcon::VerticalText => Some(winit::window::CursorIcon::VerticalText),
        egui::CursorIcon::Wait => Some(winit::window::CursorIcon::Wait),
        egui::CursorIcon::ZoomIn => Some(winit::window::CursorIcon::ZoomIn),
        egui::CursorIcon::ZoomOut => Some(winit::window::CursorIcon::ZoomOut),
    }
}

// Helpers for egui Viewports
// ---------------------------------------------------------------------------

pub fn process_viewport_commands(
    commands: Vec<ViewportCommand>,
    viewport_id: ViewportId,
    focused: Option<ViewportId>,
    window: &Arc<RwLock<winit::window::Window>>,
) {
    use winit::dpi::PhysicalSize;
    use winit::window::ResizeDirection;
    let win = window.read();

    for command in commands {
        match command {
            egui::ViewportCommand::Drag => {
                // if this is not checked on x11 the input will be permanently taken until the app is killed!
                if let Some(focus) = focused {
                    if focus == viewport_id {
                        // TODO possible return the error to `egui::Context`
                        let _ = win.drag_window();
                    }
                }
            }
            egui::ViewportCommand::InnerSize(width, height) => {
                let width = width.max(1);
                let height = height.max(1);
                win.set_inner_size(PhysicalSize::new(width, height));
            }
            egui::ViewportCommand::Resize(top, bottom, right, left) => {
                // TODO posibile return the error to `egui::Context`
                let _ = win.drag_resize_window(match (top, bottom, right, left) {
                    (true, false, false, false) => ResizeDirection::North,
                    (false, true, false, false) => ResizeDirection::South,
                    (false, false, false, true) => ResizeDirection::West,
                    (true, false, true, false) => ResizeDirection::NorthEast,
                    (false, true, true, false) => ResizeDirection::SouthEast,
                    (true, false, false, true) => ResizeDirection::NorthWest,
                    (false, true, false, true) => ResizeDirection::SouthWest,
                    _ => ResizeDirection::East,
                });
            }
            ViewportCommand::Title(title) => win.set_title(&title),
            ViewportCommand::Transparent(v) => win.set_transparent(v),
            ViewportCommand::Visible(v) => win.set_visible(v),
            ViewportCommand::OuterPosition(x, y) => {
                win.set_outer_position(LogicalPosition::new(x, y));
            }
            ViewportCommand::MinInnerSize(s) => {
                win.set_min_inner_size(s.map(|s| LogicalSize::new(s.0, s.1)));
            }
            ViewportCommand::MaxInnerSize(s) => {
                win.set_max_inner_size(s.map(|s| LogicalSize::new(s.0, s.1)));
            }
            ViewportCommand::ResizeIncrements(s) => {
                win.set_resize_increments(s.map(|s| LogicalSize::new(s.0, s.1)));
            }
            ViewportCommand::Resizable(v) => win.set_resizable(v),
            ViewportCommand::EnableButtons {
                close,
                minimized,
                maximize,
            } => win.set_enabled_buttons(
                if close {
                    WindowButtons::CLOSE
                } else {
                    WindowButtons::empty()
                } | if minimized {
                    WindowButtons::MINIMIZE
                } else {
                    WindowButtons::empty()
                } | if maximize {
                    WindowButtons::MAXIMIZE
                } else {
                    WindowButtons::empty()
                },
            ),
            ViewportCommand::Minimized(v) => win.set_minimized(v),
            ViewportCommand::Maximized(v) => win.set_maximized(v),
            ViewportCommand::Fullscreen(v) => {
                win.set_fullscreen(v.then_some(winit::window::Fullscreen::Borderless(None)));
            }
            ViewportCommand::Decorations(v) => win.set_decorations(v),
            ViewportCommand::WindowLevel(o) => win.set_window_level(match o {
                1 => WindowLevel::AlwaysOnBottom,
                2 => WindowLevel::AlwaysOnTop,
                _ => WindowLevel::Normal,
            }),
            ViewportCommand::WindowIcon(icon) => {
                win.set_window_icon(icon.map(|(bytes, width, height)| {
                    winit::window::Icon::from_rgba(bytes, width, height)
                        .expect("Invalid ICON data!")
                }));
            }
            ViewportCommand::IMEPosition(x, y) => win.set_ime_position(LogicalPosition::new(x, y)),
            ViewportCommand::IMEAllowed(v) => win.set_ime_allowed(v),
            ViewportCommand::IMEPurpose(o) => win.set_ime_purpose(match o {
                1 => winit::window::ImePurpose::Password,
                2 => winit::window::ImePurpose::Terminal,
                _ => winit::window::ImePurpose::Normal,
            }),
            ViewportCommand::RequestUserAttention(o) => win.request_user_attention(o.map(|o| {
                if o == 1 {
                    winit::window::UserAttentionType::Critical
                } else {
                    winit::window::UserAttentionType::Informational
                }
            })),
            ViewportCommand::SetTheme(o) => win.set_theme(o.map(|o| {
                if o == 1 {
                    winit::window::Theme::Dark
                } else {
                    winit::window::Theme::Light
                }
            })),
            ViewportCommand::ContentProtected(v) => win.set_content_protected(v),
            ViewportCommand::CursorPosition(x, y) => {
                if let Err(err) = win.set_cursor_position(LogicalPosition::new(x, y)) {
                    log::error!("{err}");
                }
            }
            ViewportCommand::CursorGrab(o) => {
                if let Err(err) = win.set_cursor_grab(match o {
                    1 => CursorGrabMode::Confined,
                    2 => CursorGrabMode::Locked,
                    _ => CursorGrabMode::None,
                }) {
                    log::error!("{err}");
                }
            }
            ViewportCommand::CursorVisible(v) => win.set_cursor_visible(v),
            ViewportCommand::CursorHitTest(v) => {
                if let Err(err) = win.set_cursor_hittest(v) {
                    log::error!("Setting viewport CursorHitTest: {err}");
                }
            }
        }
    }
}

pub fn process_viewports_commands(
    commands: Vec<(ViewportId, ViewportCommand)>,
    focused: Option<ViewportId>,
    get_window: impl Fn(ViewportId) -> Option<Arc<RwLock<winit::window::Window>>>,
) {
    for (viewport_id, command) in commands {
        if let Some(window) = get_window(viewport_id) {
            process_viewport_commands(vec![command], viewport_id, focused, &window);
        }
    }
}

pub fn create_winit_window_builder(builder: &ViewportBuilder) -> winit::window::WindowBuilder {
    let mut window_builder = winit::window::WindowBuilder::new()
        .with_title(builder.title.clone())
        .with_transparent(builder.transparent.map_or(false, |e| e))
        .with_decorations(builder.decorations.map_or(false, |e| e))
        .with_resizable(builder.resizable.map_or(false, |e| e))
        .with_visible(builder.visible.map_or(false, |e| e))
        .with_maximized(builder.minimized.map_or(false, |e| e))
        .with_maximized(builder.maximized.map_or(false, |e| e))
        .with_fullscreen(
            builder
                .fullscreen
                .and_then(|e| e.then_some(winit::window::Fullscreen::Borderless(None))),
        )
        .with_enabled_buttons(
            builder
                .minimize_button
                .and_then(|v| v.then_some(WindowButtons::MINIMIZE))
                .unwrap_or(WindowButtons::empty())
                | builder
                    .maximize_button
                    .and_then(|v| v.then_some(WindowButtons::MAXIMIZE))
                    .unwrap_or(WindowButtons::empty())
                | builder
                    .close_button
                    .and_then(|v| v.then_some(WindowButtons::CLOSE))
                    .unwrap_or(WindowButtons::empty()),
        )
        .with_active(builder.active.map_or(false, |e| e));
    if let Some(Some(inner_size)) = builder.inner_size {
        window_builder = window_builder
            .with_inner_size(winit::dpi::PhysicalSize::new(inner_size.0, inner_size.1));
    }
    if let Some(Some(min_inner_size)) = builder.min_inner_size {
        window_builder = window_builder.with_min_inner_size(winit::dpi::PhysicalSize::new(
            min_inner_size.0,
            min_inner_size.1,
        ));
    }
    if let Some(Some(max_inner_size)) = builder.max_inner_size {
        window_builder = window_builder.with_max_inner_size(winit::dpi::PhysicalSize::new(
            max_inner_size.0,
            max_inner_size.1,
        ));
    }
    if let Some(Some(position)) = builder.position {
        window_builder =
            window_builder.with_position(winit::dpi::PhysicalPosition::new(position.0, position.1));
    }

    if let Some(Some(icon)) = builder.icon.clone() {
        window_builder = window_builder.with_window_icon(Some(
            winit::window::Icon::from_rgba(icon.2.clone(), icon.0, icon.1)
                .expect("Invalid Icon Data!"),
        ));
    }

    #[cfg(all(feature = "wayland", target_os = "linux"))]
    if let Some(name) = builder.name.clone() {
        use winit::platform::wayland::WindowBuilderExtWayland as _;
        window_builder = window_builder.with_name(name.0, name.1);
    }

    #[cfg(target_os = "windows")]
    if let Some(enable) = builder.drag_and_drop {
        use winit::platform::windows::WindowBuilderExtWindows as _;
        window_builder = window_builder.with_drag_and_drop(enable);
    }

    // TODO: implement `ViewportBuilder::hittest`
    // Is not implemented because winit in his current state will not allow to set cursor_hittest on a `WindowBuilder`

    window_builder
}

pub fn changes_between_builders(
    new: &ViewportBuilder,
    last: &mut ViewportBuilder,
) -> (Vec<ViewportCommand>, bool) {
    let mut commands = Vec::new();

    // Title is not compared because if has a new title will create a new window
    // The title of a avalibile window can only be changed with ViewportCommand::Title

    if let Some(position) = new.position {
        if Some(position) != last.position {
            last.position = Some(position);
            if let Some(position) = position {
                commands.push(ViewportCommand::OuterPosition(position.0, position.1));
            }
        }
    }

    if let Some(inner_size) = new.inner_size {
        if Some(inner_size) != last.inner_size {
            last.inner_size = Some(inner_size);
            if let Some(inner_size) = inner_size {
                commands.push(ViewportCommand::InnerSize(inner_size.0, inner_size.1));
            }
        }
    }

    if let Some(min_inner_size) = new.min_inner_size {
        if Some(min_inner_size) != last.min_inner_size {
            last.min_inner_size = Some(min_inner_size);
            commands.push(ViewportCommand::MinInnerSize(min_inner_size));
        }
    }

    if let Some(max_inner_size) = new.max_inner_size {
        if Some(max_inner_size) != last.max_inner_size {
            last.max_inner_size = Some(max_inner_size);
            commands.push(ViewportCommand::MaxInnerSize(max_inner_size));
        }
    }

    if let Some(fullscreen) = new.fullscreen {
        if Some(fullscreen) != last.fullscreen {
            last.fullscreen = Some(fullscreen);
            commands.push(ViewportCommand::Fullscreen(fullscreen));
        }
    }

    if let Some(minimized) = new.minimized {
        if Some(minimized) != last.minimized {
            last.minimized = Some(minimized);
            commands.push(ViewportCommand::Minimized(minimized));
        }
    }

    if let Some(maximized) = new.maximized {
        if Some(maximized) != last.maximized {
            last.maximized = Some(maximized);
            commands.push(ViewportCommand::Maximized(maximized));
        }
    }

    if let Some(resizable) = new.resizable {
        if Some(resizable) != last.resizable {
            last.resizable = Some(resizable);
            commands.push(ViewportCommand::Resizable(resizable));
        }
    }

    if let Some(transparent) = new.transparent {
        if Some(transparent) != last.transparent {
            last.transparent = Some(transparent);
            commands.push(ViewportCommand::Transparent(transparent));
        }
    }

    if let Some(decorations) = new.decorations {
        if Some(decorations) != last.decorations {
            last.decorations = Some(decorations);
            commands.push(ViewportCommand::Decorations(decorations));
        }
    }

    if let Some(icon) = new.icon.clone() {
        let eq = match &icon {
            Some(icon) => {
                if let Some(last_icon) = &last.icon {
                    matches!(last_icon, Some(last_icon) if Arc::ptr_eq(icon, last_icon))
                } else {
                    false
                }
            }
            None => last.icon == Some(None),
        };

        if !eq {
            commands.push(ViewportCommand::WindowIcon(
                icon.as_ref().map(|i| (i.2.clone(), i.0, i.1)),
            ));
            last.icon = Some(icon);
        }
    }

    if let Some(visible) = new.visible {
        if Some(visible) != last.active {
            last.visible = Some(visible);
            commands.push(ViewportCommand::Visible(visible));
        }
    }

    if let Some(hittest) = new.hittest {
        if Some(hittest) != last.hittest {
            last.hittest = Some(hittest);
            commands.push(ViewportCommand::CursorHitTest(hittest));
        }
    }

    // TODO: Implement compare for windows buttons

    let mut recreate_window = false;

    if let Some(active) = new.active {
        if Some(active) != last.active {
            last.active = Some(active);
            recreate_window = true;
        }
    }

    if let Some(close_button) = new.close_button {
        if Some(close_button) != last.close_button {
            last.close_button = Some(close_button);
            recreate_window = true;
        }
    }

    if let Some(minimize_button) = new.minimize_button {
        if Some(minimize_button) != last.minimize_button {
            last.minimize_button = Some(minimize_button);
            recreate_window = true;
        }
    }

    if let Some(maximized_button) = new.maximize_button {
        if Some(maximized_button) != last.maximize_button {
            last.maximize_button = Some(maximized_button);
            recreate_window = true;
        }
    }

    if let Some(title_hidden) = new.title_hidden {
        if Some(title_hidden) != last.title_hidden {
            last.title_hidden = Some(title_hidden);
            recreate_window = true;
        }
    }

    if let Some(titlebar_transparent) = new.titlebar_transparent {
        if Some(titlebar_transparent) != last.titlebar_transparent {
            last.titlebar_transparent = Some(titlebar_transparent);
            recreate_window = true;
        }
    }

    if let Some(value) = new.fullsize_content_view {
        if Some(value) != last.fullsize_content_view {
            last.fullsize_content_view = Some(value);
            recreate_window = true;
        }
    }

    (commands, recreate_window)
}
// ---------------------------------------------------------------------------

/// Profiling macro for feature "puffin"
#[allow(unused_macros)]
macro_rules! profile_function {
    ($($arg: tt)*) => {
        #[cfg(feature = "puffin")]
        puffin::profile_function!($($arg)*);
    };
}

#[allow(unused_imports)]
pub(crate) use profile_function;

/// Profiling macro for feature "puffin"
#[allow(unused_macros)]
macro_rules! profile_scope {
    ($($arg: tt)*) => {
        #[cfg(feature = "puffin")]
        puffin::profile_scope!($($arg)*);
    };
}

#[allow(unused_imports)]
pub(crate) use profile_scope;
use winit::{
    dpi::{LogicalPosition, LogicalSize},
    window::{CursorGrabMode, WindowButtons, WindowLevel},
};
