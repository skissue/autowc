use std::{
    io,
    thread::{self, JoinHandle},
};

use crate::{
    control::{parse_control_command, ControlCommand},
    protocol, AutoWC, EventLoop,
};
use smithay::reexports::calloop::{self, channel::Event};

pub fn init_stdin(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    let (tx, rx) = calloop::channel::channel();

    event_loop
        .handle()
        .insert_source(rx, move |event: Event<String>, _, state| {
            let Event::Msg(msg) = event else { return };

            match parse_control_command(&msg) {
                Ok(Some(command)) => {
                    let responds_with_screenshot =
                        matches!(command, ControlCommand::Screenshot { .. });
                    if let Err(err) = state.process_control_command(command) {
                        protocol::send(format!("error {err}"));
                    } else if !responds_with_screenshot {
                        protocol::send("ok");
                    }
                }
                Ok(None) => {}
                Err(err) => protocol::send(format!("error {err}")),
            }
        })?;

    let handle = thread::spawn(move || {
        for line in io::stdin().lines() {
            let Ok(line) = line else {
                break;
            };

            if tx.send(line).is_err() {
                break;
            }
        }
    });

    Ok(handle)
}
