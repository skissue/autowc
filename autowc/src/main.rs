#![allow(irrefutable_let_patterns)]

mod control;
mod handlers;
mod input;
mod keycodes;
mod protocol;
mod screenshot;
mod state;
mod stdin;
mod winit;

use std::{
    ffi::OsString,
    process::{Child, Command, Stdio},
    time::Duration,
};

use clap::Parser;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
use smithay::utils::{Logical, Size};
pub use state::{AutoWC, TimingOptions};

#[derive(Debug, Parser)]
#[command(name = "autowc", about = "A small nested compositor for automation")]
struct Cli {
    /// Virtual output width in logical pixels.
    #[arg(long, default_value_t = 1280)]
    width: i32,

    /// Virtual output height in logical pixels.
    #[arg(long, default_value_t = 720)]
    height: i32,

    /// Keep AutoWC running after all client windows close.
    #[arg(long)]
    stay_alive: bool,

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
    let virtual_size = virtual_size_from_cli(&cli)?;
    let timing = timing_from_cli(&cli);

    let mut event_loop: EventLoop<AutoWC> = EventLoop::try_new()?;

    let display: Display<AutoWC> = Display::new()?;

    let mut state = AutoWC::new(
        &mut event_loop,
        display,
        virtual_size,
        cli.stay_alive,
        timing,
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

fn virtual_size_from_cli(cli: &Cli) -> Result<Size<i32, Logical>, Box<dyn std::error::Error>> {
    if cli.width <= 0 || cli.height <= 0 {
        return Err("width and height must be positive".into());
    }

    Ok(Size::from((cli.width, cli.height)))
}

fn timing_from_cli(cli: &Cli) -> TimingOptions {
    TimingOptions {
        key_event_interval: Duration::from_millis(cli.key_event_interval_ms),
        chord_key_interval: Duration::from_millis(cli.chord_key_interval_ms),
        chord_hold_duration: Duration::from_millis(cli.chord_hold_ms),
        command_interval: Duration::from_millis(cli.command_interval_ms),
    }
}

fn spawn_client(command: &[OsString]) -> Result<Child, Box<dyn std::error::Error>> {
    let Some((program, args)) = command.split_first() else {
        return Err("missing launch command".into());
    };

    Ok(Command::new(program)
        .args(args)
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
