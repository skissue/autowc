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
        AutoWcSession, AutoWcSessionConfig, RunError, RunOutcome, Screenshot, SessionError,
        WindowInfo,
    },
};

const DEFAULT_SCREENSHOT_DELAY_MS: u64 = 500;

const SERVER_INSTRUCTIONS: &str = "\
AutoWC runs applications inside a nested compositor session for GUI automation.

Typical flow:
1. `launch` starts a process inside the server's running AutoWC compositor.
2. `list` reports the currently open windows and their stable AutoWC window IDs.
3. `run` sends an ordered batch of input commands, optionally targeting a specific window.
4. `screenshot` observes the current framebuffer, optionally targeting a specific window.
5. `close` requests that a target window's client toplevel close.

The MCP server owns one AutoWC compositor session. It starts AutoWC automatically with dynamic resizing and stay-alive enabled. The `launch` tool only starts an application process inside that compositor.

When more than one window is open, call `list` and then pass the desired window ID to `run`, `screenshot`, or `close`. If `window` is omitted, AutoWC uses the first existing window (lowest ID).

Mouse coordinates are virtual-display pixels with the origin at the top-left of the target AutoWC window. Keyboard commands use W3C KeyboardEvent.code physical key names, such as KeyA, Digit1, Enter, Escape, Backspace, Tab, Space, ControlLeft, ShiftLeft, AltLeft, MetaLeft, ArrowDown, and F5. Prefer the `keys` command for keyboard input when possible as it has the most flexibility. Use `key` or `chord` only when you need lower-level primitives.

If AutoWC exits, later tool calls return ok=false with the captured stderr log.";

#[derive(Debug, Clone)]
pub struct AutoWcMcpServer {
    session: AutoWcSession,
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
    pub async fn new(autowc_binary: PathBuf) -> Result<Self, SessionError> {
        let session = AutoWcSession::new(AutoWcSessionConfig {
            autowc_binary,
            command: vec!["true".into()],
            stay_alive: true,
            key_event_interval_ms: None,
            chord_key_interval_ms: None,
            chord_hold_ms: None,
            command_interval_ms: None,
        })
        .await?;

        Ok(Self {
            session,
            tool_router: Self::tool_router(),
        })
    }

    pub async fn shutdown(&self) {
        self.session.shutdown().await;
    }

    #[tool(
        name = "launch",
        description = "Launch a process inside the running AutoWC compositor session."
    )]
    pub async fn launch(
        &self,
        Parameters(params): Parameters<LaunchParams>,
    ) -> Result<CallToolResult, String> {
        if let Err(err) = self.session.launch(&params.command).await {
            return Ok(error_result(err));
        }

        Ok(launch_result(params.command))
    }

    #[tool(
        name = "run",
        description = "Send a batch of automation commands to the running AutoWC compositor session. Returns a final screenshot by default so agents can observe the result of their commands. When run has a window target, the returned screenshot uses the same target."
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
            .session
            .run(
                &params.commands,
                params.window,
                return_screenshot,
                screenshot_delay_ms,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(err) => return Ok(run_error_result(err)),
        };

        Ok(run_result(outcome))
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
            .session
            .screenshot(params.path.as_deref(), params.window)
            .await
        {
            Ok(screenshot) => screenshot,
            Err(err) => return Ok(error_result(err)),
        };

        Ok(screenshot_result(screenshot))
    }

    #[tool(
        name = "list",
        description = "List the open AutoWC windows in the running compositor session, including stable window ids for targeted automation."
    )]
    pub async fn list(
        &self,
        Parameters(_params): Parameters<ListParams>,
    ) -> Result<CallToolResult, String> {
        let windows = match self.session.list().await {
            Ok(windows) => windows,
            Err(err) => return Ok(error_result(err)),
        };

        Ok(list_result(windows))
    }

    #[tool(
        name = "close",
        description = "Request that an AutoWC window's client toplevel close. This only sends a close request; the application may still decide whether and when to exit."
    )]
    pub async fn close(
        &self,
        Parameters(params): Parameters<CloseParams>,
    ) -> Result<CallToolResult, String> {
        if let Err(err) = self.session.close(params.window).await {
            return Ok(error_result(err));
        }

        Ok(close_result(params.window))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LaunchParams {
    #[schemars(
        description = "Command to launch inside the running AutoWC compositor as argv: the first item is the executable, and the remaining items are its arguments."
    )]
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunParams {
    #[schemars(
        description = "Ordered automation command batch. Command types are key, chord, keys, mouse_move, mouse_button, click, mouse_drag, scroll, and sleep."
    )]
    pub commands: Vec<AutomationCommand>,
    #[schemars(
        description = "Optional AutoWC window id to target for every command in this batch and for the returned screenshot. Omit to use AutoWC's default window."
    )]
    pub window: Option<u64>,
    #[schemars(
        description = "Whether to return an inline screenshot after the batch completes. Defaults to true. Set to false only when intentionally running without immediate visual feedback or if your commands intend to close the targeted window."
    )]
    pub return_screenshot: Option<bool>,
    #[schemars(
        description = "Delay in milliseconds after all commands execute before the final screenshot is captured. Defaults to 500."
    )]
    pub screenshot_delay_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    #[schemars(
        description = "Optional PNG output path. When omitted, AutoWC writes to a temporary file; the MCP result includes the image inline either way."
    )]
    pub path: Option<PathBuf>,
    #[schemars(
        description = "Optional AutoWC window id to capture. Omit to use AutoWC's default window."
    )]
    pub window: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseParams {
    #[schemars(
        description = "Optional AutoWC window id to close. Omit to use AutoWC's default window."
    )]
    pub window: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ScreenshotMetadata {
    path: PathBuf,
    mime_type: &'static str,
}

