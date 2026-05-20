use std::time::{Duration, Instant};

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
    control::{text_to_key_events, ControlCommand, ControlCommandVariant},
    state::{AutoWC, QueuedControlAction, QueuedControlActionKind},
    window::AutoWindowId,
};

pub const CONTROL_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(5);
pub const DEFAULT_KEY_EVENT_INTERVAL_MS: u64 = 20;
pub const DEFAULT_CHORD_KEY_INTERVAL_MS: u64 = 10;
pub const DEFAULT_CHORD_HOLD_DURATION_MS: u64 = 75;
pub const DEFAULT_COMMAND_INTERVAL_MS: u64 = 0;

pub const DEFAULT_KEY_EVENT_INTERVAL: Duration =
    Duration::from_millis(DEFAULT_KEY_EVENT_INTERVAL_MS);
pub const DEFAULT_CHORD_KEY_INTERVAL: Duration =
    Duration::from_millis(DEFAULT_CHORD_KEY_INTERVAL_MS);
pub const DEFAULT_CHORD_HOLD_DURATION: Duration =
    Duration::from_millis(DEFAULT_CHORD_HOLD_DURATION_MS);
pub const DEFAULT_COMMAND_INTERVAL: Duration = Duration::from_millis(DEFAULT_COMMAND_INTERVAL_MS);

impl AutoWC {
    pub fn process_control_command(&mut self, command: ControlCommand) -> Result<(), String> {
        let window_id = match command.window {
            Some(window) => {
                let window_id = AutoWindowId::from_raw(window)
                    .ok_or_else(|| "invalid window id".to_string())?;
                if self
                    .windows
                    .get(window_id)
                    .is_none_or(|window| window.is_empty())
                {
                    return Err(format!("unknown window: {}", window_id.raw()));
                }
                window_id
            }
            None => self
                .first_alive_window_id
                .ok_or_else(|| "no windows are open".to_string())?,
        };

        match command.variant {
            ControlCommandVariant::Key { code, action } => {
                for state in action.key_states() {
                    self.queue_control_action(
                        window_id,
                        QueuedControlActionKind::Key {
                            code,
                            state: *state,
                        },
                    );
                    self.queue_control_action(
                        window_id,
                        QueuedControlActionKind::Delay(self.key_event_interval),
                    );
                }
            }
            ControlCommandVariant::Chord { codes } => {
                let mut pressed_codes = codes.iter().peekable();
                while let Some(code) = pressed_codes.next() {
                    self.queue_control_action(
                        window_id,
                        QueuedControlActionKind::Key {
                            code: *code,
                            state: KeyState::Pressed,
                        },
                    );
                    if pressed_codes.peek().is_some() {
                        self.queue_control_action(
                            window_id,
                            QueuedControlActionKind::Delay(self.chord_key_interval),
                        );
                    }
                }
                self.queue_control_action(
                    window_id,
                    QueuedControlActionKind::Delay(self.chord_hold_duration),
                );
                for code in codes.iter().rev() {
                    self.queue_control_action(
                        window_id,
                        QueuedControlActionKind::Key {
                            code: *code,
                            state: KeyState::Released,
                        },
                    );
                }
                self.queue_control_action(
                    window_id,
                    QueuedControlActionKind::Delay(self.key_event_interval),
                );
            }
            ControlCommandVariant::Text(text) => {
                for (code, action) in text_to_key_events(&text)? {
                    for state in action.key_states() {
                        self.queue_control_action(
                            window_id,
                            QueuedControlActionKind::Key {
                                code,
                                state: *state,
                            },
                        );
                        self.queue_control_action(
                            window_id,
                            QueuedControlActionKind::Delay(self.key_event_interval),
                        );
                    }
                }
            }
            ControlCommandVariant::PointerMove { x, y } => {
                self.queue_control_action(window_id, QueuedControlActionKind::PointerMove { x, y });
            }
            ControlCommandVariant::PointerButton { button, action } => {
                for state in action.button_states() {
                    self.queue_control_action(
                        window_id,
                        QueuedControlActionKind::PointerButton {
                            button,
                            state: *state,
                        },
                    );
                }
            }
            ControlCommandVariant::Click { x, y, button } => {
                self.queue_control_action(window_id, QueuedControlActionKind::PointerMove { x, y });
                self.queue_control_action(
                    window_id,
                    QueuedControlActionKind::PointerButton {
                        button,
                        state: ButtonState::Pressed,
                    },
                );
                self.queue_control_action(
                    window_id,
                    QueuedControlActionKind::PointerButton {
                        button,
                        state: ButtonState::Released,
                    },
                );
            }
            ControlCommandVariant::Scroll { dx, dy } => {
                self.queue_control_action(window_id, QueuedControlActionKind::Scroll { dx, dy });
            }
            ControlCommandVariant::Screenshot { path } => {
                self.queue_control_action(window_id, QueuedControlActionKind::Screenshot { path });
            }
            ControlCommandVariant::Sleep { duration_ms } => {
                self.queue_control_action(
                    window_id,
                    QueuedControlActionKind::Delay(Duration::from_millis(duration_ms)),
                );
            }
            ControlCommandVariant::Quit => {
                self.queue_control_action(window_id, QueuedControlActionKind::Quit);
            }
        }

        if !self.command_interval.is_zero() {
            self.queue_control_action(
                window_id,
                QueuedControlActionKind::Delay(self.command_interval),
            );
        }

        Ok(())
    }

