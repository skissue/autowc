use std::{
    io,
    thread::{self, JoinHandle},
};

use crate::{protocol::ControlResponse, AutoWC, EventLoop};
use smithay::reexports::calloop::{self, channel::Event};
use tracing::{debug, trace, warn};

pub fn init_stdin(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    let (tx, rx) = calloop::channel::channel();

    event_loop
        .handle()
        .insert_source(rx, move |event: Event<String>, _, state| {
            let Event::Msg(msg) = event else { return };
            let protocol = state.protocol;
            trace!(line = %msg, "received control input");

            match protocol.parse_control_command(&msg) {
                Ok(Some(command)) => {
                    debug!(?command, "parsed control command");
                    let response = state.begin_control_response();
                    state.process_control_command(command, response);
                    state.flush_control_responses();
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(error = %err, "failed to parse control command");
                    let response = state.begin_control_response();
                    state.complete_control_response(response, ControlResponse::Error(err));
                    state.flush_control_responses();
                }
            }
        })?;

    let handle = thread::spawn(move || {
        for line in io::stdin().lines() {
            let Ok(line) = line else {
                warn!("failed to read control input from stdin");
                break;
            };

            if tx.send(line).is_err() {
                debug!("control input channel closed");
                break;
            }
        }
    });

    Ok(handle)
}
