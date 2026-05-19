use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex as StdMutex},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};
use uuid::Uuid;

use crate::command::{screenshot_line, AutomationCommand};

const STDERR_LINE_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub autowc_binary: PathBuf,
    pub command: Vec<String>,
    pub width: u32,
    pub height: u32,
    pub stay_alive: bool,
    pub key_event_interval_ms: Option<u64>,
    pub chord_key_interval_ms: Option<u64>,
    pub chord_hold_ms: Option<u64>,
    pub command_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Screenshot {
    pub path: PathBuf,
    pub mime_type: &'static str,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub width: u32,
    pub height: u32,
    pub command: Vec<String>,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub commands_executed: usize,
    pub screenshot: Option<Screenshot>,
}

#[derive(Debug)]
pub struct RunError {
    pub error: SessionError,
    pub commands_executed: usize,
    pub screenshot: Option<Screenshot>,
}

impl RunError {
    fn new(error: SessionError) -> Self {
        Self {
            error,
            commands_executed: 0,
            screenshot: None,
        }
    }
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

#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<Session>>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn launch(&self, config: SessionConfig) -> Result<SessionMetadata, SessionError> {
        if config.command.is_empty() {
            return Err(SessionError::new("launch command cannot be empty"));
        }
        if config.width == 0 || config.height == 0 {
            return Err(SessionError::new("width and height must be positive"));
        }

        let id = Uuid::new_v4().to_string();
        let metadata = SessionMetadata {
            session_id: id.clone(),
            width: config.width,
            height: config.height,
            command: config.command.clone(),
        };
        let session = Session::spawn(config).await?;

        self.sessions
            .lock()
            .await
            .insert(id, Arc::new(Mutex::new(session)));

        Ok(metadata)
    }

    pub async fn run(
        &self,
        session_id: &str,
        commands: &[AutomationCommand],
        return_screenshot: bool,
    ) -> Result<RunOutcome, RunError> {
        let session = self.get_session(session_id).await.map_err(RunError::new)?;
        let mut session = session.lock().await;
        session.run(commands, return_screenshot).await
    }

    pub async fn screenshot(
        &self,
        session_id: &str,
        path: Option<&Path>,
    ) -> Result<Screenshot, SessionError> {
        let session = self.get_session(session_id).await?;
        let mut session = session.lock().await;
        session.screenshot(path, true).await
    }

    pub async fn close(&self, session_id: &str) -> Result<bool, SessionError> {
        let Some(session) = self.sessions.lock().await.remove(session_id) else {
            return Ok(false);
        };

        let mut session = session.lock().await;
        session.close().await;
        Ok(true)
    }

    async fn get_session(&self, session_id: &str) -> Result<Arc<Mutex<Session>>, SessionError> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| SessionError::new(format!("unknown session: {session_id}")))
    }
}

#[derive(Debug)]
struct Session {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr: SharedStderr,
    exit_status: Option<ExitStatus>,
}

