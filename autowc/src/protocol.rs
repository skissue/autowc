use std::io::{self, Write};

use serde::Serialize;

use crate::control::{parse_control_command, parse_json_control_line, ControlCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Plain,
    Json,
}

impl Protocol {
    pub fn parse_control_command(self, line: &str) -> Result<Option<ControlCommand>, String> {
        match self {
            Self::Plain => parse_control_command(line),
            Self::Json => parse_json_control_line(line),
        }
    }

    pub fn send_ok(self) {
        send(self.format_ok());
    }

    pub fn send_ok_with_new_windows(self, new_windows: &[WindowInfo]) {
        send(self.format_ok_with_new_windows(new_windows));
    }

    pub fn send_error(self, error: impl AsRef<str>) {
        send(self.format_error(error));
    }

    pub fn send_screenshot(self, path: impl AsRef<str>) {
        send(self.format_screenshot(path));
    }

    pub fn send_window_list(self, windows: &[WindowInfo]) {
        send(self.format_window_list(windows));
    }

    pub fn format_ok(self) -> String {
        match self {
            Self::Plain => "ok".into(),
            Self::Json => serialize_response(&OkResponse { ok: true }),
        }
    }

    pub fn format_ok_with_new_windows(self, new_windows: &[WindowInfo]) -> String {
        match self {
            Self::Plain => {
                let mut response = String::from("ok");
                for window in new_windows {
                    response.push_str(&format!(" {} {}", window.id, window.title));
                }
                response
            }
            Self::Json => serialize_response(&OkWithNewWindowsResponse {
                ok: true,
                new_windows,
            }),
        }
    }

    pub fn format_error(self, error: impl AsRef<str>) -> String {
        let error = error.as_ref();
        match self {
            Self::Plain => format!("error {error}"),
            Self::Json => serialize_response(&ErrorResponse { ok: false, error }),
        }
    }

    pub fn format_screenshot(self, path: impl AsRef<str>) -> String {
        let path = path.as_ref();
        match self {
            Self::Plain => format!("screenshot {path}"),
            Self::Json => serialize_response(&ScreenshotResponse {
                ok: true,
                response_type: "screenshot",
                path,
            }),
        }
    }

    pub fn format_window_list(self, windows: &[WindowInfo]) -> String {
        match self {
            Self::Plain => "ok".into(),
            Self::Json => serialize_response(&WindowListResponse { ok: true, windows }),
        }
    }
}

pub fn send(line: impl AsRef<str>) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", line.as_ref());
    let _ = stdout.flush();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct OkWithNewWindowsResponse<'a> {
    ok: bool,
    new_windows: &'a [WindowInfo],
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    ok: bool,
    error: &'a str,
}

#[derive(Serialize)]
struct ScreenshotResponse<'a> {
    ok: bool,
    #[serde(rename = "type")]
    response_type: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
struct WindowListResponse<'a> {
    ok: bool,
    windows: &'a [WindowInfo],
}

fn serialize_response<T: Serialize>(response: &T) -> String {
    serde_json::to_string(response).expect("protocol responses should always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_plain_responses() {
        assert_eq!(Protocol::Plain.format_ok(), "ok");
        assert_eq!(
            Protocol::Plain.format_ok_with_new_windows(&[WindowInfo {
                id: 2,
                title: "GTK Demo".to_string(),
            }]),
            "ok 2 GTK Demo"
        );
        assert_eq!(Protocol::Plain.format_error("bad input"), "error bad input");
        assert_eq!(
            Protocol::Plain.format_screenshot("/tmp/autowc.png"),
            "screenshot /tmp/autowc.png"
        );
    }

    #[test]
    fn formats_json_responses() {
        assert_eq!(Protocol::Json.format_ok(), r#"{"ok":true}"#);
        assert_eq!(
            Protocol::Json.format_ok_with_new_windows(&[WindowInfo {
                id: 2,
                title: "GTK Demo".to_string(),
            }]),
            r#"{"ok":true,"new_windows":[{"id":2,"title":"GTK Demo"}]}"#
        );
        assert_eq!(
            Protocol::Json.format_error("bad \"input\""),
            r#"{"ok":false,"error":"bad \"input\""}"#
        );
        assert_eq!(
            Protocol::Json.format_screenshot("/tmp/autowc.png"),
            r#"{"ok":true,"type":"screenshot","path":"/tmp/autowc.png"}"#
        );
        assert_eq!(
            Protocol::Json.format_window_list(&[WindowInfo {
                id: 2,
                title: "GTK Demo".to_string(),
            }]),
            r#"{"ok":true,"windows":[{"id":2,"title":"GTK Demo"}]}"#
        );
    }
}
