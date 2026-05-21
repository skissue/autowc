use std::time::Duration;

use crate::state::AutoWC;

mod host;
pub mod keyboard;
mod synthetic;

pub const CONTROL_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(5);
pub const DEFAULT_KEY_EVENT_INTERVAL_MS: u64 = 20;
pub const DEFAULT_CHORD_KEY_INTERVAL_MS: u64 = 10;
pub const DEFAULT_CHORD_HOLD_DURATION_MS: u64 = 75;
pub const DEFAULT_COMMAND_INTERVAL_MS: u64 = 0;

pub const DEFAULT_KEY_EVENT_INTERVAL: Duration =
    Duration::from_millis(DEFAULT_KEY_EVENT_INTERVAL_MS);
pub const DEFAULT_CHORD_KEY_INTERVAL: Duration =
    Duration::from_millis(DEFAULT_CHORD_KEY_INTERVAL_MS);
pub const DEFAULT_CHORD_HOLD_DURATION: Duration =
    Duration::from_millis(DEFAULT_CHORD_HOLD_DURATION_MS);
pub const DEFAULT_COMMAND_INTERVAL: Duration = Duration::from_millis(DEFAULT_COMMAND_INTERVAL_MS);

impl AutoWC {
    fn now_msec(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }
}
