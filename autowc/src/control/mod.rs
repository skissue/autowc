use std::{ffi::OsString, path::PathBuf};

use smithay::backend::input::{ButtonState, KeyState};

use crate::keycodes::key_to_code;

mod json;
mod plain;

pub use json::{parse_json_control_command, parse_json_control_line};
pub use plain::parse_control_command;

#[derive(Debug, Clone, PartialEq)]
pub struct ControlCommand {
    pub window: Option<u64>,
    pub variant: ControlCommandVariant,
}

impl ControlCommand {
    pub fn new(variant: ControlCommandVariant) -> Self {
        Self {
            window: None,
            variant,
        }
    }

    pub fn targeted(window: Option<u64>, variant: ControlCommandVariant) -> Result<Self, String> {
        match window {
            Some(0) => Err("window id must be greater than zero".into()),
            Some(window) => Ok(Self {
                window: Some(window),
                variant,
            }),
            None => Ok(Self::new(variant)),
        }
    }
}

impl PartialEq<ControlCommandVariant> for ControlCommand {
    fn eq(&self, other: &ControlCommandVariant) -> bool {
        self.window.is_none() && self.variant == *other
    }
}

impl PartialEq<ControlCommand> for ControlCommandVariant {
    fn eq(&self, other: &ControlCommand) -> bool {
        other == self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommandVariant {
    Key { code: u32, action: PressAction },
    Chord { codes: Vec<u32> },
    Text(String),
    PointerMove { x: f64, y: f64 },
    PointerButton { button: u32, action: PressAction },
    Click { x: f64, y: f64, button: u32 },
    Scroll { dx: f64, dy: f64 },
    Screenshot { path: Option<PathBuf> },
    Sleep { duration_ms: u64 },
    Launch { command: Vec<OsString> },
    List,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressAction {
    Down,
    Up,
    Press,
}

impl PressAction {
    pub fn key_states(self) -> &'static [KeyState] {
        match self {
            Self::Down => &[KeyState::Pressed],
            Self::Up => &[KeyState::Released],
            Self::Press => &[KeyState::Pressed, KeyState::Released],
        }
    }

    pub fn button_states(self) -> &'static [ButtonState] {
        match self {
            Self::Down => &[ButtonState::Pressed],
            Self::Up => &[ButtonState::Released],
            Self::Press => &[ButtonState::Pressed, ButtonState::Released],
        }
    }
}

pub fn text_to_key_events(text: &str) -> Result<Vec<(u32, PressAction)>, String> {
    let mut events = Vec::new();
    let shift = key_to_code("ShiftLeft").unwrap();

    for ch in text.chars() {
        let Some((code, shifted)) = char_to_key(ch) else {
            return Err(format!("unsupported text character: {ch:?}"));
        };

        if shifted {
            events.push((shift, PressAction::Down));
        }
        events.push((code, PressAction::Press));
        if shifted {
            events.push((shift, PressAction::Up));
        }
    }

    Ok(events)
}

fn parse_press_action(value: Option<&str>) -> Result<PressAction, String> {
    match value {
        Some("down") => Ok(PressAction::Down),
        Some("up") => Ok(PressAction::Up),
        Some("press") => Ok(PressAction::Press),
        _ => Err("expected action: down, up, or press".into()),
    }
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

fn parse_button(value: Option<&str>) -> Result<u32, String> {
    match value {
        Some("left") => Ok(BTN_LEFT),
        Some("right") => Ok(BTN_RIGHT),
        Some("middle") => Ok(BTN_MIDDLE),
        Some(value) => value
            .parse::<u32>()
            .map_err(|_| format!("unknown button: {value}")),
        None => Err("expected button".into()),
    }
}

fn parse_f64(value: Option<&str>, name: &str) -> Result<f64, String> {
    value
        .ok_or_else(|| format!("expected {name}"))?
        .parse::<f64>()
        .map_err(|_| format!("invalid {name}"))
}

fn ensure_no_extra<'a>(mut parts: impl Iterator<Item = &'a str>) -> Result<(), String> {
    if let Some(extra) = parts.next() {
        Err(format!("unexpected argument: {extra}"))
    } else {
        Ok(())
    }
}

fn char_to_key(ch: char) -> Option<(u32, bool)> {
    let (key, shifted) = match ch {
        'a'..='z' => {
            return key_to_code(&format!("Key{}", ch.to_ascii_uppercase()))
                .map(|code| (code, false))
        }
        'A'..='Z' => return key_to_code(&format!("Key{ch}")).map(|code| (code, true)),
        '0' => ("Digit0", false),
        '1' => ("Digit1", false),
        '2' => ("Digit2", false),
        '3' => ("Digit3", false),
        '4' => ("Digit4", false),
        '5' => ("Digit5", false),
        '6' => ("Digit6", false),
        '7' => ("Digit7", false),
        '8' => ("Digit8", false),
        '9' => ("Digit9", false),
        ' ' => ("Space", false),
        '\n' => ("Enter", false),
        '\t' => ("Tab", false),
        '-' => ("Minus", false),
        '_' => ("Minus", true),
        '=' => ("Equal", false),
        '+' => ("Equal", true),
        '[' => ("BracketLeft", false),
        '{' => ("BracketLeft", true),
        ']' => ("BracketRight", false),
        '}' => ("BracketRight", true),
        '\\' => ("Backslash", false),
        '|' => ("Backslash", true),
        ';' => ("Semicolon", false),
        ':' => ("Semicolon", true),
        '\'' => ("Quote", false),
        '"' => ("Quote", true),
        '`' => ("Backquote", false),
        '~' => ("Backquote", true),
        ',' => ("Comma", false),
        '<' => ("Comma", true),
        '.' => ("Period", false),
        '>' => ("Period", true),
        '/' => ("Slash", false),
        '?' => ("Slash", true),
        '!' => ("Digit1", true),
        '@' => ("Digit2", true),
        '#' => ("Digit3", true),
        '$' => ("Digit4", true),
        '%' => ("Digit5", true),
        '^' => ("Digit6", true),
        '&' => ("Digit7", true),
        '*' => ("Digit8", true),
        '(' => ("Digit9", true),
        ')' => ("Digit0", true),
        _ => return None,
    };

    key_to_code(key).map(|code| (code, shifted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(variant: ControlCommandVariant) -> Option<ControlCommand> {
        Some(ControlCommand::new(variant))
    }

    fn targeted_command(window: u64, variant: ControlCommandVariant) -> Option<ControlCommand> {
        Some(ControlCommand {
            window: Some(window),
            variant,
        })
    }

    #[test]
    fn parses_key_command() {
        assert_eq!(
            parse_control_command("key KeyA").unwrap(),
            command(ControlCommandVariant::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Press,
            })
        );
        assert_eq!(
            parse_control_command("key KeyA down").unwrap(),
            command(ControlCommandVariant::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Down,
            })
        );
    }

    #[test]
    fn parses_chord() {
        assert_eq!(
            parse_control_command("chord ControlLeft KeyL").unwrap(),
            command(ControlCommandVariant::Chord {
                codes: vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyL").unwrap(),
                ],
            })
        );
        assert!(parse_control_command("chord").is_err());
        assert!(parse_control_command("chord KeyNope").is_err());
    }

