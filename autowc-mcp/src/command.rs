use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[schemars(inline)]
pub enum AutomationCommand {
    #[schemars(description = "Send a keyboard key event.")]
    Key {
        #[schemars(description = "Physical key name using the W3C KeyboardEvent.code scheme.")]
        key: String,
        #[schemars(description = "Key transition to send. Defaults to press.")]
        #[serde(default)]
        action: KeyAction,
    },
    #[schemars(description = "Press and release multiple keyboard keys together.")]
    Chord {
        #[schemars(description = "Physical key names using the W3C KeyboardEvent.code scheme.")]
        keys: Vec<String>,
    },
    #[schemars(
        description = "Send a compact keyboard sequence. Prefer this for keyboard automation because it can mix literal text, Emacs-style chords, special keys, and waits in one ordered command. Text outside angle tokens is typed literally. Angle tokens send keys, chords, or directives such as <C-l>, <RET>, <C-->, <C-M-->, and <w:500>."
    )]
    Keys {
        #[schemars(
            description = "Key sequence to send. Examples: '<C-l>example.org<RET>' focuses the address bar and visits a URL in a web browser; '<C-f>hello' opens find and types text; '<w:500>' waits 500 ms before continuing. Angle tokens use Emacs-style modifiers: C for Control, M for Alt/Meta, S for Shift, and lowercase s for Super. Common aliases include RET, ESC, TAB, SPC, BS, DEL, UP, DOWN, LEFT, and RIGHT. Use \\< to type a literal '<' and \\\\ before a token to type a literal backslash."
        )]
        keys: String,
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
        action: MouseButtonAction,
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
    #[schemars(
        description = "Send a mouse wheel scroll event. Amounts are wheel detents: 1.0 is one wheel notch, 0.5 is half a notch, and 3.0 is a medium scroll."
    )]
    Scroll {
        #[schemars(description = "Horizontal scroll amount in wheel detents.")]
        dx: f64,
        #[schemars(description = "Vertical scroll amount in wheel detents.")]
        dy: f64,
    },
    #[schemars(description = "Pause before continuing the batch.")]
    Sleep {
        #[schemars(description = "Sleep duration in whole milliseconds.")]
        ms: u64,
    },
}

