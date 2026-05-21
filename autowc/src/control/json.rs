use std::path::PathBuf;

use serde::Deserialize;

use crate::keycodes::key_to_code;

use super::{parse_button, ControlCommand, ControlCommandVariant, PressAction, BTN_LEFT};

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

#[derive(Debug, Deserialize)]
struct JsonControlCommand {
    window: Option<u64>,
    #[serde(flatten)]
    variant: JsonControlCommandVariant,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum JsonControlCommandVariant {
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
    Launch {
        command: Vec<String>,
    },
    List,
    Quit,
}

impl JsonControlCommand {
    fn into_control_command(self) -> Result<ControlCommand, String> {
        let variant = match self.variant {
            JsonControlCommandVariant::Key { key, action } => {
                let code = key_to_code(&key).ok_or_else(|| format!("unknown key: {key}"))?;
                ControlCommandVariant::Key {
                    code,
                    action: action.into(),
                }
            }
            JsonControlCommandVariant::Chord { keys } => {
                if keys.is_empty() {
                    return Err("chord requires at least one key".into());
                }

                let mut codes = Vec::with_capacity(keys.len());
                for key in keys {
                    codes.push(key_to_code(&key).ok_or_else(|| format!("unknown key: {key}"))?);
                }
                ControlCommandVariant::Chord { codes }
            }
            JsonControlCommandVariant::Text { text } => ControlCommandVariant::Text(text),
            JsonControlCommandVariant::MouseMove { x, y } => {
                ControlCommandVariant::PointerMove { x, y }
            }
            JsonControlCommandVariant::MouseButton { action, button } => {
                ControlCommandVariant::PointerButton {
                    button: parse_json_button(button)?,
                    action: action.into(),
                }
            }
            JsonControlCommandVariant::Click { x, y, button } => ControlCommandVariant::Click {
                x,
                y,
                button: parse_json_button(button)?,
            },
            JsonControlCommandVariant::Scroll { dx, dy } => {
                ControlCommandVariant::Scroll { dx, dy }
            }
            JsonControlCommandVariant::Screenshot { path } => {
                ControlCommandVariant::Screenshot { path }
            }
            JsonControlCommandVariant::Sleep { ms } => {
                ControlCommandVariant::Sleep { duration_ms: ms }
            }
            JsonControlCommandVariant::Launch { command } => {
                if command.is_empty() {
                    return Err("launch requires at least one command argument".into());
                }
                ControlCommandVariant::Launch { command }
            }
            JsonControlCommandVariant::List => ControlCommandVariant::List,
            JsonControlCommandVariant::Quit => ControlCommandVariant::Quit,
        };
        ControlCommand::targeted(self.window, variant)
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
