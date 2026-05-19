use std::path::PathBuf;

use smithay::backend::input::{ButtonState, KeyState};

use crate::keycodes::key_to_code;

#[derive(Debug, Clone, PartialEq)]
pub enum ControlCommand {
    Key { code: u32, action: PressAction },
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
        "mouse" => parse_mouse(parts),
        "click" => parse_click(parts),
        "scroll" => parse_scroll(parts),
        "screenshot" => parse_screenshot(parts),
        "sleep" => parse_sleep(parts),
        _ => Err(format!("unknown command: {command}")),
    }
}

pub fn text_to_key_events(text: &str) -> Result<Vec<(u32, PressAction)>, String> {
    let mut events = Vec::new();
    let shift = key_to_code("KEY_LEFTSHIFT").unwrap();

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

fn parse_key<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let key = parts
        .next()
        .ok_or_else(|| "usage: key <KEY_NAME> [down|up|press]".to_string())?;
    let action = match parts.next() {
        Some(action) => parse_press_action(Some(action))?,
        None => PressAction::Press,
    };
    ensure_no_extra(parts)?;

    let code = key_to_code(key).ok_or_else(|| format!("unknown key: {key}"))?;
    Ok(Some(ControlCommand::Key { code, action }))
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
            return key_to_code(&format!("KEY_{}", ch.to_ascii_uppercase()))
                .map(|code| (code, false))
        }
        'A'..='Z' => return key_to_code(&format!("KEY_{ch}")).map(|code| (code, true)),
        '0' => ("KEY_0", false),
        '1' => ("KEY_1", false),
        '2' => ("KEY_2", false),
        '3' => ("KEY_3", false),
        '4' => ("KEY_4", false),
        '5' => ("KEY_5", false),
        '6' => ("KEY_6", false),
        '7' => ("KEY_7", false),
        '8' => ("KEY_8", false),
        '9' => ("KEY_9", false),
        ' ' => ("KEY_SPACE", false),
        '\n' => ("KEY_ENTER", false),
        '-' => ("KEY_MINUS", false),
        '_' => ("KEY_MINUS", true),
        '=' => ("KEY_EQUAL", false),
        '+' => ("KEY_EQUAL", true),
        '[' => ("KEY_LEFTBRACE", false),
        '{' => ("KEY_LEFTBRACE", true),
        ']' => ("KEY_RIGHTBRACE", false),
        '}' => ("KEY_RIGHTBRACE", true),
        '\\' => ("KEY_BACKSLASH", false),
        '|' => ("KEY_BACKSLASH", true),
        ';' => ("KEY_SEMICOLON", false),
        ':' => ("KEY_SEMICOLON", true),
        '\'' => ("KEY_APOSTROPHE", false),
        '"' => ("KEY_APOSTROPHE", true),
        '`' => ("KEY_GRAVE", false),
        '~' => ("KEY_GRAVE", true),
        ',' => ("KEY_COMMA", false),
        '<' => ("KEY_COMMA", true),
        '.' => ("KEY_DOT", false),
        '>' => ("KEY_DOT", true),
        '/' => ("KEY_SLASH", false),
        '?' => ("KEY_SLASH", true),
        '!' => ("KEY_1", true),
        '@' => ("KEY_2", true),
        '#' => ("KEY_3", true),
        '$' => ("KEY_4", true),
        '%' => ("KEY_5", true),
        '^' => ("KEY_6", true),
        '&' => ("KEY_7", true),
        '*' => ("KEY_8", true),
        '(' => ("KEY_9", true),
        ')' => ("KEY_0", true),
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
            parse_control_command("key KEY_A").unwrap(),
            Some(ControlCommand::Key {
                code: key_to_code("KEY_A").unwrap(),
                action: PressAction::Press,
            })
        );
        assert_eq!(
            parse_control_command("key KEY_A down").unwrap(),
            Some(ControlCommand::Key {
                code: key_to_code("KEY_A").unwrap(),
                action: PressAction::Down,
            })
        );
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
    fn rejects_invalid_input() {
        assert!(parse_control_command("KEY_A").is_err());
        assert!(parse_control_command("key KEY_NOPE press").is_err());
        assert!(parse_control_command("key KEY_A tap").is_err());
        assert!(parse_control_command("pointer move 10 20").is_err());
        assert!(parse_control_command("mouse move 10").is_err());
        assert!(parse_control_command("mouse button tap").is_err());
    }
}
