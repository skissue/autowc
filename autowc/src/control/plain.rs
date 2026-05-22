use std::{ffi::OsString, path::PathBuf};

use crate::input::keyboard::{key_to_code, keys_sequence::parse_keys_sequence};

use super::{
    ensure_no_extra, parse_button, parse_f64, parse_press_action, ControlCommand,
    ControlCommandVariant, PressAction, BTN_LEFT,
};

pub fn parse_control_command(line: &str) -> Result<Option<ControlCommand>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    let (window, line) = parse_plain_window_prefix(line)?;

    if line == "quit" {
        return Ok(Some(ControlCommand::targeted(
            window,
            ControlCommandVariant::Quit,
        )?));
    }

    if let Some(text) = line.strip_prefix("text") {
        let text = text
            .strip_prefix(char::is_whitespace)
            .ok_or_else(|| "usage: text <text>".to_string())?;
        return Ok(Some(ControlCommand::targeted(
            window,
            ControlCommandVariant::Text(text.to_string()),
        )?));
    }

    if let Some(keys) = line.strip_prefix("keys") {
        let keys = keys
            .strip_prefix(char::is_whitespace)
            .ok_or_else(|| "usage: keys <sequence>".to_string())?;
        return Ok(Some(ControlCommand::targeted(
            window,
            ControlCommandVariant::KeysSequence {
                actions: parse_keys_sequence(keys)?,
            },
        )?));
    }

    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap();
    let command = match command {
        "key" => parse_key(parts),
        "chord" => parse_chord(parts),
        "mouse" => parse_mouse(parts),
        "click" => parse_click(parts),
        "scroll" => parse_scroll(parts),
        "screenshot" => parse_screenshot(parts),
        "sleep" => parse_sleep(parts),
        "launch" => parse_launch(parts),
        "list" => parse_list(parts),
        "close" => parse_close(parts),
        _ => Err(format!("unknown command: {command}")),
    }?;

    Ok(command
        .map(|command| ControlCommand::targeted(window, command.variant))
        .transpose()?)
}

fn parse_plain_window_prefix(line: &str) -> Result<(Option<u64>, &str), String> {
    let Some((first, rest)) = line.split_once(char::is_whitespace) else {
        return Ok((None, line));
    };
    let Ok(window) = first.parse::<u64>() else {
        return Ok((None, line));
    };
    if window == 0 {
        return Err("window id must be greater than zero".into());
    }
    Ok((Some(window), rest.trim_start()))
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
    Ok(Some(ControlCommand::new(ControlCommandVariant::Key {
        code,
        action,
    })))
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

    Ok(Some(ControlCommand::new(ControlCommandVariant::Chord {
        codes,
    })))
}

fn parse_mouse<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    match parts.next() {
        Some("move") => {
            let x = parse_f64(parts.next(), "x")?;
            let y = parse_f64(parts.next(), "y")?;
            ensure_no_extra(parts)?;
            Ok(Some(ControlCommand::new(
                ControlCommandVariant::PointerMove { x, y },
            )))
        }
        Some("button") => {
            let action = match parts.next() {
                Some(action @ ("down" | "up" | "press")) => parse_press_action(Some(action))?,
                Some(button) => {
                    let parsed_button = parse_button(Some(button))?;
                    ensure_no_extra(parts)?;
                    return Ok(Some(ControlCommand::new(
                        ControlCommandVariant::PointerButton {
                            button: parsed_button,
                            action: PressAction::Press,
                        },
                    )));
                }
                None => PressAction::Press,
            };
            let button = match parts.next() {
                Some(button) => parse_button(Some(button))?,
                None => BTN_LEFT,
            };
            ensure_no_extra(parts)?;
            Ok(Some(ControlCommand::new(
                ControlCommandVariant::PointerButton { button, action },
            )))
        }
        Some("drag") => {
            let start_x = parse_f64(parts.next(), "start_x")?;
            let start_y = parse_f64(parts.next(), "start_y")?;
            let end_x = parse_f64(parts.next(), "end_x")?;
            let end_y = parse_f64(parts.next(), "end_y")?;
            let button = match parts.next() {
                Some(button) => parse_button(Some(button))?,
                None => BTN_LEFT,
            };
            ensure_no_extra(parts)?;
            Ok(Some(ControlCommand::new(
                ControlCommandVariant::PointerDrag {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                    button,
                },
            )))
        }
        _ => Err(
            "usage: mouse move <x> <y> | mouse button [down|up|press] [button] | mouse drag <start_x> <start_y> <end_x> <end_y> [button]"
                .into(),
        ),
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

    Ok(Some(ControlCommand::new(ControlCommandVariant::Click {
        x,
        y,
        button,
    })))
}

fn parse_scroll<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let dx = parse_f64(parts.next(), "dx")?;
    let dy = parse_f64(parts.next(), "dy")?;
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::new(ControlCommandVariant::Scroll {
        dx,
        dy,
    })))
}

fn parse_screenshot<'a>(
    mut parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let path = parts.next().map(PathBuf::from);
    ensure_no_extra(parts)?;

    Ok(Some(ControlCommand::new(
        ControlCommandVariant::Screenshot { path },
    )))
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

    Ok(Some(ControlCommand::new(ControlCommandVariant::Sleep {
        duration_ms,
    })))
}

fn parse_launch<'a>(
    parts: impl Iterator<Item = &'a str>,
) -> Result<Option<ControlCommand>, String> {
    let command = parts.map(OsString::from).collect::<Vec<_>>();
    if command.is_empty() {
        return Err("usage: launch <command> [args...]".into());
    }

    Ok(Some(ControlCommand::new(ControlCommandVariant::Launch {
        command,
    })))
}

fn parse_list<'a>(parts: impl Iterator<Item = &'a str>) -> Result<Option<ControlCommand>, String> {
    ensure_no_extra(parts)?;
    Ok(Some(ControlCommand::new(ControlCommandVariant::List)))
}

fn parse_close<'a>(parts: impl Iterator<Item = &'a str>) -> Result<Option<ControlCommand>, String> {
    ensure_no_extra(parts)?;
    Ok(Some(ControlCommand::new(ControlCommandVariant::Close)))
}