fn launch_result(command: Vec<String>) -> CallToolResult {
    CallToolResult::structured(json!({
        "ok": true,
        "command": command,
    }))
}

fn run_result(outcome: RunOutcome) -> CallToolResult {
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
            "commands_executed": outcome.commands_executed,
            "screenshot": screenshot_metadata,
    }));
    result
}

fn run_error_result(err: RunError) -> CallToolResult {
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
        "error": err.error.message,
        "error_stage": err.stage,
        "failed_command_index": err.failed_command_index,
        "exit_status": err.error.exit_status,
        "stderr": err.error.stderr,
        "commands_executed": err.commands_executed,
        "screenshot": screenshot_metadata,
    }));
    result
}

fn screenshot_result(screenshot: Screenshot) -> CallToolResult {
    let mut result = CallToolResult::success(vec![
        Content::text("ok"),
        Content::image(screenshot.data_base64.clone(), screenshot.mime_type),
    ]);
    result.structured_content = Some(json!({
            "ok": true,
            "screenshot": ScreenshotMetadata {
                path: screenshot.path,
                mime_type: screenshot.mime_type,
            },
    }));
    result
}

fn list_result(windows: Vec<WindowInfo>) -> CallToolResult {
    CallToolResult::structured(json!({
        "ok": true,
        "windows": windows,
    }))
}

fn close_result(window: Option<u64>) -> CallToolResult {
    CallToolResult::structured(json!({
        "ok": true,
        "window": window,
    }))
}

