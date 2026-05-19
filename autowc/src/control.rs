use std::path::PathBuf;

use serde::Deserialize;
use smithay::backend::input::{ButtonState, KeyState};

use crate::keycodes::key_to_code;

#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    Key { code: u32, action: PressAction },
    Chord { codes: Vec<u32> },
    Text(String),
    PointerMove { x: f64, y: f64 },
    PointerButton { button: u32, action: PressAction },
    Click { x: f64, y: f64, button: u32 },
    Scroll { dx: f64, dy: f64 },
    Screenshot { path: Option<PathBuf> },
    Sleep { duration_ms: u64 },
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

pub fn parse_control_command(line: &str) -> Result<Option<ControlCommand>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    if line == "quit" {
        return Ok(Some(ControlCommand::Quit));
    }

    if let Some(text) = line.strip_prefix("text") {
        let text = text
            .strip_prefix(char::is_whitespace)
            .ok_or_else(|| "usage: text <text>".to_string())?;
        return Ok(Some(ControlCommand::Text(text.to_string())));
    }

    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap();
    match command {
        "key" => parse_key(parts),
        "chord" => parse_chord(parts),
        "mouse" => parse_mouse(parts),
        "click" => parse_click(parts),
        "scroll" => parse_scroll(parts),
        "screenshot" => parse_screenshot(parts),
        "sleep" => parse_sleep(parts),
        _ => Err(format!("unknown command: {command}")),
    }
}

pub fn parse_json_control_line(line: &str) -> Result<Option<ControlCommand>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }

    parse_json_control_command(line).map(Some)
}

