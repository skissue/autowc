#![allow(irrefutable_let_patterns)]

mod control;
mod host;
mod input;
mod keycodes;
mod protocol;
mod render;
mod screenshot;
mod state;
mod stdin;
mod wayland;
mod window;
mod winit;

use std::{
    ffi::OsString,
    process::{Child, Command, Stdio},
    time::Duration,
};

use clap::Parser;
use protocol::Protocol;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use smithay::utils::{Logical, Size};
pub use state::{AutoWC, TimingOptions};

const DEFAULT_INITIAL_WIDTH: i32 = 1280;
const DEFAULT_INITIAL_HEIGHT: i32 = 720;

#[derive(Debug, Parser)]
#[command(name = "autowc", about = "A small nested compositor for automation")]
struct Cli {
    /// Virtual output width in logical pixels.
    #[arg(long)]
    width: Option<i32>,

    /// Virtual output height in logical pixels.
    #[arg(long)]
    height: Option<i32>,

    /// Resize the virtual output to match the host window.
    #[arg(long)]
    dynamic_resize: bool,

    /// Keep AutoWC running after all client windows close.
    #[arg(long)]
    stay_alive: bool,

    /// Use newline-delimited JSON for stdin commands and stdout responses.
    #[arg(long)]
    json: bool,

    /// Delay between key press/release events for key and text commands, in milliseconds.
    #[arg(long, default_value_t = crate::input::DEFAULT_KEY_EVENT_INTERVAL_MS)]
    key_event_interval_ms: u64,

    /// Delay between pressing each key in a chord command, in milliseconds.
    #[arg(long, default_value_t = crate::input::DEFAULT_CHORD_KEY_INTERVAL_MS)]
    chord_key_interval_ms: u64,

    /// Time to hold a chord after all chord keys are pressed, in milliseconds.
    #[arg(long, default_value_t = crate::input::DEFAULT_CHORD_HOLD_DURATION_MS)]
    chord_hold_ms: u64,

    /// Delay after each stdin command, in milliseconds.
    #[arg(long, default_value_t = crate::input::DEFAULT_COMMAND_INTERVAL_MS)]
    command_interval_ms: u64,

    /// Command to launch inside AutoWC, followed by its arguments.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    let cli = Cli::parse();
    let output_sizing = output_sizing_from_cli(&cli)?;
    let timing = timing_from_cli(&cli);
    let protocol = protocol_from_cli(&cli);

    let mut event_loop: EventLoop<AutoWC> = EventLoop::try_new()?;

    let display: Display<AutoWC> = Display::new()?;

    let mut state = AutoWC::new(
        &mut event_loop,
        display,
        output_sizing.initial_size,
        output_sizing.dynamic_resize,
        cli.stay_alive,
        timing,
        protocol,
    );

    // Open a Wayland/X11 window for our nested compositor
    crate::winit::init_winit(&mut event_loop, &mut state)?;

    // Set WAYLAND_DISPLAY to our socket name, so child processes connect to AutoWC rather
    // than the host compositor
    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);

    state.child = Some(spawn_client(&cli.command)?);
    init_child_watcher(&mut event_loop)?;
    init_control_scheduler(&mut event_loop)?;

    crate::stdin::init_stdin(&mut event_loop)?;

    event_loop.run(None, &mut state, move |_| {
        // AutoWC is running
    })?;

    Ok(())
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .init();
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OutputSizing {
    initial_size: Size<i32, Logical>,
    dynamic_resize: bool,
}

fn output_sizing_from_cli(cli: &Cli) -> Result<OutputSizing, Box<dyn std::error::Error>> {
    let has_width = cli.width.is_some();
    let has_height = cli.height.is_some();

    if cli.dynamic_resize && (has_width || has_height) {
        return Err("dynamic resize cannot be used with explicit width or height".into());
    }

    let dynamic_resize = cli.dynamic_resize || (!has_width && !has_height);
    if dynamic_resize {
        return Ok(OutputSizing {
            initial_size: Size::from((DEFAULT_INITIAL_WIDTH, DEFAULT_INITIAL_HEIGHT)),
            dynamic_resize,
        });
    }

    let (Some(width), Some(height)) = (cli.width, cli.height) else {
        return Err("width and height must be specified together".into());
    };

    if width <= 0 || height <= 0 {
        return Err("width and height must be positive".into());
    }

    Ok(OutputSizing {
        initial_size: Size::from((width, height)),
        dynamic_resize,
    })
}

fn timing_from_cli(cli: &Cli) -> TimingOptions {
    TimingOptions {
        key_event_interval: Duration::from_millis(cli.key_event_interval_ms),
        chord_key_interval: Duration::from_millis(cli.chord_key_interval_ms),
        chord_hold_duration: Duration::from_millis(cli.chord_hold_ms),
        command_interval: Duration::from_millis(cli.command_interval_ms),
    }
}

fn protocol_from_cli(cli: &Cli) -> Protocol {
    if cli.json {
        Protocol::Json
    } else {
        Protocol::Plain
    }
}

fn spawn_client(command: &[OsString]) -> Result<Child, Box<dyn std::error::Error>> {
    let Some((program, args)) = command.split_first() else {
        return Err("missing launch command".into());
    };

    Ok(Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

fn init_child_watcher(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<(), Box<dyn std::error::Error>> {
    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(100)),
        |_, _, state| {
            state.check_child_exit();
            TimeoutAction::ToDuration(Duration::from_millis(100))
        },
    )?;

    Ok(())
}

fn init_control_scheduler(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<(), Box<dyn std::error::Error>> {
    event_loop.handle().insert_source(
        Timer::from_duration(crate::input::CONTROL_QUEUE_POLL_INTERVAL),
        |_, _, state| {
            state.process_pending_control_actions();
            TimeoutAction::ToDuration(crate::input::CONTROL_QUEUE_POLL_INTERVAL)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn output_sizing_defaults_to_dynamic_without_resolution_options() {
        let cli = parse(&["autowc", "foot"]);

        assert_eq!(
            output_sizing_from_cli(&cli).unwrap(),
            OutputSizing {
                initial_size: Size::from((DEFAULT_INITIAL_WIDTH, DEFAULT_INITIAL_HEIGHT)),
                dynamic_resize: true,
            }
        );
    }

    #[test]
    fn output_sizing_uses_fixed_size_when_resolution_options_are_specified() {
        let cli = parse(&["autowc", "--width", "800", "--height", "600", "foot"]);

        assert_eq!(
            output_sizing_from_cli(&cli).unwrap(),
            OutputSizing {
                initial_size: Size::from((800, 600)),
                dynamic_resize: false,
            }
        );
    }

    #[test]
    fn output_sizing_rejects_partial_resolution_options() {
        let cli = parse(&["autowc", "--width", "800", "foot"]);

        assert_eq!(
            output_sizing_from_cli(&cli).unwrap_err().to_string(),
            "width and height must be specified together"
        );
    }

    #[test]
    fn output_sizing_rejects_dynamic_resize_with_resolution_options() {
        let cli = parse(&[
            "autowc",
            "--dynamic-resize",
            "--width",
            "800",
            "--height",
            "600",
            "foot",
        ]);

        assert_eq!(
            output_sizing_from_cli(&cli).unwrap_err().to_string(),
            "dynamic resize cannot be used with explicit width or height"
        );
    }

    #[test]
    fn output_sizing_rejects_non_positive_fixed_size() {
        let cli = parse(&["autowc", "--width", "0", "--height", "600", "foot"]);

        assert_eq!(
            output_sizing_from_cli(&cli).unwrap_err().to_string(),
            "width and height must be positive"
        );
    }
}
