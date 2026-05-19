use std::time::{SystemTime, UNIX_EPOCH};

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, Keycode, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Physical, Point, SERIAL_COUNTER},
};

use crate::{
    control::{text_to_key_events, ControlCommand},
    state::AutoWC,
};

impl AutoWC {
    pub fn process_control_command(&mut self, command: ControlCommand) -> Result<(), String> {
        match command {
            ControlCommand::Key { code, action } => {
                for state in action.key_states() {
                    self.process_virtual_input_event(code, *state);
                }
            }
            ControlCommand::Text(text) => {
                for (code, action) in text_to_key_events(&text)? {
                    for state in action.key_states() {
                        self.process_virtual_input_event(code, *state);
                    }
                }
            }
            ControlCommand::PointerMove { x, y } => {
                self.process_virtual_pointer_motion((x, y).into());
            }
            ControlCommand::PointerButton { button, action } => {
                for state in action.button_states() {
                    self.process_virtual_pointer_button(button, *state);
                }
            }
            ControlCommand::Click { x, y, button } => {
                self.process_virtual_pointer_motion((x, y).into());
                self.process_virtual_pointer_button(button, ButtonState::Pressed);
                self.process_virtual_pointer_button(button, ButtonState::Released);
            }
            ControlCommand::Scroll { dx, dy } => {
                self.process_virtual_scroll(dx, dy);
            }
            ControlCommand::Quit => {
                self.loop_signal.stop();
            }
        }

        Ok(())
    }

    pub fn process_virtual_input_event(&mut self, code: u32, state: KeyState) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = now_msec();

        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            Keycode::new(code + 8),
            state,
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    pub fn process_virtual_pointer_motion(&mut self, pos: Point<f64, smithay::utils::Logical>) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();
        let under = self.surface_under(pos);

        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time: now_msec(),
            },
        );
        pointer.frame(self);
    }

    pub fn process_virtual_pointer_button(&mut self, button: u32, state: ButtonState) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();

        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time: now_msec(),
            },
        );
        pointer.frame(self);
    }

    pub fn process_virtual_scroll(&mut self, dx: f64, dy: f64) {
        let mut frame = AxisFrame::new(now_msec()).source(AxisSource::Wheel);
        if dx != 0.0 {
            frame = frame.value(Axis::Horizontal, dx);
        }
        if dy != 0.0 {
            frame = frame.value(Axis::Vertical, dy);
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotion { .. } => {}
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let host_pos: Point<f64, Physical> = (event.x(), event.y()).into();
                let pointer = self.seat.get_pointer().unwrap();

                let (pos, under) = if let Some(pos) = self.host_to_virtual(host_pos) {
                    self.pointer_in_viewport = true;
                    (pos, self.surface_under(pos))
                } else {
                    self.pointer_in_viewport = false;
                    (pointer.current_location(), None)
                };

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                if !self.pointer_in_viewport {
                    return;
                }

                let pointer = self.seat.get_pointer().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                if !self.pointer_in_viewport {
                    return;
                }

                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
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
}

fn now_msec() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32
}