pub fn parse_json_control_command(line: &str) -> Result<ControlCommand, String> {
    let command = serde_json::from_str::<JsonControlCommand>(line)
        .map_err(|err| format!("invalid json command: {err}"))?;
    command.into_control_command()
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum JsonControlCommand {
    Key {
        key: String,
        #[serde(default = "default_json_press_action")]
        action: JsonPressAction,
    },
    Chord {
        keys: Vec<String>,
    },
    Text {
        text: String,
    },
    MouseMove {
        x: f64,
        y: f64,
    },
    MouseButton {
        #[serde(default = "default_json_press_action")]
        action: JsonPressAction,
        button: Option<JsonMouseButton>,
    },
    Click {
        x: f64,
        y: f64,
        button: Option<JsonMouseButton>,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    Screenshot {
        path: Option<PathBuf>,
    },
    Sleep {
        ms: u64,
    },
    Quit,
}

impl JsonControlCommand {
    fn into_control_command(self) -> Result<ControlCommand, String> {
        match self {
            Self::Key { key, action } => {
                let code = key_to_code(&key).ok_or_else(|| format!("unknown key: {key}"))?;
                Ok(ControlCommand::Key {
                    code,
                    action: action.into(),
                })
            }
            Self::Chord { keys } => {
                if keys.is_empty() {
                    return Err("chord requires at least one key".into());
                }

                let mut codes = Vec::with_capacity(keys.len());
                for key in keys {
                    codes.push(key_to_code(&key).ok_or_else(|| format!("unknown key: {key}"))?);
                }
                Ok(ControlCommand::Chord { codes })
            }
            Self::Text { text } => Ok(ControlCommand::Text(text)),
            Self::MouseMove { x, y } => Ok(ControlCommand::PointerMove { x, y }),
            Self::MouseButton { action, button } => Ok(ControlCommand::PointerButton {
                button: parse_json_button(button)?,
                action: action.into(),
            }),
            Self::Click { x, y, button } => Ok(ControlCommand::Click {
                x,
                y,
                button: parse_json_button(button)?,
            }),
            Self::Scroll { dx, dy } => Ok(ControlCommand::Scroll { dx, dy }),
            Self::Screenshot { path } => Ok(ControlCommand::Screenshot { path }),
            Self::Sleep { ms } => Ok(ControlCommand::Sleep { duration_ms: ms }),
            Self::Quit => Ok(ControlCommand::Quit),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JsonPressAction {
    Down,
    Up,
    Press,
}

impl From<JsonPressAction> for PressAction {
    fn from(action: JsonPressAction) -> Self {
        match action {
            JsonPressAction::Down => Self::Down,
            JsonPressAction::Up => Self::Up,
            JsonPressAction::Press => Self::Press,
        }
    }
}

fn default_json_press_action() -> JsonPressAction {
    JsonPressAction::Press
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonMouseButton {
    Name(String),
    Code(u32),
}

fn parse_json_button(button: Option<JsonMouseButton>) -> Result<u32, String> {
    match button {
        Some(JsonMouseButton::Name(button)) => parse_button(Some(&button)),
        Some(JsonMouseButton::Code(button)) => Ok(button),
        None => Ok(BTN_LEFT),
    }
}

fn parse_key<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let key = parts
        .next()
        .ok_or_else(|| "usage: key <KeyboardEvent.code> [down|up|press]".to_string())?;
    let action = match parts.next() {
        Some(action) => parse_press_action(Some(action))?,
        None => PressAction::Press,
    };
    ensure_no_extra(parts)?;

    let code = key_to_code(key).ok_or_else(|| format!("unknown key: {key}"))?;
    Ok(Some(ControlCommand::Key { code, action }))
}

fn parse_chord<'a>(parts: impl Iterator<Item = &'a str>) -> Result<Option<ControlCommand>, String> {
    let keys = parts.collect::<Vec<_>>();
    if keys.is_empty() {
        return Err("usage: chord <KeyboardEvent.code> [KeyboardEvent.code ...]".into());
    }

    let mut codes = Vec::with_capacity(keys.len());
    for key in keys {
        codes.push(key_to_code(key).ok_or_else(|| format!("unknown key: {key}"))?);
    }

    Ok(Some(ControlCommand::Chord { codes }))
}

fn parse_mouse<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    match parts.next() {
        Some("move") => {
            let x = parse_f64(parts.next(), "x")?;
            let y = parse_f64(parts.next(), "y")?;
            ensure_no_extra(parts)?;
            Ok(Some(ControlCommand::PointerMove { x, y }))
        }
        Some("button") => {
            let action = match parts.next() {
                Some(action @ ("down" | "up" | "press")) => parse_press_action(Some(action))?,
                Some(button) => {
                    let parsed_button = parse_button(Some(button))?;
                    ensure_no_extra(parts)?;
                    return Ok(Some(ControlCommand::PointerButton {
                        button: parsed_button,
                        action: PressAction::Press,
                    }));
                }
                None => PressAction::Press,
            };
            let button = match parts.next() {
                Some(button) => parse_button(Some(button))?,
                None => BTN_LEFT,
            };
            ensure_no_extra(parts)?;
            Ok(Some(ControlCommand::PointerButton { button, action }))
        }
        _ => Err("usage: mouse move <x> <y> | mouse button [down|up|press] [button]".into()),
    }
}

fn parse_click<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let x = parse_f64(parts.next(), "x")?;
    let y = parse_f64(parts.next(), "y")?;
    let button = match parts.next() {
        Some(button) => parse_button(Some(button))?,
        None => BTN_LEFT,
    };
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::Click { x, y, button }))
}

fn parse_scroll<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let dx = parse_f64(parts.next(), "dx")?;
    let dy = parse_f64(parts.next(), "dy")?;
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::Scroll { dx, dy }))
}

fn parse_screenshot<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let path = parts.next().map(PathBuf::from);
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::Screenshot { path }))
}

