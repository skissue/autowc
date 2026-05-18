#![allow(irrefutable_let_patterns)]

mod handlers;
mod input;
mod keycodes;
mod state;
mod stdin;
mod winit;

use std::{process::Child, time::Duration};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
pub use state::AutoWC;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let mut event_loop: EventLoop<AutoWC> = EventLoop::try_new()?;

    let display: Display<AutoWC> = Display::new()?;

    let mut state = AutoWC::new(&mut event_loop, display);

    // Open a Wayland/X11 window for our nested compositor
    crate::winit::init_winit(&mut event_loop, &mut state)?;

    // Set WAYLAND_DISPLAY to our socket name, so child processes connect to AutoWC rather
    // than the host compositor
    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);

    state.child = Some(spawn_client()?);
    init_child_watcher(&mut event_loop)?;

    crate::stdin::init_stdin(&mut event_loop)?;

    event_loop.run(None, &mut state, move |_| {
        // AutoWC is running
    })?;

    Ok(())
}

fn init_logging() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }
}

fn spawn_client() -> Result<Child, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        return Err("usage: autowc <command> [args...]".into());
    };

    Ok(std::process::Command::new(command).args(args).spawn()?)
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