    #[test]
    fn parses_mouse_commands() {
        assert_eq!(
            parse_control_command("mouse move 10 20").unwrap(),
            command(ControlCommandVariant::PointerMove { x: 10.0, y: 20.0 })
        );
        assert_eq!(
            parse_control_command("mouse button").unwrap(),
            command(ControlCommandVariant::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Press,
            })
        );
        assert_eq!(
            parse_control_command("mouse button down").unwrap(),
            command(ControlCommandVariant::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Down,
            })
        );
        assert_eq!(
            parse_control_command("mouse button up right").unwrap(),
            command(ControlCommandVariant::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Up,
            })
        );
        assert_eq!(
            parse_control_command("mouse button middle").unwrap(),
            command(ControlCommandVariant::PointerButton {
                button: BTN_MIDDLE,
                action: PressAction::Press,
            })
        );
    }

    #[test]
    fn parses_click_scroll_and_quit() {
        assert_eq!(
            parse_control_command("click 640 360").unwrap(),
            command(ControlCommandVariant::Click {
                x: 640.0,
                y: 360.0,
                button: BTN_LEFT,
            })
        );
        assert_eq!(
            parse_control_command("scroll 0 -120").unwrap(),
            command(ControlCommandVariant::Scroll {
                dx: 0.0,
                dy: -120.0,
            })
        );
        assert_eq!(
            parse_control_command("quit").unwrap(),
            command(ControlCommandVariant::Quit)
        );
    }

    #[test]
    fn parses_screenshot() {
        assert_eq!(
            parse_control_command("screenshot").unwrap(),
            command(ControlCommandVariant::Screenshot { path: None })
        );
        assert_eq!(
            parse_control_command("screenshot /tmp/autowc.png").unwrap(),
            command(ControlCommandVariant::Screenshot {
                path: Some(PathBuf::from("/tmp/autowc.png")),
            })
        );
    }

    #[test]
    fn parses_sleep() {
        assert_eq!(
            parse_control_command("sleep 250").unwrap(),
            command(ControlCommandVariant::Sleep { duration_ms: 250 })
        );
        assert!(parse_control_command("sleep 1.5").is_err());
        assert!(parse_control_command("sleep -1").is_err());
    }

    #[test]
    fn parses_launch() {
        assert_eq!(
            parse_control_command("launch gtk4-demo --run entry").unwrap(),
            command(ControlCommandVariant::Launch {
                command: vec!["gtk4-demo".into(), "--run".into(), "entry".into(),],
            })
        );
        assert!(parse_control_command("launch").is_err());
    }

    #[test]
    fn parses_list() {
        assert_eq!(
            parse_control_command("list").unwrap(),
            command(ControlCommandVariant::List)
        );
        assert!(parse_control_command("list extra").is_err());
    }

    #[test]
    fn parses_text_with_spaces() {
        assert_eq!(
            parse_control_command("text hello world").unwrap(),
            command(ControlCommandVariant::Text("hello world".to_string()))
        );
    }

    #[test]
    fn parses_plain_window_prefix() {
        assert_eq!(
            parse_control_command("2 text second window").unwrap(),
            targeted_command(2, ControlCommandVariant::Text("second window".to_string()))
        );
        assert_eq!(
            parse_control_command("3 key KeyA").unwrap(),
            targeted_command(
                3,
                ControlCommandVariant::Key {
                    code: key_to_code("KeyA").unwrap(),
                    action: PressAction::Press,
                }
            )
        );
        assert!(parse_control_command("0 text bad").is_err());
        assert!(parse_control_command("2").is_err());
    }

    #[test]
    fn plain_text_does_not_unescape_backslash_sequences() {
        assert_eq!(
            parse_control_command(r"text \n").unwrap(),
            command(ControlCommandVariant::Text(r"\n".to_string()))
        );
        assert_eq!(
            text_to_key_events(r"\n").unwrap(),
            [
                (key_to_code("Backslash").unwrap(), PressAction::Press),
                (key_to_code("KeyN").unwrap(), PressAction::Press),
            ]
        );
    }

    #[test]
    fn json_text_unescapes_backslash_sequences() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"text","text":"\n"}"#).unwrap(),
            ControlCommandVariant::Text("\n".to_string())
        );
        assert_eq!(
            text_to_key_events("\n").unwrap(),
            [(key_to_code("Enter").unwrap(), PressAction::Press)]
        );
    }

    #[test]
    fn text_tabs_map_to_tab() {
        assert_eq!(
            text_to_key_events("\t").unwrap(),
            [(key_to_code("Tab").unwrap(), PressAction::Press)]
        );
    }

    #[test]
    fn rejects_invalid_input() {
        assert!(parse_control_command("KeyA").is_err());
        assert!(parse_control_command("key KeyNope press").is_err());
        assert!(parse_control_command("key KeyA tap").is_err());
        assert!(parse_control_command("pointer move 10 20").is_err());
        assert!(parse_control_command("mouse move 10").is_err());
        assert!(parse_control_command("mouse button tap").is_err());
    }

    #[test]
    fn parses_json_key_commands() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"key","key":"KeyA"}"#).unwrap(),
            ControlCommandVariant::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Press,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"key","key":"KeyA","action":"down"}"#).unwrap(),
            ControlCommandVariant::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Down,
            }
        );
    }

    #[test]
    fn parses_json_chord_and_text_commands() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"chord","keys":["ControlLeft","KeyL"]}"#)
                .unwrap(),
            ControlCommandVariant::Chord {
                codes: vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyL").unwrap(),
                ],
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"text","text":" hello\nworld "}"#).unwrap(),
            ControlCommandVariant::Text(" hello\nworld ".to_string())
        );
    }

    #[test]
    fn parses_json_mouse_commands() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_move","x":10,"y":20}"#).unwrap(),
            ControlCommandVariant::PointerMove { x: 10.0, y: 20.0 }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button"}"#).unwrap(),
            ControlCommandVariant::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Press,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button","action":"up","button":"right"}"#)
                .unwrap(),
            ControlCommandVariant::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Up,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button","button":273}"#).unwrap(),
            ControlCommandVariant::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Press,
            }
        );
    }

    #[test]
    fn parses_json_click_scroll_sleep_screenshot_and_quit() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"click","x":640,"y":360}"#).unwrap(),
            ControlCommandVariant::Click {
                x: 640.0,
                y: 360.0,
                button: BTN_LEFT,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"scroll","dx":0,"dy":-120}"#).unwrap(),
            ControlCommandVariant::Scroll {
                dx: 0.0,
                dy: -120.0,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"sleep","ms":250}"#).unwrap(),
            ControlCommandVariant::Sleep { duration_ms: 250 }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"launch","command":["gtk4-demo"]}"#).unwrap(),
            ControlCommandVariant::Launch {
                command: vec!["gtk4-demo".into()],
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"list"}"#).unwrap(),
            ControlCommandVariant::List
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"screenshot","path":"/tmp/autowc.png"}"#)
                .unwrap(),
            ControlCommandVariant::Screenshot {
                path: Some(PathBuf::from("/tmp/autowc.png")),
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"quit"}"#).unwrap(),
            ControlCommandVariant::Quit
        );
    }

    #[test]
    fn parses_json_targeted_window_commands() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"text","window":2,"text":"second"}"#).unwrap(),
            ControlCommand {
                window: Some(2),
                variant: ControlCommandVariant::Text("second".to_string()),
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"screenshot","window":3}"#).unwrap(),
            ControlCommand {
                window: Some(3),
                variant: ControlCommandVariant::Screenshot { path: None },
            }
        );
        assert!(parse_json_control_command(r#"{"type":"text","window":0,"text":"bad"}"#).is_err());
    }

    #[test]
    fn rejects_invalid_json_input() {
        assert!(parse_json_control_command(r#"{"type":"key","key":"KeyNope"}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"chord","keys":[]}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"launch","command":[]}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"mouse_button","action":"tap"}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"sleep","ms":-1}"#).is_err());
    }
}