fn parse_sleep<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let duration_ms = parts
        .next()
        .ok_or_else(|| "usage: sleep <ms>".to_string())?
        .parse::<u64>()
        .map_err(|_| "invalid sleep duration".to_string())?;
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::Sleep { duration_ms }))
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

    #[test]
    fn parses_key_command() {
        assert_eq!(
            parse_control_command("key KeyA").unwrap(),
            Some(ControlCommand::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Press,
            })
        );
        assert_eq!(
            parse_control_command("key KeyA down").unwrap(),
            Some(ControlCommand::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Down,
            })
        );
    }

    #[test]
    fn parses_chord() {
        assert_eq!(
            parse_control_command("chord ControlLeft KeyL").unwrap(),
            Some(ControlCommand::Chord {
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
            Some(ControlCommand::PointerMove { x: 10.0, y: 20.0 })
        );
        assert_eq!(
            parse_control_command("mouse button").unwrap(),
            Some(ControlCommand::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Press,
            })
        );
        assert_eq!(
            parse_control_command("mouse button down").unwrap(),
            Some(ControlCommand::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Down,
            })
        );
        assert_eq!(
            parse_control_command("mouse button up right").unwrap(),
            Some(ControlCommand::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Up,
            })
        );
        assert_eq!(
            parse_control_command("mouse button middle").unwrap(),
            Some(ControlCommand::PointerButton {
                button: BTN_MIDDLE,
                action: PressAction::Press,
            })
        );
    }

    #[test]
    fn parses_click_scroll_and_quit() {
        assert_eq!(
            parse_control_command("click 640 360").unwrap(),
            Some(ControlCommand::Click {
                x: 640.0,
                y: 360.0,
                button: BTN_LEFT,
            })
        );
        assert_eq!(
            parse_control_command("scroll 0 -120").unwrap(),
            Some(ControlCommand::Scroll {
                dx: 0.0,
                dy: -120.0,
            })
        );
        assert_eq!(
            parse_control_command("quit").unwrap(),
            Some(ControlCommand::Quit)
        );
    }

    #[test]
    fn parses_screenshot() {
        assert_eq!(
            parse_control_command("screenshot").unwrap(),
            Some(ControlCommand::Screenshot { path: None })
        );
        assert_eq!(
            parse_control_command("screenshot /tmp/autowc.png").unwrap(),
            Some(ControlCommand::Screenshot {
                path: Some(PathBuf::from("/tmp/autowc.png")),
            })
        );
    }

    #[test]
    fn parses_sleep() {
        assert_eq!(
            parse_control_command("sleep 250").unwrap(),
            Some(ControlCommand::Sleep { duration_ms: 250 })
        );
        assert!(parse_control_command("sleep 1.5").is_err());
        assert!(parse_control_command("sleep -1").is_err());
    }

    #[test]
    fn parses_text_with_spaces() {
        assert_eq!(
            parse_control_command("text hello world").unwrap(),
            Some(ControlCommand::Text("hello world".to_string()))
        );
    }

    #[test]
    fn plain_text_does_not_unescape_backslash_sequences() {
        assert_eq!(
            parse_control_command(r"text \n").unwrap(),
            Some(ControlCommand::Text(r"\n".to_string()))
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
            ControlCommand::Text("\n".to_string())
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
            ControlCommand::Key {
                code: key_to_code("KeyA").unwrap(),
                action: PressAction::Press,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"key","key":"KeyA","action":"down"}"#).unwrap(),
            ControlCommand::Key {
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
            ControlCommand::Chord {
                codes: vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyL").unwrap(),
                ],
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"text","text":" hello\nworld "}"#).unwrap(),
            ControlCommand::Text(" hello\nworld ".to_string())
        );
    }

    #[test]
    fn parses_json_mouse_commands() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_move","x":10,"y":20}"#).unwrap(),
            ControlCommand::PointerMove { x: 10.0, y: 20.0 }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button"}"#).unwrap(),
            ControlCommand::PointerButton {
                button: BTN_LEFT,
                action: PressAction::Press,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button","action":"up","button":"right"}"#)
                .unwrap(),
            ControlCommand::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Up,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"mouse_button","button":273}"#).unwrap(),
            ControlCommand::PointerButton {
                button: BTN_RIGHT,
                action: PressAction::Press,
            }
        );
    }

    #[test]
    fn parses_json_click_scroll_sleep_screenshot_and_quit() {
        assert_eq!(
            parse_json_control_command(r#"{"type":"click","x":640,"y":360}"#).unwrap(),
            ControlCommand::Click {
                x: 640.0,
                y: 360.0,
                button: BTN_LEFT,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"scroll","dx":0,"dy":-120}"#).unwrap(),
            ControlCommand::Scroll {
                dx: 0.0,
                dy: -120.0,
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"sleep","ms":250}"#).unwrap(),
            ControlCommand::Sleep { duration_ms: 250 }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"screenshot","path":"/tmp/autowc.png"}"#)
                .unwrap(),
            ControlCommand::Screenshot {
                path: Some(PathBuf::from("/tmp/autowc.png")),
            }
        );
        assert_eq!(
            parse_json_control_command(r#"{"type":"quit"}"#).unwrap(),
            ControlCommand::Quit
        );
    }

    #[test]
    fn rejects_invalid_json_input() {
        assert!(parse_json_control_command(r#"{"type":"key","key":"KeyNope"}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"chord","keys":[]}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"mouse_button","action":"tap"}"#).is_err());
        assert!(parse_json_control_command(r#"{"type":"sleep","ms":-1}"#).is_err());
    }
}
