use std::path::PathBuf;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    command::AutomationCommand,
    session::{
        RunError, RunOutcome, Screenshot, SessionConfig, SessionError, SessionManager,
        SessionMetadata,
    },
};

const DEFAULT_SCREENSHOT_DELAY_MS: u64 = 500;

const SERVER_INSTRUCTIONS: &str = "\
AutoWC runs applications inside nested compositor sessions for GUI automation.

Typical flow:
1. launch starts a session by running a command inside AutoWC.
2. run sends an ordered batch of input commands to that session.
3. screenshot observes the current framebuffer without sending input.
4. close stops the compositor process.

Mouse coordinates are virtual-display pixels with the origin at the top-left of the AutoWC display. The launch width and height set that virtual display size.

Keyboard commands use W3C KeyboardEvent.code physical key names, such as KeyA, Digit1, Enter, Escape, Backspace, Tab, Space, ControlLeft, ShiftLeft, AltLeft, MetaLeft, ArrowDown, and F5.

The run tool returns a final screenshot by default so agents can observe the result of a batch. Set return_screenshot to false only when intentionally running without immediate visual feedback.

If a session exits, later tool calls return ok=false with the captured stderr log.";

#[derive(Debug, Clone)]
pub struct AutoWcMcpServer {
    autowc_binary: PathBuf,
    sessions: SessionManager,
    tool_router: ToolRouter<Self>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AutoWcMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("autowc-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}

#[tool_router(router = tool_router)]
impl AutoWcMcpServer {
    pub fn new(autowc_binary: PathBuf) -> Self {
        Self {
            autowc_binary,
            sessions: SessionManager::new(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "launch",
        description = "Start an AutoWC session by running a command inside a nested compositor."
    )]
    pub async fn launch(
        &self,
        Parameters(params): Parameters<LaunchParams>,
    ) -> Result<CallToolResult, String> {
        let metadata = match self
            .sessions
            .launch(SessionConfig {
                autowc_binary: self.autowc_binary.clone(),
                command: params.command,
                width: params.width.unwrap_or(1280),
                height: params.height.unwrap_or(720),
                stay_alive: params.stay_alive.unwrap_or(false),
                key_event_interval_ms: params.key_event_interval_ms,
                chord_key_interval_ms: params.chord_key_interval_ms,
                chord_hold_ms: params.chord_hold_ms,
                command_interval_ms: params.command_interval_ms,
            })
            .await
        {
            Ok(metadata) => metadata,
            Err(err) => return Ok(error_result(None, err)),
        };

        Ok(metadata_result(metadata))
    }

    #[tool(
        name = "run",
        description = "Send a batch of automation commands to a session."
    )]
    pub async fn run(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<CallToolResult, String> {
        let return_screenshot = params.return_screenshot.unwrap_or(true);
        let screenshot_delay_ms = params
            .screenshot_delay_ms
            .unwrap_or(DEFAULT_SCREENSHOT_DELAY_MS);
        let outcome = match self
            .sessions
            .run(
                &params.session_id,
                &params.commands,
                return_screenshot,
                screenshot_delay_ms,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(err) => return Ok(run_error_result(params.session_id, err)),
        };

        Ok(run_result(params.session_id, outcome))
    }

    #[tool(
        name = "screenshot",
        description = "Capture the latest framebuffer without sending input."
    )]
    pub async fn screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotParams>,
    ) -> Result<CallToolResult, String> {
        let screenshot = match self
            .sessions
            .screenshot(&params.session_id, params.path.as_deref())
            .await
        {
            Ok(screenshot) => screenshot,
            Err(err) => return Ok(error_result(Some(params.session_id), err)),
        };

        Ok(screenshot_result(params.session_id, screenshot))
    }

    #[tool(
        name = "close",
        description = "Close a session and stop its compositor process."
    )]
    pub async fn close(
        &self,
        Parameters(params): Parameters<CloseParams>,
    ) -> Result<CallToolResult, String> {
        let closed = match self.sessions.close(&params.session_id).await {
            Ok(closed) => closed,
            Err(err) => return Ok(error_result(Some(params.session_id), err)),
        };
        Ok(CallToolResult::structured(json!({
            "ok": true,
            "session_id": params.session_id,
            "closed": closed,
        })))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LaunchParams {
    #[schemars(
        description = "Command to run inside AutoWC as argv: the first item is the executable, and the remaining items are its arguments."
    )]
    pub command: Vec<String>,
    #[schemars(description = "Virtual display width in pixels. Defaults to 1280.")]
    pub width: Option<u32>,
    #[schemars(description = "Virtual display height in pixels. Defaults to 720.")]
    pub height: Option<u32>,
    #[schemars(
        description = "When true, keep the AutoWC session running after all client windows exit. Defaults to false."
    )]
    pub stay_alive: Option<bool>,
    #[schemars(
        description = "Delay in milliseconds between synthetic key press and release events. Uses AutoWC's default when omitted."
    )]
    pub key_event_interval_ms: Option<u64>,
    #[schemars(
        description = "Delay in milliseconds between pressing each key in a chord. Uses AutoWC's default when omitted."
    )]
    pub chord_key_interval_ms: Option<u64>,
    #[schemars(
        description = "Duration in milliseconds to hold a chord after all keys are pressed. Uses AutoWC's default when omitted."
    )]
    pub chord_hold_ms: Option<u64>,
    #[schemars(
        description = "Delay in milliseconds after each command in a run batch. Uses AutoWC's default when omitted."
    )]
    pub command_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunParams {
    #[schemars(description = "Session id returned by launch.")]
    pub session_id: String,
    #[schemars(
        description = "Ordered automation command batch. Command types are key, chord, text, mouse_move, mouse_button, click, scroll, and sleep."
    )]
    pub commands: Vec<AutomationCommand>,
    #[schemars(
        description = "Whether to return an inline screenshot after the batch completes. Defaults to true."
    )]
    pub return_screenshot: Option<bool>,
    #[schemars(
        description = "Delay in milliseconds after all commands execute before the final screenshot is captured. Defaults to 500."
    )]
    pub screenshot_delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    #[schemars(description = "Session id returned by launch.")]
    pub session_id: String,
    #[schemars(
        description = "Optional PNG output path. When omitted, AutoWC writes to a temporary file; the MCP result includes the image inline either way."
    )]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseParams {
    #[schemars(description = "Session id returned by launch.")]
    pub session_id: String,
}