impl AutomationCommand {
    pub fn to_autowc_line(&self, window: Option<u64>) -> Result<String, String> {
        let mut value = match self {
            Self::Key { key, action } => {
                if key.trim().is_empty() || key.split_whitespace().count() != 1 {
                    return Err("key must be one non-empty token".into());
                }
                serde_json::json!({
                    "type": "key",
                    "key": key,
                    "action": action,
                })
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
                serde_json::json!({
                    "type": "chord",
                    "keys": keys,
                })
            }
            Self::Keys { keys } => serde_json::json!({
                "type": "keys",
                "keys": keys,
            }),
            Self::MouseMove { x, y } => serde_json::json!({
                "type": "mouse_move",
                "x": x,
                "y": y,
            }),
            Self::MouseButton { action, button } => serde_json::json!({
                "type": "mouse_button",
                "action": action,
                "button": button,
            }),
            Self::Click { x, y, button } => serde_json::json!({
                "type": "click",
                "x": x,
                "y": y,
                "button": button,
            }),
            Self::Scroll { dx, dy } => serde_json::json!({
                "type": "scroll",
                "dx": dx,
                "dy": dy,
            }),
            Self::Sleep { ms } => serde_json::json!({
                "type": "sleep",
                "ms": ms,
            }),
        };

        if let Some(window) = window {
            value["window"] = serde_json::json!(window);
        }

        serde_json::to_string(&value).map_err(|err| err.to_string())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum KeyAction {
    Down,
    Up,
    #[default]
    Press,
}

impl std::fmt::Display for KeyAction {
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
#[schemars(inline)]
pub enum MouseButtonAction {
    Down,
    Up,
    #[default]
    Press,
}

impl std::fmt::Display for MouseButtonAction {
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
#[schemars(inline)]
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
    #[cfg(test)]
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
#[schemars(inline)]
pub enum NamedMouseButton {
    Left,
    Right,
    Middle,
}

pub fn screenshot_line(path: Option<&Path>, window: Option<u64>) -> Result<String, String> {
    let mut value = if let Some(path) = path {
        let path = path
            .to_str()
            .ok_or_else(|| "screenshot path must be valid UTF-8".to_string())?;
        serde_json::json!({
            "type": "screenshot",
            "path": path,
        })
    } else {
        serde_json::json!({
            "type": "screenshot",
        })
    };

    if let Some(window) = window {
        value["window"] = serde_json::json!(window);
    }

    serde_json::to_string(&value).map_err(|err| err.to_string())
}

pub fn list_line() -> String {
    r#"{"type":"list"}"#.to_string()
}

pub fn close_line(window: Option<u64>) -> Result<String, String> {
    let mut value = serde_json::json!({
        "type": "close",
    });

    if let Some(window) = window {
        value["window"] = serde_json::json!(window);
    }

    serde_json::to_string(&value).map_err(|err| err.to_string())
}

pub fn launch_line(command: &[String]) -> Result<String, String> {
    if command.is_empty() {
        return Err("launch command cannot be empty".into());
    }
    serde_json::to_string(&serde_json::json!({
        "type": "launch",
        "command": command,
    }))
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_default_key_press() {
        let command = AutomationCommand::Key {
            key: "KeyA".into(),
            action: KeyAction::default(),
        };

        assert_eq!(
            command.to_autowc_line(None).unwrap(),
            r#"{"action":"press","key":"KeyA","type":"key"}"#
        );
    }

    #[test]
    fn serializes_top_level_window_target() {
        let command = AutomationCommand::Key {
            key: "KeyA".into(),
            action: KeyAction::default(),
        };

        assert_eq!(
            command.to_autowc_line(Some(2)).unwrap(),
            r#"{"action":"press","key":"KeyA","type":"key","window":2}"#
        );
    }

    #[test]
    fn serializes_mouse_defaults() {
        let command = AutomationCommand::MouseButton {
            action: MouseButtonAction::default(),
            button: MouseButton::default(),
        };

        assert_eq!(
            command.to_autowc_line(None).unwrap(),
            r#"{"action":"press","button":"left","type":"mouse_button"}"#
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
    fn serializes_keys_sequence() {
        let command = AutomationCommand::Keys {
            keys: "<C-l>example.org<RET><w:500>".into(),
        };

        assert_eq!(
            command.to_autowc_line(None).unwrap(),
            r#"{"keys":"<C-l>example.org<RET><w:500>","type":"keys"}"#
        );
        assert_eq!(
            command.to_autowc_line(Some(2)).unwrap(),
            r#"{"keys":"<C-l>example.org<RET><w:500>","type":"keys","window":2}"#
        );
    }

    #[test]
    fn rejects_bad_tokens() {
        assert!(AutomationCommand::Key {
            key: "KeyA KeyB".into(),
            action: KeyAction::Press,
        }
        .to_autowc_line(None)
        .is_err());
        assert!(AutomationCommand::Chord { keys: vec![] }
            .to_autowc_line(None)
            .is_err());
    }

    #[test]
    fn serializes_screenshot_paths_with_whitespace() {
        assert_eq!(
            screenshot_line(Some(Path::new("/tmp/has space.png")), None).unwrap(),
            r#"{"path":"/tmp/has space.png","type":"screenshot"}"#
        );
        assert_eq!(
            screenshot_line(None, None).unwrap(),
            r#"{"type":"screenshot"}"#
        );
        assert_eq!(
            screenshot_line(None, Some(3)).unwrap(),
            r#"{"type":"screenshot","window":3}"#
        );
    }

    #[test]
    fn serializes_list_command() {
        assert_eq!(list_line(), r#"{"type":"list"}"#);
    }

    #[test]
    fn serializes_close_command() {
        assert_eq!(close_line(None).unwrap(), r#"{"type":"close"}"#);
        assert_eq!(
            close_line(Some(5)).unwrap(),
            r#"{"type":"close","window":5}"#
        );
    }

    #[test]
    fn serializes_launch_command() {
        assert_eq!(
            launch_line(&["foot".into(), "--hold".into()]).unwrap(),
            r#"{"command":["foot","--hold"],"type":"launch"}"#
        );
        assert!(launch_line(&[]).is_err());
    }

    #[test]
    fn command_schema_documents_agent_visible_fields() {
        let schema = schemars::schema_for!(AutomationCommand);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("W3C KeyboardEvent.code"));
        assert!(schema.contains("Prefer this for keyboard automation"));
        assert!(schema.contains("w:500"));
        assert!(schema.contains("waits 500 ms"));
        assert!(schema.contains("lowercase s for Super"));
        assert!(schema.contains("Virtual-display x coordinate"));
        assert!(schema.contains("wheel detents"));
        assert!(schema.contains("3.0 is a medium scroll"));
        assert!(schema.contains("Sleep duration in whole milliseconds"));
    }
}
