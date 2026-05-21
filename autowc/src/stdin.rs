use std::{
    collections::HashSet,
    io,
    thread::{self, JoinHandle},
};

use crate::{AutoWC, EventLoop};
use smithay::reexports::calloop::{self, channel::Event};

pub fn init_stdin(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    let (tx, rx) = calloop::channel::channel();

    event_loop
        .handle()
        .insert_source(rx, move |event: Event<String>, _, state| {
            let Event::Msg(msg) = event else { return };
            let protocol = state.protocol;

            match protocol.parse_control_command(&msg) {
                Ok(Some(command)) => {
                    let responds_with_screenshot = command.responds_with_screenshot();
                    let before_windows = state
                        .mapped_window_ids()
                        .into_iter()
                        .collect::<HashSet<_>>();
                    if let Err(err) = state.process_control_command(command) {
                        protocol.send_error(err);
                    } else if !responds_with_screenshot {
                        let new_windows = state
                            .mapped_window_ids()
                            .into_iter()
                            .filter(|window_id| !before_windows.contains(window_id))
                            .filter_map(|window_id| state.window_info(window_id))
                            .collect::<Vec<_>>();
                        protocol.send_ok_with_new_windows(&new_windows);
                    }
                }
                Ok(None) => {}
                Err(err) => protocol.send_error(err),
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
