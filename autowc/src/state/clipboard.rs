use std::{
    ffi::OsString, io::Read, os::fd::OwnedFd, os::unix::net::UnixStream, sync::Mutex, thread,
};

use smithay::{
    input::Seat,
    wayland::selection::{
        data_device::request_data_device_client_selection, SelectionSource, SelectionTarget,
    },
};
use tracing::{debug, error, trace, warn};
use wl_clipboard_rs::copy::{
    clear, ClipboardType, MimeType as HostMimeType, Options as HostClipboardOptions,
    Seat as HostSeat, Source as HostClipboardSource,
};

use super::AutoWC;

static WAYLAND_DISPLAY_ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct PendingClipboardSync {
    seat: Seat<AutoWC>,
    mime_type: String,
}

impl AutoWC {
    pub fn sync_clipboard_to_host(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        seat: Seat<Self>,
    ) {
        if ty != SelectionTarget::Clipboard {
            trace!(?ty, "ignoring non-clipboard selection update");
            return;
        }

        let Some(source) = source else {
            debug!("nested clipboard source cleared");
            self.clear_host_clipboard();
            return;
        };

        let mime_types = source.mime_types();
        let Some(mime_type) = preferred_clipboard_mime_type(&mime_types) else {
            warn!("clipboard source advertised no mime types");
            return;
        };

        debug!(%mime_type, "queued clipboard sync to host");
        self.pending_clipboard_sync = Some(PendingClipboardSync { seat, mime_type });
    }

    pub fn process_pending_clipboard_sync(&mut self) {
        let Some(PendingClipboardSync { seat, mime_type }) = self.pending_clipboard_sync.take()
        else {
            return;
        };

        if let Err(err) = self.copy_selection_to_host_clipboard(seat, mime_type) {
            error!(error = %err, "failed to sync clipboard to host");
        }
    }

    pub fn reap_clipboard_sync_threads(&mut self) {
        let mut index = 0;
        while index < self.clipboard_sync_threads.len() {
            if self.clipboard_sync_threads[index].is_finished() {
                let thread = self.clipboard_sync_threads.swap_remove(index);
                if thread.join().is_err() {
                    error!("host clipboard sync thread panicked");
                }
            } else {
                index += 1;
            }
        }
    }

    fn copy_selection_to_host_clipboard(
        &mut self,
        seat: Seat<Self>,
        mime_type: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (mut read_end, write_end) = UnixStream::pair()?;
        let fd: OwnedFd = write_end.into();
        debug!(%mime_type, "requesting nested clipboard data");
        request_data_device_client_selection(&seat, mime_type.clone(), fd)?;

        let host_wayland_display = self.host_wayland_display.clone();
        let thread = thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Err(err) = read_end.read_to_end(&mut bytes) {
                error!(error = %err, "failed to read nested clipboard data");
                return;
            }

            debug!(byte_count = bytes.len(), %mime_type, "copying clipboard data to host");
            if let Err(err) = copy_bytes_to_host_clipboard(host_wayland_display, mime_type, bytes) {
                error!(error = %err, "failed to copy clipboard data to host");
            }
        });
        self.clipboard_sync_threads.push(thread);
        self.reap_clipboard_sync_threads();
        Ok(())
    }

    fn clear_host_clipboard(&mut self) {
        let host_wayland_display = self.host_wayland_display.clone();
        let thread = thread::spawn(move || {
            debug!("clearing host clipboard");
            let result = with_host_wayland_display(host_wayland_display, || {
                clear(ClipboardType::Regular, HostSeat::All)
            });
            if let Err(err) = result {
                error!(error = %err, "failed to clear host clipboard");
            }
        });
        self.clipboard_sync_threads.push(thread);
        self.reap_clipboard_sync_threads();
    }
}

fn copy_bytes_to_host_clipboard(
    host_wayland_display: Option<OsString>,
    mime_type: String,
    bytes: Vec<u8>,
) -> Result<(), wl_clipboard_rs::copy::Error> {
    with_host_wayland_display(host_wayland_display, || {
        let mut options = HostClipboardOptions::new();
        options.clipboard(ClipboardType::Regular);
        options.copy(
            HostClipboardSource::Bytes(bytes.into_boxed_slice()),
            HostMimeType::Specific(mime_type),
        )
    })
}

fn with_host_wayland_display<T>(
    host_wayland_display: Option<OsString>,
    f: impl FnOnce() -> T,
) -> T {
    let _guard = WAYLAND_DISPLAY_ENV_LOCK.lock().unwrap();
    let previous = std::env::var_os("WAYLAND_DISPLAY");

    match host_wayland_display {
        Some(display) => std::env::set_var("WAYLAND_DISPLAY", display),
        None => std::env::remove_var("WAYLAND_DISPLAY"),
    }

    let result = f();

    match previous {
        Some(display) => std::env::set_var("WAYLAND_DISPLAY", display),
        None => std::env::remove_var("WAYLAND_DISPLAY"),
    }

    result
}

fn preferred_clipboard_mime_type(mime_types: &[String]) -> Option<String> {
    ["text/plain;charset=utf-8", "text/plain"]
        .iter()
        .find(|mime| mime_types.iter().any(|candidate| candidate == **mime))
        .map(|mime| (*mime).to_string())
        .or_else(|| mime_types.first().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_utf8_plain_text_clipboard_mime_type() {
        let mime_types = vec!["text/html".into(), "text/plain;charset=utf-8".into()];

        assert_eq!(
            preferred_clipboard_mime_type(&mime_types),
            Some("text/plain;charset=utf-8".into())
        );
    }

    #[test]
    fn falls_back_to_first_clipboard_mime_type() {
        let mime_types = vec!["image/png".into(), "application/octet-stream".into()];

        assert_eq!(
            preferred_clipboard_mime_type(&mime_types),
            Some("image/png".into())
        );
    }

    #[test]
    fn restores_wayland_display_after_host_clipboard_call() {
        std::env::set_var("WAYLAND_DISPLAY", "nested-display");

        with_host_wayland_display(Some("host-display".into()), || {
            assert_eq!(
                std::env::var_os("WAYLAND_DISPLAY"),
                Some(OsString::from("host-display"))
            );
        });

        assert_eq!(
            std::env::var_os("WAYLAND_DISPLAY"),
            Some(OsString::from("nested-display"))
        );
    }
}
