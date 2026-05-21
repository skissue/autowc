use std::time::{Duration, Instant};

use smithay::{
    backend::input::{Axis, AxisSource, ButtonState, KeyState, Keycode},
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Point, SERIAL_COUNTER},
};

use crate::{
    control::{text_to_key_events, ControlCommand, ControlCommandVariant},
    state::{AutoWC, QueuedControlAction, QueuedControlActionKind},
    window::AutoWindowId,
};
use tracing::{debug, trace};

impl AutoWC {
    pub fn process_control_command(&mut self, command: ControlCommand) -> Result<(), String> {
        debug!(window = ?command.window, variant = ?command.variant, "processing control command");
        if let ControlCommandVariant::Launch { command } = &command.variant {
            return self.launch_child(command);
        }
        if command.variant == ControlCommandVariant::List {
            return Ok(());
        }

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
        debug!(?window_id, "control command target selected");

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
            ControlCommandVariant::Launch { .. } => unreachable!("launch is handled immediately"),
            ControlCommandVariant::List => unreachable!("list is handled by the protocol layer"),
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
            trace!(?action, "processing queued control action");
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
        trace!(?window_id, ?kind, "queueing control action");
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
        trace!(
            ?window_id,
            code,
            ?state,
            ?serial,
            time,
            "sending virtual key event"
        );

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
        trace!(
            ?window_id,
            ?pos,
            has_surface = under.is_some(),
            ?serial,
            "sending virtual pointer motion"
        );

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
        trace!(button, ?state, ?serial, "sending virtual pointer button");

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
        trace!(dx, dy, "sending virtual scroll");
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
}
