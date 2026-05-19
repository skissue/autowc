use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationCommand {
    #[schemars(description = "Send a keyboard key event.")]
    Key {
        #[schemars(description = "Physical key name using the W3C KeyboardEvent.code scheme.")]
        key: String,
        #[schemars(description = "Key transition to send. Defaults to press.")]
        #[serde(default)]
        state: KeyState,
    },
    #[schemars(description = "Press and release multiple keyboard keys together.")]
    Chord {
        #[schemars(description = "Physical key names using the W3C KeyboardEvent.code scheme.")]
        keys: Vec<String>,
    },
    #[schemars(description = "Type literal text.")]
    Text {
        #[schemars(
            description = "Text to type. Newline characters are currently converted to Enter key presses."
        )]
        text: String,
    },
    #[schemars(description = "Move the mouse pointer.")]
    MouseMove {
        #[schemars(description = "Virtual-display x coordinate in pixels.")]
        x: f64,
        #[schemars(description = "Virtual-display y coordinate in pixels.")]
        y: f64,
    },
    #[schemars(description = "Send a mouse button event.")]
    MouseButton {
        #[schemars(description = "Mouse button transition to send. Defaults to press.")]
        #[serde(default)]
        state: MouseButtonState,
        #[schemars(description = "Mouse button to send. Defaults to left.")]
        #[serde(default)]
        button: MouseButton,
    },
    #[schemars(description = "Move the mouse pointer, then press and release a mouse button.")]
    Click {
        #[schemars(description = "Virtual-display x coordinate in pixels.")]
        x: f64,
        #[schemars(description = "Virtual-display y coordinate in pixels.")]
        y: f64,
        #[schemars(description = "Mouse button to click. Defaults to left.")]
        #[serde(default)]
        button: MouseButton,
    },
    #[schemars(description = "Send a mouse wheel scroll event.")]
    Scroll {
        #[schemars(description = "Horizontal scroll amount in wheel units.")]
        dx: f64,
        #[schemars(description = "Vertical scroll amount in wheel units.")]
        dy: f64,
    },
    #[schemars(description = "Pause before continuing the batch.")]
    Sleep {
        #[schemars(description = "Sleep duration in whole milliseconds.")]
        ms: u64,
    },
}

impl AutomationCommand {
    pub fn to_autowc_lines(&self) -> Result<Vec<String>, String> {
        match self {
            Self::Key { key, state } => {
                if key.trim().is_empty() || key.split_whitespace().count() != 1 {
                    return Err("key must be one non-empty token".into());
                }
                Ok(vec![format!("key {key} {state}")])
            }
            Self::Chord { keys } => {
                if keys.is_empty() {
                    return Err("chord requires at least one key".into());
                }
                for key in keys {
                    if key.trim().is_empty() || key.split_whitespace().count() != 1 {
                        return Err("chord keys must be non-empty tokens".into());
                    }
                }
                Ok(vec![format!("chord {}", keys.join(" "))])
            }
            Self::Text { text } => Ok(text_to_autowc_lines(text)),
            Self::MouseMove { x, y } => Ok(vec![format!("mouse move {x} {y}")]),
            Self::MouseButton { state, button } => {
                Ok(vec![format!("mouse button {state} {}", button.as_autowc())])
            }
            Self::Click { x, y, button } => {
                Ok(vec![format!("click {x} {y} {}", button.as_autowc())])
            }
            Self::Scroll { dx, dy } => Ok(vec![format!("scroll {dx} {dy}")]),
            Self::Sleep { ms } => Ok(vec![format!("sleep {ms}")]),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    Down,
    Up,
    #[default]
    Press,
}

impl std::fmt::Display for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Down => f.write_str("down"),
            Self::Up => f.write_str("up"),
            Self::Press => f.write_str("press"),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButtonState {
    Down,
    Up,
    #[default]
    Press,
}

impl std::fmt::Display for MouseButtonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Down => f.write_str("down"),
            Self::Up => f.write_str("up"),
            Self::Press => f.write_str("press"),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum MouseButton {
    Named(NamedMouseButton),
    Other(u32),
}

impl Default for MouseButton {
    fn default() -> Self {
        Self::Named(NamedMouseButton::Left)
    }
}

impl MouseButton {
    fn as_autowc(self) -> String {
        match self {
            Self::Named(NamedMouseButton::Left) => "left".into(),
            Self::Named(NamedMouseButton::Right) => "right".into(),
            Self::Named(NamedMouseButton::Middle) => "middle".into(),
            Self::Other(button) => button.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NamedMouseButton {
    Left,
    Right,
    Middle,
}

pub fn screenshot_line(path: Option<&Path>) -> Result<String, String> {
    let Some(path) = path else {
        return Ok("screenshot".into());
    };
    let path = path
        .to_str()
        .ok_or_else(|| "screenshot path must be valid UTF-8".to_string())?;
    if path.split_whitespace().count() != 1 {
        return Err("screenshot path cannot contain whitespace".into());
    }
    Ok(format!("screenshot {path}"))
}

fn text_to_autowc_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;

    while let Some((line, tail)) = rest.split_once('\n') {
        if !line.is_empty() {
            lines.push(format!("text {line}"));
        }
        lines.push("key Enter press".into());
        rest = tail;
    }

    if !rest.is_empty() {
        lines.push(format!("text {rest}"));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_default_key_press() {
        let command = AutomationCommand::Key {
            key: "KeyA".into(),
            state: KeyState::default(),
        };

        assert_eq!(command.to_autowc_lines().unwrap(), ["key KeyA press"]);
    }

    #[test]
    fn serializes_mouse_defaults() {
        let command = AutomationCommand::MouseButton {
            state: MouseButtonState::default(),
            button: MouseButton::default(),
        };

        assert_eq!(
            command.to_autowc_lines().unwrap(),
            ["mouse button press left"]
        );
    }

    #[test]
    fn deserializes_named_and_numbered_mouse_buttons() {
        let named: MouseButton = serde_json::from_str("\"right\"").unwrap();
        let numbered: MouseButton = serde_json::from_str("273").unwrap();

        assert_eq!(named.as_autowc(), "right");
        assert_eq!(numbered.as_autowc(), "273");
    }

    #[test]
    fn serializes_text_with_newlines() {
        let command = AutomationCommand::Text {
            text: "hello\nworld".into(),
        };

        assert_eq!(
            command.to_autowc_lines().unwrap(),
            ["text hello", "key Enter press", "text world"]
        );
    }

    #[test]
    fn rejects_bad_tokens() {
        assert!(AutomationCommand::Key {
            key: "KeyA KeyB".into(),
            state: KeyState::Press,
        }
        .to_autowc_lines()
        .is_err());
        assert!(AutomationCommand::Chord { keys: vec![] }
            .to_autowc_lines()
            .is_err());
    }

    #[test]
    fn rejects_screenshot_paths_with_whitespace() {
        assert!(screenshot_line(Some(Path::new("/tmp/has space.png"))).is_err());
    }

    #[test]
    fn command_schema_documents_agent_visible_fields() {
        let schema = schemars::schema_for!(AutomationCommand);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("W3C KeyboardEvent.code"));
        assert!(schema.contains("Newline characters are currently converted to Enter"));
        assert!(schema.contains("Virtual-display x coordinate"));
        assert!(schema.contains("Sleep duration in whole milliseconds"));
    }
}