impl Session {
    async fn spawn(config: SessionConfig) -> Result<Self, SessionError> {
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

    async fn run(
        &mut self,
        commands: &[AutomationCommand],
        return_screenshot: bool,
    ) -> Result<RunOutcome, RunError> {
        self.ensure_running().await.map_err(RunError::new)?;
        let mut commands_executed = 0;

        for command in commands {
            let line = match command.to_autowc_line().map_err(SessionError::new) {
                Ok(line) => line,
                Err(error) => {
                    return Err(self
                        .run_error(commands_executed, error, return_screenshot)
                        .await)
                }
            };

            if let Err(error) = self.write_command_and_expect_ok(&line).await {
                return Err(self
                    .run_error(commands_executed, error, return_screenshot)
                    .await);
            }

            commands_executed += 1;
        }

        // Use a screenshot as a protocol sync point even when the caller does
        // not want the image back; otherwise command errors could remain unread.
        let screenshot = self
            .screenshot(None, return_screenshot)
            .await
            .map_err(|error| RunError {
                error,
                commands_executed,
                screenshot: None,
            })?;

        Ok(RunOutcome {
            commands_executed,
            screenshot: return_screenshot.then_some(screenshot),
        })
    }

    async fn run_error(
        &mut self,
        commands_executed: usize,
        error: SessionError,
        return_screenshot: bool,
    ) -> RunError {
        let screenshot = if return_screenshot {
            self.screenshot(None, true).await.ok()
        } else {
            None
        };

        RunError {
            error,
            commands_executed,
            screenshot,
        }
    }

    async fn write_command_and_expect_ok(&mut self, line: &str) -> Result<(), SessionError> {
        self.write_line(line).await?;
        match self.read_response().await? {
            AutowcResponse::Ok => Ok(()),
            AutowcResponse::Screenshot { .. } => Err(SessionError::new(
                "unexpected screenshot response while awaiting ok",
            )),
        }
    }

    async fn screenshot(
        &mut self,
        path: Option<&Path>,
        include_data: bool,
    ) -> Result<Screenshot, SessionError> {
        self.ensure_running().await?;
        let line = screenshot_line(path).map_err(SessionError::new)?;
        self.write_line(&line).await?;

        loop {
            match self.read_response().await? {
                AutowcResponse::Ok => continue,
                AutowcResponse::Screenshot { path } => {
                    let data_base64 = if include_data {
                        STANDARD.encode(fs::read(&path).await.map_err(|err| {
                            SessionError::new(format!("failed reading screenshot: {err}"))
                        })?)
                    } else {
                        let _ = fs::remove_file(&path).await;
                        String::new()
                    };
                    return Ok(Screenshot {
                        path,
                        mime_type: "image/png",
                        data_base64,
                    });
                }
            }
        }
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

    async fn close(&mut self) {
        let _ = self.write_line(r#"{"type":"quit"}"#).await;
        if timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
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

fn autowc_args(config: &SessionConfig) -> Vec<String> {
    let mut args = vec![
        "--json".into(),
        "--width".into(),
        config.width.to_string(),
        "--height".into(),
        config.height.to_string(),
    ];

    if config.stay_alive {
        args.push("--stay-alive".into());
    }
    push_optional_ms_arg(
        &mut args,
        "--key-event-interval-ms",
        config.key_event_interval_ms,
    );
    push_optional_ms_arg(
        &mut args,
        "--chord-key-interval-ms",
        config.chord_key_interval_ms,
    );
    push_optional_ms_arg(&mut args, "--chord-hold-ms", config.chord_hold_ms);
    push_optional_ms_arg(
        &mut args,
        "--command-interval-ms",
        config.command_interval_ms,
    );
    args.extend(config.command.clone());
    args
}

fn push_optional_ms_arg(args: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        args.push(flag.into());
        args.push(value.to_string());
    }
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
    Screenshot { path: PathBuf },
}

fn parse_response(line: &str) -> Result<AutowcResponse, String> {
    let response: JsonAutowcResponse =
        serde_json::from_str(line).map_err(|err| format!("invalid AutoWC JSON response: {err}"))?;

    if let Some(error) = response.error {
        return Err(error);
    }
    if !response.ok {
        return Err("AutoWC returned ok=false without an error message".into());
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
            AutowcResponse::Screenshot { path } => {
                assert_eq!(path, PathBuf::from("/tmp/autowc.png"));
            }
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
        assert_eq!(
            parse_response(r#"{"ok":false,"error":"unsupported key"}"#).unwrap_err(),
            "unsupported key"
        );
    }

    #[test]
    fn builds_launch_args_with_timing_options() {
        let args = autowc_args(&SessionConfig {
            autowc_binary: "autowc".into(),
            command: vec!["gtk4-demo".into()],
            width: 800,
            height: 600,
            stay_alive: true,
            key_event_interval_ms: Some(25),
            chord_key_interval_ms: Some(10),
            chord_hold_ms: Some(90),
            command_interval_ms: Some(5),
        });

        assert_eq!(
            args,
            [
                "--json",
                "--width",
                "800",
                "--height",
                "600",
                "--stay-alive",
                "--key-event-interval-ms",
                "25",
                "--chord-key-interval-ms",
                "10",
                "--chord-hold-ms",
                "90",
                "--command-interval-ms",
                "5",
                "gtk4-demo",
            ]
        );
    }

    #[test]
    fn omits_unset_launch_timing_options() {
        let args = autowc_args(&SessionConfig {
            autowc_binary: "autowc".into(),
            command: vec!["foot".into()],
            width: 1280,
            height: 720,
            stay_alive: false,
            key_event_interval_ms: None,
            chord_key_interval_ms: None,
            chord_hold_ms: None,
            command_interval_ms: None,
        });

        assert_eq!(
            args,
            ["--json", "--width", "1280", "--height", "720", "foot"]
        );
    }
}
