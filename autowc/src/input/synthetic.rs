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
    input::{
        keyboard::keys_sequence::KeysSequenceAction, SCROLL_AXIS_VALUE_PER_WHEEL_DETENT,
        SCROLL_V120_PER_WHEEL_DETENT,
    },
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct PreparedScrollAxis {
    axis: f64,
    v120: i32,
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
            ControlCommandVariant::PointerDrag {
                start_x,
                start_y,
                end_x,
                end_y,
                button,
            } => {
                for kind in pointer_drag_actions(start_x, start_y, end_x, end_y, button) {
                    self.queue_control_action(window_id, response, kind);
                }
            }
            ControlCommandVariant::Scroll { dx, dy } => {
                let dx = match prepare_scroll_axis(dx, "dx") {
                    Ok(axis) => axis,
                    Err(err) => {
                        self.complete_control_response(response, ControlResponse::Error(err));
                        return;
                    }
                };
                let dy = match prepare_scroll_axis(dy, "dy") {
                    Ok(axis) => axis,
                    Err(err) => {
                        self.complete_control_response(response, ControlResponse::Error(err));
                        return;
                    }
                };
                if dx.v120 != 0 || dy.v120 != 0 {
                    self.queue_control_action(
                        window_id,
                        response,
                        QueuedControlActionKind::Scroll {
                            dx_axis: dx.axis,
                            dy_axis: dy.axis,
                            dx_v120: dx.v120,
                            dy_v120: dy.v120,
                        },
                    );
                }
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
                QueuedControlActionKind::Scroll {
                    dx_axis,
                    dy_axis,
                    dx_v120,
                    dy_v120,
                } => {
                    self.process_virtual_scroll(dx_axis, dy_axis, dx_v120, dy_v120);
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

    pub fn process_virtual_scroll(
        &mut self,
        dx_axis: f64,
        dy_axis: f64,
        dx_v120: i32,
        dy_v120: i32,
    ) {
        if dx_v120 == 0 && dy_v120 == 0 {
            trace!("ignoring zero virtual scroll");
            return;
        }

        trace!(dx_axis, dy_axis, dx_v120, dy_v120, "sending virtual scroll");
        let mut frame = AxisFrame::new(self.now_msec()).source(AxisSource::Wheel);
        if dx_v120 != 0 {
            frame = frame
                .value(Axis::Horizontal, dx_axis)
                .v120(Axis::Horizontal, dx_v120);
        }
        if dy_v120 != 0 {
            frame = frame
                .value(Axis::Vertical, dy_axis)
                .v120(Axis::Vertical, dy_v120);
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

fn prepare_scroll_axis(detents: f64, name: &str) -> Result<PreparedScrollAxis, String> {
    if !detents.is_finite() {
        return Err(format!("scroll {name} must be finite"));
    }

    let v120 = (detents * SCROLL_V120_PER_WHEEL_DETENT).round();
    if !v120.is_finite() || v120 < i32::MIN as f64 || v120 > i32::MAX as f64 {
        return Err(format!("scroll {name} is too large"));
    }

    let v120 = v120 as i32;
    Ok(PreparedScrollAxis {
        axis: v120 as f64 * SCROLL_AXIS_VALUE_PER_WHEEL_DETENT / SCROLL_V120_PER_WHEEL_DETENT,
        v120,
    })
}

fn pointer_drag_actions(
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    button: u32,
) -> [QueuedControlActionKind; 4] {
    [
        QueuedControlActionKind::PointerMove {
            x: start_x,
            y: start_y,
        },
        QueuedControlActionKind::PointerButton {
            button,
            state: ButtonState::Pressed,
        },
        QueuedControlActionKind::PointerMove { x: end_x, y: end_y },
        QueuedControlActionKind::PointerButton {
            button,
            state: ButtonState::Released,
        },
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_pointer_drag_to_low_level_mouse_actions() {
        let actions = pointer_drag_actions(10.0, 20.0, 30.0, 40.0, 273);

        match actions {
            [QueuedControlActionKind::PointerMove { x: 10.0, y: 20.0 }, QueuedControlActionKind::PointerButton {
                button: 273,
                state: ButtonState::Pressed,
            }, QueuedControlActionKind::PointerMove { x: 30.0, y: 40.0 }, QueuedControlActionKind::PointerButton {
                button: 273,
                state: ButtonState::Released,
            }] => {}
            other => panic!("unexpected drag action expansion: {other:?}"),
        }
    }

    #[test]
    fn prepares_scroll_detents() {
        assert_eq!(
            prepare_scroll_axis(1.0, "dy").unwrap(),
            PreparedScrollAxis {
                axis: 15.0,
                v120: 120,
            }
        );
        assert_eq!(
            prepare_scroll_axis(0.25, "dy").unwrap(),
            PreparedScrollAxis {
                axis: 3.75,
                v120: 30,
            }
        );
        assert_eq!(
            prepare_scroll_axis(-2.0, "dy").unwrap(),
            PreparedScrollAxis {
                axis: -30.0,
                v120: -240,
            }
        );
    }

    #[test]
    fn rounds_scroll_detents_to_nearest_v120_unit() {
        assert_eq!(
            prepare_scroll_axis(1.0 / 120.0, "dy").unwrap(),
            PreparedScrollAxis {
                axis: 0.125,
                v120: 1,
            }
        );
        assert_eq!(
            prepare_scroll_axis(0.001, "dy").unwrap(),
            PreparedScrollAxis { axis: 0.0, v120: 0 }
        );
    }

    #[test]
    fn rejects_invalid_scroll_detents() {
        assert_eq!(
            prepare_scroll_axis(f64::NAN, "dy").unwrap_err(),
            "scroll dy must be finite"
        );
        assert_eq!(
            prepare_scroll_axis(f64::INFINITY, "dy").unwrap_err(),
            "scroll dy must be finite"
        );
        assert_eq!(
            prepare_scroll_axis(i32::MAX as f64, "dy").unwrap_err(),
            "scroll dy is too large"
        );
    }
}
