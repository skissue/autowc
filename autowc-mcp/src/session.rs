use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::Mutex,
};

use crate::command::{close_line, launch_line, list_line, screenshot_line, AutomationCommand};

const STDERR_LINE_LIMIT: usize = 200;
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const FORCED_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const QUIT_COMMAND: &str = r#"{"type":"quit"}"#;

#[derive(Debug, Clone)]
pub struct AutoWcSessionConfig {
    pub autowc_binary: PathBuf,
    pub command: Vec<String>,
    pub stay_alive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Screenshot {
    pub path: PathBuf,
    pub mime_type: &'static str,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub width: i32,
    pub height: i32,
    pub fixed: bool,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub commands_executed: usize,
    pub screenshot: Option<Screenshot>,
}

#[derive(Debug)]
pub struct RunError {
    pub error: SessionError,
    pub stage: RunErrorStage,
    pub commands_executed: usize,
    pub failed_command_index: Option<usize>,
    pub screenshot: Option<Screenshot>,
}

impl RunError {
    fn new(error: SessionError) -> Self {
        Self {
            error,
            stage: RunErrorStage::Session,
            commands_executed: 0,
            failed_command_index: None,
            screenshot: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunErrorStage {
    Session,
    Prepare,
    Command,
    Screenshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionError {
    pub message: String,
    pub exit_status: Option<String>,
    pub stderr: String,
}

impl SessionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_status: None,
            stderr: String::new(),
        }
    }

    fn with_process(
        message: impl Into<String>,
        exit_status: Option<ExitStatus>,
        stderr: String,
    ) -> Self {
        Self {
            message: message.into(),
            exit_status: exit_status.map(|status| status.to_string()),
            stderr,
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone)]
pub struct AutoWcSession {
    inner: Arc<Mutex<AutoWcProcess>>,
}

impl AutoWcSession {
    pub async fn new(config: AutoWcSessionConfig) -> Result<Self, SessionError> {
        let process = AutoWcProcess::spawn(config).await?;
        Ok(Self {
            inner: Arc::new(Mutex::new(process)),
        })
    }

    pub async fn launch(&self, command: &[String]) -> Result<(), SessionError> {
        let mut process = self.inner.lock().await;
        process.launch(command).await
    }

    pub async fn run(
        &self,
        commands: &[AutomationCommand],
        window: Option<u64>,
        return_screenshot: bool,
        screenshot_delay_ms: u64,
    ) -> Result<RunOutcome, RunError> {
        let mut process = self.inner.lock().await;
        process
            .run(commands, window, return_screenshot, screenshot_delay_ms)
            .await
    }

    pub async fn screenshot(
        &self,
        path: Option<&Path>,
        window: Option<u64>,
    ) -> Result<Screenshot, SessionError> {
        let mut process = self.inner.lock().await;
        process.screenshot(path, window).await
    }

    pub async fn list(&self) -> Result<Vec<WindowInfo>, SessionError> {
        let mut process = self.inner.lock().await;
        process.list().await
    }

    pub async fn close(&self, window: Option<u64>) -> Result<(), SessionError> {
        let mut process = self.inner.lock().await;
        process.close(window).await
    }

    pub async fn shutdown(&self) {
        let mut process = self.inner.lock().await;
        process.shutdown().await;
    }
}

#[derive(Debug)]
struct AutoWcProcess {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr: SharedStderr,
    exit_status: Option<ExitStatus>,
}

impl AutoWcProcess {
    async fn spawn(config: AutoWcSessionConfig) -> Result<Self, SessionError> {
        let mut command = Command::new(&config.autowc_binary);
        command
            .args(autowc_args(&config))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|err| SessionError::new(format!("failed to spawn AutoWC: {err}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SessionError::new("failed to open AutoWC stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SessionError::new("failed to open AutoWC stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SessionError::new("failed to open AutoWC stderr"))?;
        let stderr = SharedStderr::spawn(stderr);

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            stderr,
            exit_status: None,
        })
    }

    async fn launch(&mut self, command: &[String]) -> Result<(), SessionError> {
        self.ensure_running().await?;
        let line = launch_line(command).map_err(SessionError::new)?;
        self.execute_plan([PlannedCommand::expect_ok(line)])
            .await
            .map(|_| ())
    }

    async fn run(
        &mut self,
        commands: &[AutomationCommand],
        window: Option<u64>,
        return_screenshot: bool,
        screenshot_delay_ms: u64,
    ) -> Result<RunOutcome, RunError> {
        self.ensure_running().await.map_err(RunError::new)?;
        let mut commands_executed = 0;

        for (index, command) in commands.iter().enumerate() {
            let line = match command.to_autowc_line(window).map_err(SessionError::new) {
                Ok(line) => line,
                Err(error) => {
                    return Err(self
                        .run_error(
                            commands_executed,
                            Some(index),
                            RunErrorStage::Prepare,
                            error,
                            window,
                            return_screenshot,
                            screenshot_delay_ms,
                        )
                        .await)
                }
            };

            if let Err(error) = self.execute_plan([PlannedCommand::expect_ok(line)]).await {
                return Err(self
                    .run_error(
                        commands_executed,
                        Some(index),
                        RunErrorStage::Command,
                        error,
                        window,
                        return_screenshot,
                        screenshot_delay_ms,
                    )
                    .await);
            }

            commands_executed += 1;
        }

        let screenshot = if return_screenshot {
            Some(
                self.delayed_screenshot(None, window, screenshot_delay_ms)
                    .await
                    .map_err(|error| RunError {
                        error,
                        stage: RunErrorStage::Screenshot,
                        commands_executed,
                        failed_command_index: None,
                        screenshot: None,
                    })?,
            )
        } else {
            None
        };

        Ok(RunOutcome {
            commands_executed,
            screenshot,
        })
    }

    async fn run_error(
        &mut self,
        commands_executed: usize,
        failed_command_index: Option<usize>,
        stage: RunErrorStage,
        error: SessionError,
        window: Option<u64>,
        return_screenshot: bool,
        screenshot_delay_ms: u64,
    ) -> RunError {
        let screenshot = if return_screenshot {
            self.delayed_screenshot(None, window, screenshot_delay_ms)
                .await
                .ok()
        } else {
            None
        };

        RunError {
            error,
            stage,
            commands_executed,
            failed_command_index,
            screenshot,
        }
    }

    async fn screenshot(
        &mut self,
        path: Option<&Path>,
        window: Option<u64>,
    ) -> Result<Screenshot, SessionError> {
        self.ensure_running().await?;
        self.delayed_screenshot(path, window, 0).await
    }

    async fn delayed_screenshot(
        &mut self,
        path: Option<&Path>,
        window: Option<u64>,
        delay_ms: u64,
    ) -> Result<Screenshot, SessionError> {
        let mut plan = screenshot_plan(path, window, delay_ms)?;
        let observed = self.execute_plan(plan.drain(..)).await?;
        match observed.into_iter().last() {
            Some(ObservedResponse::Screenshot(screenshot)) => Ok(screenshot),
            _ => Err(SessionError::new(
                "internal error: screenshot plan did not return a screenshot",
            )),
        }
    }

    async fn list(&mut self) -> Result<Vec<WindowInfo>, SessionError> {
        self.ensure_running().await?;
        let observed = self
            .execute_plan([PlannedCommand::expect_window_list(list_line())])
            .await?;
        match observed.into_iter().next() {
            Some(ObservedResponse::WindowList(windows)) => Ok(windows),
            _ => Err(SessionError::new(
                "internal error: list plan did not return a window list",
            )),
        }
    }

    async fn close(&mut self, window: Option<u64>) -> Result<(), SessionError> {
        self.ensure_running().await?;
        let line = close_line(window).map_err(SessionError::new)?;
        self.execute_plan([PlannedCommand::expect_ok(line)])
            .await
            .map(|_| ())
    }

    async fn shutdown(&mut self) {
        self.refresh_exit_status();
        if self.exit_status.is_some() {
            return;
        }

        let quit_sent = self.write_line(QUIT_COMMAND).await.is_ok();
        if quit_sent {
            if let Ok(Ok(status)) =
                tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, self.child.wait()).await
            {
                self.exit_status = Some(status);
                return;
            }
        }

        let _ = self.child.start_kill();
        if let Ok(Ok(status)) =
            tokio::time::timeout(FORCED_SHUTDOWN_TIMEOUT, self.child.wait()).await
        {
            self.exit_status = Some(status);
        }
    }

    async fn execute_plan(
        &mut self,
        plan: impl IntoIterator<Item = PlannedCommand>,
    ) -> Result<Vec<ObservedResponse>, SessionError> {
        let mut observed = Vec::new();
        for command in plan {
            self.write_line(&command.line).await?;
            let response = self.read_response().await?;
            observed.push(observe_response(response, command.expected).await?);
        }
        Ok(observed)
    }

    async fn read_response(&mut self) -> Result<AutowcResponse, SessionError> {
        let response = self
            .stdout
            .next_line()
            .await
            .map_err(|err| self.process_error(format!("failed reading AutoWC stdout: {err}")))?
            .ok_or_else(|| {
                self.refresh_exit_status();
                self.process_error("AutoWC exited before responding")
            })?;

        parse_response(&response).map_err(SessionError::new)
    }

    async fn write_line(&mut self, line: &str) -> Result<(), SessionError> {
        self.ensure_running().await?;
        self.stdin.write_all(line.as_bytes()).await.map_err(|err| {
            self.refresh_exit_status();
            self.process_error(format!("failed writing AutoWC command: {err}"))
        })?;
        self.stdin.write_all(b"\n").await.map_err(|err| {
            self.refresh_exit_status();
            self.process_error(format!("failed writing AutoWC command: {err}"))
        })?;
        self.stdin.flush().await.map_err(|err| {
            self.refresh_exit_status();
            self.process_error(format!("failed flushing AutoWC command: {err}"))
        })
    }

    async fn ensure_running(&mut self) -> Result<(), SessionError> {
        self.refresh_exit_status();
        if self.exit_status.is_some() {
            return Err(self.process_error("AutoWC process has exited"));
        }
        Ok(())
    }

    fn refresh_exit_status(&mut self) {
        if self.exit_status.is_some() {
            return;
        }
        if let Ok(status) = self.child.try_wait() {
            self.exit_status = status;
        }
    }

    fn process_error(&self, message: impl Into<String>) -> SessionError {
        SessionError::with_process(message, self.exit_status, self.stderr.snapshot())
    }
}

impl Drop for AutoWcProcess {
    fn drop(&mut self) {
        self.refresh_exit_status();
        if self.exit_status.is_none() {
            let _ = self.child.start_kill();
        }
    }
}

fn autowc_args(config: &AutoWcSessionConfig) -> Vec<String> {
    let mut args = vec!["--json".into()];

    if config.stay_alive {
        args.push("--stay-alive".into());
    }
    args.extend(config.command.clone());
    args
}

#[derive(Debug, Clone)]
struct SharedStderr {
    lines: Arc<StdMutex<std::collections::VecDeque<String>>>,
}

impl SharedStderr {
    fn spawn(stderr: ChildStderr) -> Self {
        let lines = Arc::new(StdMutex::new(std::collections::VecDeque::new()));
        let reader_lines = lines.clone();

        tokio::spawn(async move {
            let mut stderr = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = stderr.next_line().await {
                let Ok(mut lines) = reader_lines.lock() else {
                    break;
                };
                lines.push_back(line);
                while lines.len() > STDERR_LINE_LIMIT {
                    lines.pop_front();
                }
            }
        });

        Self { lines }
    }

    fn snapshot(&self) -> String {
        self.lines.lock().map_or_else(
            |_| String::new(),
            |lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"),
        )
    }
}

#[derive(Debug)]
enum AutowcResponse {
    Ok,
    Error(String),
    Screenshot { path: PathBuf },
    WindowList { windows: Vec<WindowInfo> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedResponse {
    Ok,
    Screenshot,
    WindowList,
}

#[derive(Debug)]
struct PlannedCommand {
    line: String,
    expected: ExpectedResponse,
}

impl PlannedCommand {
    fn expect_ok(line: String) -> Self {
        Self {
            line,
            expected: ExpectedResponse::Ok,
        }
    }

    fn expect_screenshot(line: String) -> Self {
        Self {
            line,
            expected: ExpectedResponse::Screenshot,
        }
    }

    fn expect_window_list(line: String) -> Self {
        Self {
            line,
            expected: ExpectedResponse::WindowList,
        }
    }
}

#[derive(Debug)]
enum ObservedResponse {
    Ok,
    Screenshot(Screenshot),
    WindowList(Vec<WindowInfo>),
}

fn screenshot_plan(
    path: Option<&Path>,
    window: Option<u64>,
    delay_ms: u64,
) -> Result<Vec<PlannedCommand>, SessionError> {
    let mut plan = Vec::new();
    if delay_ms > 0 {
        let line = AutomationCommand::Sleep { ms: delay_ms }
            .to_autowc_line(window)
            .map_err(SessionError::new)?;
        plan.push(PlannedCommand::expect_ok(line));
    }

    let line = screenshot_line(path, window).map_err(SessionError::new)?;
    plan.push(PlannedCommand::expect_screenshot(line));
    Ok(plan)
}

async fn observe_response(
    response: AutowcResponse,
    expected: ExpectedResponse,
) -> Result<ObservedResponse, SessionError> {
    match (response, expected) {
        (AutowcResponse::Error(error), _) => Err(SessionError::new(error)),
        (AutowcResponse::Ok, ExpectedResponse::Ok) => Ok(ObservedResponse::Ok),
        (AutowcResponse::WindowList { windows }, ExpectedResponse::WindowList) => {
            Ok(ObservedResponse::WindowList(windows))
        }
        (AutowcResponse::Screenshot { path }, ExpectedResponse::Screenshot) => {
            let data_base64 =
                STANDARD.encode(fs::read(&path).await.map_err(|err| {
                    SessionError::new(format!("failed reading screenshot: {err}"))
                })?);
            Ok(ObservedResponse::Screenshot(Screenshot {
                path,
                mime_type: "image/png",
                data_base64,
            }))
        }
        (AutowcResponse::Ok, ExpectedResponse::Screenshot) => Err(SessionError::new(
            "unexpected ok response while awaiting screenshot",
        )),
        (AutowcResponse::Ok, ExpectedResponse::WindowList) => Err(SessionError::new(
            "unexpected ok response while awaiting window list",
        )),
        (AutowcResponse::Screenshot { .. }, ExpectedResponse::Ok) => Err(SessionError::new(
            "unexpected screenshot response while awaiting ok",
        )),
        (AutowcResponse::Screenshot { .. }, ExpectedResponse::WindowList) => Err(
            SessionError::new("unexpected screenshot response while awaiting window list"),
        ),
        (AutowcResponse::WindowList { .. }, ExpectedResponse::Ok) => Err(SessionError::new(
            "unexpected window list response while awaiting ok",
        )),
        (AutowcResponse::WindowList { .. }, ExpectedResponse::Screenshot) => Err(
            SessionError::new("unexpected window list response while awaiting screenshot"),
        ),
    }
}

fn parse_response(line: &str) -> Result<AutowcResponse, String> {
    let response: JsonAutowcResponse =
        serde_json::from_str(line).map_err(|err| format!("invalid AutoWC JSON response: {err}"))?;

    if let Some(error) = response.error {
        return Ok(AutowcResponse::Error(error));
    }
    if !response.ok {
        return Ok(AutowcResponse::Error(
            "AutoWC returned ok=false without an error message".into(),
        ));
    }

    if let Some(windows) = response.windows {
        return Ok(AutowcResponse::WindowList { windows });
    }

    match response.response_type.as_deref() {
        None => Ok(AutowcResponse::Ok),
        Some("screenshot") => {
            let path = response
                .path
                .ok_or_else(|| "AutoWC returned screenshot without a path".to_string())?;
            if path.as_os_str().is_empty() {
                return Err("AutoWC returned an empty screenshot path".into());
            }
            Ok(AutowcResponse::Screenshot { path })
        }
        Some(response_type) => Err(format!("unexpected AutoWC response type: {response_type}")),
    }
}

#[derive(Debug, Deserialize)]
struct JsonAutowcResponse {
    ok: bool,
    #[serde(rename = "type")]
    response_type: Option<String>,
    path: Option<PathBuf>,
    windows: Option<Vec<WindowInfo>>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_screenshot_response() {
        let response =
            parse_response(r#"{"ok":true,"type":"screenshot","path":"/tmp/autowc.png"}"#).unwrap();
        match response {
            AutowcResponse::Ok => panic!("expected screenshot response"),
            AutowcResponse::Error(err) => panic!("expected screenshot response, got {err}"),
            AutowcResponse::Screenshot { path } => {
                assert_eq!(path, PathBuf::from("/tmp/autowc.png"));
            }
            AutowcResponse::WindowList { .. } => panic!("expected screenshot response"),
        }
    }

    #[test]
    fn parses_ok_response() {
        assert!(matches!(
            parse_response(r#"{"ok":true}"#).unwrap(),
            AutowcResponse::Ok
        ));
    }

    #[test]
    fn parses_error_response() {
        match parse_response(r#"{"ok":false,"error":"unsupported key"}"#).unwrap() {
            AutowcResponse::Error(err) => assert_eq!(err, "unsupported key"),
            other => panic!("expected error response, got {other:?}"),
        }
    }

    #[test]
    fn plans_screenshot_without_hidden_sync_when_no_delay() {
        let plan = screenshot_plan(None, Some(4), 0).unwrap();

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].line, r#"{"type":"screenshot","window":4}"#);
        assert_eq!(plan[0].expected, ExpectedResponse::Screenshot);
    }

    #[test]
    fn plans_hidden_sleep_before_delayed_screenshot() {
        let plan = screenshot_plan(None, Some(4), 250).unwrap();

        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].line, r#"{"ms":250,"type":"sleep","window":4}"#);
        assert_eq!(plan[0].expected, ExpectedResponse::Ok);
        assert_eq!(plan[1].line, r#"{"type":"screenshot","window":4}"#);
        assert_eq!(plan[1].expected, ExpectedResponse::Screenshot);
    }

    #[tokio::test]
    async fn observes_autowc_error_as_session_error() {
        let err = observe_response(
            AutowcResponse::Error("unknown window: 9".into()),
            ExpectedResponse::Ok,
        )
        .await
        .unwrap_err();

        assert_eq!(err.message, "unknown window: 9");
    }

    #[tokio::test]
    async fn rejects_unexpected_ok_for_screenshot() {
        let err = observe_response(AutowcResponse::Ok, ExpectedResponse::Screenshot)
            .await
            .unwrap_err();

        assert_eq!(
            err.message,
            "unexpected ok response while awaiting screenshot"
        );
    }

    #[test]
    fn parses_window_list_response() {
        let response = parse_response(
            r#"{"ok":true,"windows":[{"id":2,"title":"Dialog","width":640,"height":480,"fixed":true}]}"#,
        )
        .unwrap();
        match response {
            AutowcResponse::WindowList { windows } => {
                assert_eq!(
                    windows,
                    vec![WindowInfo {
                        id: 2,
                        title: "Dialog".into(),
                        width: 640,
                        height: 480,
                        fixed: true,
                    }]
                );
            }
            other => panic!("expected window list response, got {other:?}"),
        }
    }

    #[test]
    fn builds_launch_args_with_stay_alive() {
        let args = autowc_args(&AutoWcSessionConfig {
            autowc_binary: "autowc".into(),
            command: vec!["gtk4-demo".into()],
            stay_alive: true,
        });

        assert_eq!(args, ["--json", "--stay-alive", "gtk4-demo"]);
    }

    #[test]
    fn omits_default_sizing_args() {
        let args = autowc_args(&AutoWcSessionConfig {
            autowc_binary: "autowc".into(),
            command: vec!["foot".into()],
            stay_alive: false,
        });

        assert_eq!(args, ["--json", "foot"]);
    }
}