    pub fn process_pending_control_actions(&mut self) {
        let now = Instant::now();
        if self
            .next_control_action_at
            .is_some_and(|instant| instant > now)
        {
            return;
        }
        self.next_control_action_at = None;

        while let Some(action) = self.control_queue.pop_front() {
            match action.kind {
                QueuedControlActionKind::Key { code, state } => {
                    self.process_virtual_input_event(action.window_id, code, state);
                }
                QueuedControlActionKind::PointerMove { x, y } => {
                    self.process_virtual_pointer_motion(action.window_id, (x, y).into());
                }
                QueuedControlActionKind::PointerButton { button, state } => {
                    self.process_virtual_pointer_button(button, state);
                }
                QueuedControlActionKind::Scroll { dx, dy } => {
                    self.process_virtual_scroll(dx, dy);
                }
                QueuedControlActionKind::Screenshot { path } => {
                    self.queue_screenshot(action.window_id, path);
                }
                QueuedControlActionKind::Quit => {
                    self.request_shutdown();
                }
                QueuedControlActionKind::Delay(duration) => {
                    self.next_control_action_at = Some(Instant::now() + duration);
                    return;
                }
            }
        }
    }

    fn queue_control_action(&mut self, window_id: AutoWindowId, kind: QueuedControlActionKind) {
        self.control_queue
            .push_back(QueuedControlAction { window_id, kind });
    }

    pub fn process_virtual_input_event(
        &mut self,
        window_id: AutoWindowId,
        code: u32,
        state: KeyState,
    ) {
        self.focus_auto_window(window_id);

        let serial = SERIAL_COUNTER.next_serial();
        let time = self.now_msec();

        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            Keycode::new(code + 8),
            state,
            serial,
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    pub fn process_virtual_pointer_motion(
        &mut self,
        window_id: AutoWindowId,
        pos: Point<f64, smithay::utils::Logical>,
    ) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();
        let under = self.surface_under(window_id, pos);

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

    pub fn process_virtual_pointer_button(&mut self, button: u32, state: ButtonState) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.seat.get_pointer().unwrap();

        pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time: self.now_msec(),
            },
        );
        pointer.frame(self);
    }

    pub fn process_virtual_scroll(&mut self, dx: f64, dy: f64) {
        let mut frame = AxisFrame::new(self.now_msec()).source(AxisSource::Wheel);
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

    pub fn process_input_event<I: InputBackend>(
        &mut self,
        window_id: AutoWindowId,
        event: InputEvent<I>,
    ) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                self.focus_auto_window(window_id);

                let serial = SERIAL_COUNTER.next_serial();
                let time = self.now_msec();

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

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
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

    fn now_msec(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }
}
