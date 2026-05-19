use std::{
    io,
    thread::{self, JoinHandle},
};

use crate::{control::parse_control_command, AutoWC, EventLoop};
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
                    if let Err(err) = state.process_control_command(command) {
                        eprintln!("control error: {err}");
                    }
                }
                Ok(None) => {}
                Err(err) => eprintln!("control parse error: {err}"),
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
