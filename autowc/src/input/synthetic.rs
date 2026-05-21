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
    control::{text_to_key_events, ControlCommand, ControlCommandVariant, PressAction},
    input::keyboard::keys_sequence::KeysSequenceAction,
    protocol::ControlResponse,
    state::{AutoWC, ControlResponseHandle, QueuedControlAction, QueuedControlActionKind},
    window::AutoWindowId,
};
use tracing::{debug, trace};

enum PreparedKeysSequenceAction {
    TextEvents(Vec<(u32, PressAction)>),
    Chord(Vec<u32>),
    Wait(Duration),
}

impl AutoWC {
    pub fn process_control_command(
        &mut self,
        command: ControlCommand,
        response: ControlResponseHandle,
    ) {
        debug!(window = ?command.window, variant = ?command.variant, "processing control command");
        if let ControlCommandVariant::Launch { command } = &command.variant {
            let command_response = match self.launch_child(command) {
                Ok(()) => ControlResponse::Ok,
                Err(err) => ControlResponse::Error(err),
            };
            self.complete_control_response(response, command_response);
            return;
        }
        if command.variant == ControlCommandVariant::List {
            let windows = self.window_infos();
            self.complete_control_response(response, ControlResponse::WindowList { windows });
            return;
        }
        if command.variant == ControlCommandVariant::Quit {
            self.request_shutdown();
            self.complete_control_response(response, ControlResponse::Ok);
            return;
        }

        let window_id = match command.window {
            Some(window) => {
                let Some(window_id) = AutoWindowId::from_raw(window) else {
                    self.complete_control_response(
                        response,
                        ControlResponse::Error("invalid window id".to_string()),
                    );
                    return;
                };
                if self
                    .windows
                    .get(window_id)
                    .is_none_or(|window| window.is_empty())
                {
                    self.complete_control_response(
                        response,
                        ControlResponse::Error(format!("unknown window: {}", window_id.raw())),
                    );
                    return;
                }
                window_id
            }
            None => match self.first_alive_window_id {
                Some(window_id) => window_id,
                None => {
                    self.complete_control_response(
                        response,
                        ControlResponse::Error("no windows are open".to_string()),
                    );
                    return;
                }
            },
        };
        debug!(?window_id, "control command target selected");

        let queued_action_handles_response = matches!(
            &command.variant,
            ControlCommandVariant::Screenshot { .. } | ControlCommandVariant::Close
        );

        match command.variant {
            ControlCommandVariant::Key { code, action } => {
                self.queue_key_press_action(window_id, response, code, action);
            }
            ControlCommandVariant::Chord { codes } => {
                self.queue_chord_codes(window_id, response, &codes);
            }
            ControlCommandVariant::Text(text) => {
                let events = match text_to_key_events(&text) {
                    Ok(events) => events,
                    Err(err) => {
                        self.complete_control_response(response, ControlResponse::Error(err));
                        return;
                    }
                };
                self.queue_text_key_events(window_id, response, events);
            }
            ControlCommandVariant::KeysSequence { actions } => {
                let actions = match prepare_keys_sequence_actions(actions) {
                    Ok(actions) => actions,
                    Err(err) => {
                        self.complete_control_response(response, ControlResponse::Error(err));
                        return;
                    }
                };
                for action in actions {
                    match action {
                        PreparedKeysSequenceAction::TextEvents(events) => {
                            self.queue_text_key_events(window_id, response, events);
                        }
                        PreparedKeysSequenceAction::Chord(codes) => {
                            self.queue_chord_codes(window_id, response, &codes);
                        }
                        PreparedKeysSequenceAction::Wait(duration) => {
                            self.queue_control_action(
                                window_id,
                                response,
                                QueuedControlActionKind::Delay(duration),
                            );
                        }
                    }
                }
            }
            ControlCommandVariant::PointerMove { x, y } => {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::PointerMove { x, y },
                );
            }
            ControlCommandVariant::PointerButton { button, action } => {
                for state in action.button_states() {
                    self.queue_control_action(
                        window_id,
                        response,
                        QueuedControlActionKind::PointerButton {
                            button,
                            state: *state,
                        },
                    );
                }
            }
            ControlCommandVariant::Click { x, y, button } => {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::PointerMove { x, y },
                );
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::PointerButton {
                        button,
                        state: ButtonState::Pressed,
                    },
                );
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::PointerButton {
                        button,
                        state: ButtonState::Released,
                    },
                );
            }
            ControlCommandVariant::Scroll { dx, dy } => {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::Scroll { dx, dy },
                );
            }
            ControlCommandVariant::Screenshot { path } => {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::Screenshot {
                        path,
                        delay_after: self.command_interval,
                    },
                );
            }
            ControlCommandVariant::Close => {
                self.queue_control_action(window_id, response, QueuedControlActionKind::Close);
            }
            ControlCommandVariant::Sleep { duration_ms } => {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::Delay(Duration::from_millis(duration_ms)),
                );
            }
            ControlCommandVariant::Launch { .. } => unreachable!("launch is handled immediately"),
            ControlCommandVariant::List => unreachable!("list is handled by the protocol layer"),
            ControlCommandVariant::Quit => unreachable!("quit is handled immediately"),
        }

        if !queued_action_handles_response {
            self.queue_control_action(
                window_id,
                response,
                QueuedControlActionKind::Respond {
                    response: ControlResponse::Ok,
                    delay_after: self.command_interval,
                },
            );
        }
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
            if action.kind.requires_live_window()
                && self
                    .windows
                    .get(action.window_id)
                    .is_none_or(|window| window.is_empty())
            {
                self.complete_control_response(
                    action.response,
                    ControlResponse::Error(format!("unknown window: {}", action.window_id.raw())),
                );
                self.control_queue
                    .retain(|queued| queued.response != action.response);
                self.flush_control_responses();
                continue;
            }
            if action.kind.delivers_to_window() {
                self.note_control_response_delivered(action.response);
            }

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
                QueuedControlActionKind::Screenshot { path, delay_after } => {
                    self.queue_screenshot(action.window_id, path, action.response);
                    if !delay_after.is_zero() {
                        self.next_control_action_at = Some(Instant::now() + delay_after);
                        return;
                    }
                }
                QueuedControlActionKind::Close => {
                    let response = match self.close_auto_window(action.window_id) {
                        Ok(()) => ControlResponse::Ok,
                        Err(err) => ControlResponse::Error(err),
                    };
                    self.complete_control_response(action.response, response);
                    self.flush_control_responses();
                }
                QueuedControlActionKind::Delay(duration) => {
                    self.next_control_action_at = Some(Instant::now() + duration);
                    return;
                }
                QueuedControlActionKind::Respond {
                    response,
                    delay_after,
                } => {
                    self.complete_control_response(action.response, response);
                    self.flush_control_responses();
                    if !delay_after.is_zero() {
                        self.next_control_action_at = Some(Instant::now() + delay_after);
                        return;
                    }
                }
            }
        }
    }

    fn queue_control_action(
        &mut self,
        window_id: AutoWindowId,
        response: ControlResponseHandle,
        kind: QueuedControlActionKind,
    ) {
        trace!(?window_id, ?kind, "queueing control action");
        self.control_queue.push_back(QueuedControlAction {
            window_id,
            response,
            kind,
        });
    }

    fn queue_key_press_action(
        &mut self,
        window_id: AutoWindowId,
        response: ControlResponseHandle,
        code: u32,
        action: PressAction,
    ) {
        for state in action.key_states() {
            self.queue_control_action(
                window_id,
                response,
                QueuedControlActionKind::Key {
                    code,
                    state: *state,
                },
            );
            self.queue_control_action(
                window_id,
                response,
                QueuedControlActionKind::Delay(self.key_event_interval),
            );
        }
    }

    fn queue_chord_codes(
        &mut self,
        window_id: AutoWindowId,
        response: ControlResponseHandle,
        codes: &[u32],
    ) {
        let mut pressed_codes = codes.iter().peekable();
        while let Some(code) = pressed_codes.next() {
            self.queue_control_action(
                window_id,
                response,
                QueuedControlActionKind::Key {
                    code: *code,
                    state: KeyState::Pressed,
                },
            );
            if pressed_codes.peek().is_some() {
                self.queue_control_action(
                    window_id,
                    response,
                    QueuedControlActionKind::Delay(self.chord_key_interval),
                );
            }
        }
        self.queue_control_action(
            window_id,
            response,
            QueuedControlActionKind::Delay(self.chord_hold_duration),
        );
        for code in codes.iter().rev() {
            self.queue_control_action(
                window_id,
                response,
                QueuedControlActionKind::Key {
                    code: *code,
                    state: KeyState::Released,
                },
            );
        }
        self.queue_control_action(
            window_id,
            response,
            QueuedControlActionKind::Delay(self.key_event_interval),
        );
    }

    fn queue_text_key_events(
        &mut self,
        window_id: AutoWindowId,
        response: ControlResponseHandle,
        events: impl IntoIterator<Item = (u32, PressAction)>,
    ) {
        for (code, action) in events {
            self.queue_key_press_action(window_id, response, code, action);
        }
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

fn prepare_keys_sequence_actions(
    actions: Vec<KeysSequenceAction>,
) -> Result<Vec<PreparedKeysSequenceAction>, String> {
    let mut prepared = Vec::with_capacity(actions.len());
    for action in actions {
        prepared.push(match action {
            KeysSequenceAction::Text(text) => {
                PreparedKeysSequenceAction::TextEvents(text_to_key_events(&text)?)
            }
            KeysSequenceAction::Chord(codes) => PreparedKeysSequenceAction::Chord(codes),
            KeysSequenceAction::Wait { duration_ms } => {
                PreparedKeysSequenceAction::Wait(Duration::from_millis(duration_ms))
            }
        });
    }
    Ok(prepared)
}
