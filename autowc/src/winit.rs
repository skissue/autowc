use std::sync::Arc;

use smithay::{
    reexports::{
        calloop::EventLoop,
        winit::{
            dpi::LogicalSize,
            window::{Window as WinitWindow, WindowId as HostWindowId},
        },
    },
    utils::{Physical, Size},
};
use tracing::{debug, error, info, trace, warn};

use crate::{
    host::{self, HostEvent},
    render::RenderWindows,
    window::AutoWindowId,
    AutoWC,
};

pub fn init_winit(
    event_loop: &mut EventLoop<AutoWC>,
    state: &mut AutoWC,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!(
        size = ?state.initial_virtual_size,
        "initializing host winit backend"
    );
    let window_attributes = WinitWindow::default_attributes()
        .with_inner_size(LogicalSize::new(
            state.initial_virtual_size.w as f64,
            state.initial_virtual_size.h as f64,
        ))
        .with_title("AutoWC Bootstrap")
        .with_visible(false);
    let (mut backend, host_events, requester) = host::init_from_attributes(window_attributes)?;
    state.set_host_window_requester(requester);
    debug!("host window requester installed");

    let mut render_windows = RenderWindows::new();

    event_loop
        .handle()
        .insert_source(host_events, move |event, _, state| {
            handle_host_event(state, &mut backend, &mut render_windows, event);
        })?;

    Ok(())
}

fn handle_host_event(
    state: &mut AutoWC,
    backend: &mut host::HostGraphicsBackend,
    render_windows: &mut RenderWindows,
    event: HostEvent,
) {
    match event {
        HostEvent::WindowCreated {
            auto_window_id,
            window_id,
            window,
            size,
            scale_factor,
            fullscreen,
        } => handle_window_created(
            state,
            backend,
            render_windows,
            auto_window_id,
            window_id,
            window,
            size,
            scale_factor,
            fullscreen,
        ),
        HostEvent::WindowCreateFailed {
            auto_window_id,
            error,
        } => {
            error!(?auto_window_id, %error, "failed to create host window");
        }
        HostEvent::WindowClosed { window_id } => {
            info!(?window_id, "host window closed");
            backend.remove_window(window_id);
            render_windows.remove_host_window(window_id);
        }
        HostEvent::Resized {
            window_id,
            size,
            scale_factor,
            fullscreen,
            ..
        } => {
            debug!(
                ?window_id,
                ?size,
                scale_factor,
                fullscreen,
                "host window resized"
            );
            render_windows.resize_host_window(state, window_id, size, scale_factor, fullscreen);
        }
        HostEvent::FullscreenChanged {
            window_id,
            fullscreen,
        } => {
            debug!(?window_id, fullscreen, "host fullscreen changed");
            render_windows.sync_host_fullscreen(state, window_id, fullscreen);
        }
        HostEvent::Input { window_id, event } => {
            let Some(auto_window_id) = render_windows.auto_window_id(window_id) else {
                warn!(?window_id, "received input for unknown host window");
                return;
            };
            if state.has_pending_control_actions() {
                if state.should_process_blocked_host_input(auto_window_id, &event) {
                    trace!(
                        ?window_id,
                        ?auto_window_id,
                        ?event,
                        "processing blocked host key release"
                    );
                } else {
                    trace!(
                        ?window_id,
                        "ignoring host input while synthetic control actions are pending"
                    );
                    return;
                }
            }
            trace!(?window_id, ?auto_window_id, ?event, "processing host input");
            state.process_input_event(auto_window_id, event);
        }
        HostEvent::Redraw { window_id } => {
            trace!(?window_id, "redrawing host window");
            render_windows.redraw_host_window(state, backend, window_id);
        }
        HostEvent::CloseRequested { window_id } => {
            info!(?window_id, "host window close requested");
            state.close_host_window(window_id);
        }
        HostEvent::Focus { window_id, focused } => {
            let Some(auto_window_id) = render_windows.auto_window_id(window_id) else {
                warn!(
                    ?window_id,
                    focused, "received focus event for unknown host window"
                );
                return;
            };
            if !focused {
                state.release_pressed_host_keys(auto_window_id);
            }
            if state.has_pending_control_actions() {
                trace!(
                    ?window_id,
                    focused,
                    "ignoring host focus while synthetic control actions are pending"
                );
                return;
            }
            if focused {
                debug!(?window_id, ?auto_window_id, "host window focused");
                state.focus_auto_window(auto_window_id);
            } else {
                trace!(?window_id, ?auto_window_id, "host window unfocused");
            }
        }
    }
}

fn handle_window_created(
    state: &mut AutoWC,
    backend: &mut host::HostGraphicsBackend,
    render_windows: &mut RenderWindows,
    auto_window_id: AutoWindowId,
    host_window_id: HostWindowId,
    window: Arc<WinitWindow>,
    size: Size<i32, Physical>,
    scale_factor: f64,
    fullscreen: bool,
) {
    if let Err(err) = backend.add_window(window) {
        error!(?auto_window_id, ?host_window_id, error = %err, "failed to initialize host window renderer");
        return;
    }

    info!(
        ?auto_window_id,
        ?host_window_id,
        ?size,
        scale_factor,
        fullscreen,
        "host window created"
    );
    render_windows.add_host_window(
        state,
        host_window_id,
        auto_window_id,
        size,
        scale_factor,
        fullscreen,
    );
    backend.window(host_window_id).request_redraw();
}