fn error_result(err: SessionError) -> CallToolResult {
    let mut result = CallToolResult::error(vec![Content::text(err.message.clone())]);
    result.structured_content = Some(json!({
        "ok": false,
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
        assert!(SERVER_INSTRUCTIONS.contains("one AutoWC compositor session"));
        assert!(SERVER_INSTRUCTIONS.contains("starts AutoWC automatically"));
        assert!(SERVER_INSTRUCTIONS.contains("call `list`"));
        assert!(SERVER_INSTRUCTIONS.contains("W3C KeyboardEvent.code"));
        assert!(SERVER_INSTRUCTIONS.contains("Prefer the `keys` command"));
        assert!(!SERVER_INSTRUCTIONS.contains("return_screenshot"));
        assert!(!SERVER_INSTRUCTIONS.contains("The close tool only sends a close request"));
        assert!(!SERVER_INSTRUCTIONS.contains("close stops"));
        assert!(!SERVER_INSTRUCTIONS.contains("KEY_*"));
        assert!(!SERVER_INSTRUCTIONS.contains("newline characters"));
        assert!(!SERVER_INSTRUCTIONS.contains("`text`"));
    }

    #[test]
    fn launch_schema_documents_process_command() {
        let schema = schemars::schema_for!(LaunchParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("first item is the executable"));
        assert!(schema.contains("running AutoWC compositor"));
        assert!(!schema.contains("width"));
        assert!(!schema.contains("session"));
    }

    #[test]
    fn run_schema_documents_batch_and_screenshot_default() {
        let schema = schemars::schema_for!(RunParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("Ordered automation command batch"));
        assert!(schema.contains("key, chord, keys"));
        assert!(schema.contains("mouse_drag"));
        assert!(!schema.contains("text, keys"));
        assert!(schema.contains("Optional AutoWC window id"));
        assert!(schema.contains("Defaults to true"));
        assert!(schema.contains("without immediate visual feedback"));
        assert!(schema.contains("intend to close the targeted window"));
        assert!(schema.contains("Defaults to 500"));
    }

    #[test]
    fn screenshot_schema_documents_optional_path() {
        let schema = schemars::schema_for!(ScreenshotParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("Optional PNG output path"));
        assert!(schema.contains("temporary file"));
        assert!(schema.contains("image inline"));
        assert!(schema.contains("Optional AutoWC window id"));
    }

    #[test]
    fn list_schema_has_no_required_params() {
        let schema = schemars::schema_for!(ListParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(!schema.contains("session_id"));
    }

    #[test]
    fn close_schema_documents_optional_window() {
        let schema = schemars::schema_for!(CloseParams);
        let schema = serde_json::to_string(&schema).unwrap();

        assert!(schema.contains("Optional AutoWC window id"));
        assert!(schema.contains("default window"));
    }

    #[test]
    fn run_result_includes_inline_image_when_present() {
        let result = run_result(RunOutcome {
            commands_executed: 1,
            screenshot: Some(Screenshot {
                path: PathBuf::from("/tmp/image.png"),
                mime_type: "image/png",
                data_base64: "abc".into(),
            }),
        });

        assert_eq!(result.content.len(), 2);
        assert!(result.content[1].as_image().is_some());
        assert_eq!(result.structured_content.unwrap()["commands_executed"], 1);
    }

    #[test]
    fn run_error_result_includes_partial_count_and_screenshot() {
        let result = run_error_result(RunError {
            error: SessionError {
                message: "unknown key: CTRL".into(),
                exit_status: None,
                stderr: String::new(),
            },
            stage: crate::session::RunErrorStage::Command,
            commands_executed: 2,
            failed_command_index: Some(2),
            screenshot: Some(Screenshot {
                path: PathBuf::from("/tmp/image.png"),
                mime_type: "image/png",
                data_base64: "abc".into(),
            }),
        });

        assert!(result.is_error.unwrap_or(false));
        assert_eq!(result.content.len(), 2);
        assert!(result.content[1].as_image().is_some());
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["commands_executed"], 2);
        assert_eq!(structured["error_stage"], "command");
        assert_eq!(structured["failed_command_index"], 2);
        assert_eq!(structured["error"], "unknown key: CTRL");
        assert_eq!(structured["screenshot"]["mime_type"], "image/png");
    }

    #[test]
    fn list_result_includes_windows() {
        let result = list_result(vec![WindowInfo {
            id: 2,
            title: "Dialog".into(),
            width: 640,
            height: 480,
        }]);

        let structured = result.structured_content.unwrap();
        assert_eq!(structured["ok"], true);
        assert_eq!(structured["windows"][0]["id"], 2);
        assert_eq!(structured["windows"][0]["title"], "Dialog");
        assert_eq!(structured["windows"][0]["width"], 640);
        assert_eq!(structured["windows"][0]["height"], 480);
    }

    #[test]
    fn close_result_includes_target_window() {
        let result = close_result(Some(7));
        let structured = result.structured_content.unwrap();

        assert_eq!(structured["ok"], true);
        assert_eq!(structured["window"], 7);
    }
}
