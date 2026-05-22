use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Physical, Point, SERIAL_COUNTER},
};

use crate::{
    input::{SCROLL_AXIS_VALUE_PER_WHEEL_DETENT, SCROLL_V120_PER_WHEEL_DETENT},
    state::AutoWC,
    window::AutoWindowId,
};
use tracing::trace;

impl AutoWC {
    pub fn should_process_blocked_host_input<I: InputBackend>(
        &self,
        window_id: AutoWindowId,
        event: &InputEvent<I>,
    ) -> bool {
        matches!(
            event,
            InputEvent::Keyboard { event, .. }
                if event.state() == KeyState::Released
                    && self.is_host_key_pressed(window_id, event.key_code())
        )
    }

    pub fn release_pressed_host_keys(&mut self, window_id: AutoWindowId) {
        let Some(keys) = self.host_pressed_keys.remove(&window_id) else {
            return;
        };

        let mut keys = keys.into_iter().collect::<Vec<_>>();
        keys.sort_by_key(|key| key.raw());
        trace!(
            ?window_id,
            count = keys.len(),
            "releasing pressed host keys after host focus loss"
        );

        self.focus_auto_window(window_id);
        for key_code in keys {
            self.forward_host_keyboard_input(window_id, key_code, KeyState::Released);
        }
    }

    pub fn process_input_event<I: InputBackend>(
        &mut self,
        window_id: AutoWindowId,
        event: InputEvent<I>,
    ) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                trace!(?window_id, key_code = ?event.key_code(), state = ?event.state(), "forwarding host keyboard input");
                self.focus_auto_window(window_id);

                let key_code = event.key_code();
                let state = event.state();
                self.record_host_key_state(window_id, key_code, state);
                self.forward_host_keyboard_input(window_id, key_code, state);
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let host_pos: Point<f64, Physical> = (event.x(), event.y()).into();
                trace!(?window_id, ?host_pos, "forwarding host pointer motion");
                let pointer = self.seat.get_pointer().unwrap();

                let (pos, under) = self
                    .host_to_virtual(window_id, host_pos)
                    .map(|pos| (pos, self.surface_under(window_id, pos)))
                    .unwrap_or_else(|| (pointer.current_location(), None));

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: self.now_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();
                trace!(
                    ?window_id,
                    button,
                    ?button_state,
                    "forwarding host pointer button"
                );

                if button_state == ButtonState::Pressed && !pointer.is_grabbed() {
                    let focus = self.element_under(window_id, pointer.current_location());
                    self.focus_window(window_id, focus.as_ref());
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: self.now_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();
                trace!(?window_id, ?source, "forwarding host pointer axis");

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0)
                        * SCROLL_AXIS_VALUE_PER_WHEEL_DETENT
                        / SCROLL_V120_PER_WHEEL_DETENT
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0)
                        * SCROLL_AXIS_VALUE_PER_WHEEL_DETENT
                        / SCROLL_V120_PER_WHEEL_DETENT
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(self.now_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    fn is_host_key_pressed(&self, window_id: AutoWindowId, key_code: Keycode) -> bool {
        self.host_pressed_keys
            .get(&window_id)
            .is_some_and(|keys| keys.contains(&key_code))
    }

    fn record_host_key_state(
        &mut self,
        window_id: AutoWindowId,
        key_code: Keycode,
        state: KeyState,
    ) {
        match state {
            KeyState::Pressed => {
                self.host_pressed_keys
                    .entry(window_id)
                    .or_default()
                    .insert(key_code);
            }
            KeyState::Released => {
                let Some(keys) = self.host_pressed_keys.get_mut(&window_id) else {
                    return;
                };
                keys.remove(&key_code);
                if keys.is_empty() {
                    self.host_pressed_keys.remove(&window_id);
                }
            }
        }
    }

    fn forward_host_keyboard_input(
        &mut self,
        window_id: AutoWindowId,
        key_code: Keycode,
        state: KeyState,
    ) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();

        trace!(
            ?window_id,
            ?key_code,
            ?state,
            ?serial,
            time,
            "sending host key event"
        );
        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            key_code,
            state,
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }
}
