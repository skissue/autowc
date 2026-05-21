use std::{
    io,
    thread::{self, JoinHandle},
};

use crate::{control::ControlCommandVariant, AutoWC, EventLoop};
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
                    if command.variant == ControlCommandVariant::List {
                        let windows = state.window_infos();
                        debug!(
                            window_count = windows.len(),
                            "responding to window list command"
                        );
                        protocol.send_window_list(&windows);
                        return;
                    }

                    let responds_with_screenshot = command.responds_with_screenshot();
                    if let Err(err) = state.process_control_command(command) {
                        warn!(error = %err, "control command failed");
                        protocol.send_error(err);
                    } else if !responds_with_screenshot {
                        debug!("control command completed");
                        protocol.send_ok();
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(error = %err, "failed to parse control command");
                    protocol.send_error(err);
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