#[derive(Debug, Serialize)]
struct ScreenshotMetadata {
    path: PathBuf,
    mime_type: &'static str,
}

fn metadata_result(metadata: SessionMetadata) -> CallToolResult {
    CallToolResult::structured(json!({
        "ok": true,
        "session_id": metadata.session_id,
        "width": metadata.width,
        "height": metadata.height,
        "command": metadata.command,
    }))
}

fn run_result(session_id: String, outcome: RunOutcome) -> CallToolResult {
    let mut content = vec![Content::text("ok")];
    let screenshot_metadata = if let Some(screenshot) = outcome.screenshot {
        content.push(Content::image(
            screenshot.data_base64.clone(),
            screenshot.mime_type,
        ));
        Some(ScreenshotMetadata {
            path: screenshot.path,
            mime_type: screenshot.mime_type,
        })
    } else {
        None
    };

    let mut result = CallToolResult::success(content);
    result.structured_content = Some(json!({
            "ok": true,
            "session_id": session_id,
            "commands_executed": outcome.commands_executed,
            "screenshot": screenshot_metadata,
    }));
    result
}

fn run_error_result(session_id: String, err: RunError) -> CallToolResult {
    let mut content = vec![Content::text(err.error.message.clone())];
    let screenshot_metadata = if let Some(screenshot) = err.screenshot {
        content.push(Content::image(
            screenshot.data_base64.clone(),
            screenshot.mime_type,
        ));
        Some(ScreenshotMetadata {
            path: screenshot.path,
            mime_type: screenshot.mime_type,
        })
    } else {
        None
    };

    let mut result = CallToolResult::error(content);
    result.structured_content = Some(json!({
        "ok": false,
        "session_id": session_id,
        "error": err.error.message,
        "exit_status": err.error.exit_status,
        "stderr": err.error.stderr,
        "commands_executed": err.commands_executed,
        "screenshot": screenshot_metadata,
    }));
    result
}

