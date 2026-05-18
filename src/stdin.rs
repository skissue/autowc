use std::{
    io,
    thread::{self, JoinHandle},
};

use crate::{keycodes::key_to_code, AutoWC, EventLoop};
use smithay::{
    backend::input::KeyState,
    reexports::calloop::{self, channel::Event},
};

pub fn init_stdin(
    event_loop: &mut EventLoop<AutoWC>,
) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    let (tx, rx) = calloop::channel::channel();

    event_loop
        .handle()
        .insert_source(rx, move |event: Event<String>, _, state| {
            let Event::Msg(msg) = event else { return };

            let key_code = key_to_code(&msg);
            state.process_virtual_input_event(key_code, KeyState::Pressed);
            state.process_virtual_input_event(key_code, KeyState::Released);
        })?;

    let handle = thread::spawn(move || {
        for line in io::stdin().lines() {
            let line = line.unwrap();

            tx.send(line.trim().to_string()).unwrap();
        }
    });

    Ok(handle)
}