fn screenshot_result(session_id: String, screenshot: Screenshot) -> CallToolResult {
    let mut result = CallToolResult::success(vec![
        Content::text("ok"),
        Content::image(screenshot.data_base64.clone(), screenshot.mime_type),
    ]);
    result.structured_content = Some(json!({
            "ok": true,
            "session_id": session_id,
            "screenshot": ScreenshotMetadata {
                path: screenshot.path,
                mime_type: screenshot.mime_type,
            },
    }));
    result
}

fn error_result(session_id: Option<String>, err: SessionError) -> CallToolResult {
    let mut result = CallToolResult::error(vec![Content::text(err.message.clone())]);
    result.structured_content = Some(json!({
        "ok": false,
        "session_id": session_id,
        "error": err.message,
        "exit_status": err.exit_status,
        "stderr": err.stderr,
    }));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_instructions_are_the_canonical_usage_guide() {
        assert!(SERVER_INSTRUCTIONS.contains("Typical flow"));
        assert!(SERVER_INSTRUCTIONS.contains("return_screenshot"));
        assert!(SERVER_INSTRUCTIONS.contains("W3C KeyboardEvent.code"));
        assert!(!SERVER_INSTRUCTIONS.contains("KEY_*"));
        assert!(!SERVER_INSTRUCTIONS.contains("newline characters"));
    }

    #[test]
    fn launch_schema_documents_session_options() {
        let schema = schemars::schema_for!(LaunchParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("first item is the executable"));
        assert!(schema.contains("Defaults to 1280"));
        assert!(schema.contains("Defaults to 720"));
        assert!(schema.contains("stay"));
        assert!(schema.contains("Delay in milliseconds"));
    }

    #[test]
    fn run_schema_documents_batch_and_screenshot_default() {
        let schema = schemars::schema_for!(RunParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("Ordered automation command batch"));
        assert!(schema.contains("key, chord, text"));
        assert!(schema.contains("Defaults to true"));
        assert!(schema.contains("Defaults to 500"));
    }

    #[test]
    fn screenshot_schema_documents_optional_path() {
        let schema = schemars::schema_for!(ScreenshotParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("Optional PNG output path"));
        assert!(schema.contains("temporary file"));
        assert!(schema.contains("image inline"));
    }

    #[test]
    fn run_result_includes_inline_image_when_present() {
        let result = run_result(
            "session".into(),
            RunOutcome {
                commands_executed: 1,
                screenshot: Some(Screenshot {
                    path: PathBuf::from("/tmp/image.png"),
                    mime_type: "image/png",
                    data_base64: "abc".into(),
                }),
            },
        );

        assert_eq!(result.content.len(), 2);
        assert!(result.content[1].as_image().is_some());
        assert_eq!(result.structured_content.unwrap()["commands_executed"], 1);
    }

    #[test]
    fn run_error_result_includes_partial_count_and_screenshot() {
        let result = run_error_result(
            "session".into(),
            RunError {
                error: SessionError {
                    message: "unknown key: CTRL".into(),
                    exit_status: None,
                    stderr: String::new(),
                },
                commands_executed: 2,
                screenshot: Some(Screenshot {
                    path: PathBuf::from("/tmp/image.png"),
                    mime_type: "image/png",
                    data_base64: "abc".into(),
                }),
            },
        );

        assert!(result.is_error.unwrap_or(false));
        assert_eq!(result.content.len(), 2);
        assert!(result.content[1].as_image().is_some());
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["commands_executed"], 2);
        assert_eq!(structured["error"], "unknown key: CTRL");
        assert_eq!(structured["screenshot"]["mime_type"], "image/png");
    }
}
