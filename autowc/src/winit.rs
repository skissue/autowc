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
    let window_attributes = WinitWindow::default_attributes()
        .with_inner_size(LogicalSize::new(
            state.initial_virtual_size.w as f64,
            state.initial_virtual_size.h as f64,
        ))
        .with_title("AutoWC Bootstrap")
        .with_visible(false);
    let (mut backend, host_events, requester) =
        host::init_from_attributes(window_attributes)?;
    state.set_host_window_requester(requester);

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
        } => handle_window_created(
            state,
            backend,
            render_windows,
            auto_window_id,
            window_id,
            window,
            size,
            scale_factor,
        ),
        HostEvent::WindowCreateFailed {
            auto_window_id,
            error,
        } => {
            eprintln!("AutoWC failed to create host window {auto_window_id:?}: {error}");
        }
        HostEvent::WindowClosed { window_id } => {
            backend.remove_window(window_id);
            render_windows.remove_host_window(window_id);
        }
        HostEvent::Resized {
            window_id,
            size,
            scale_factor,
            ..
        } => {
            render_windows.resize_host_window(state, window_id, size, scale_factor);
        }
        HostEvent::Input { window_id, event } => {
            if let Some(auto_window_id) = render_windows.auto_window_id(window_id) {
                state.process_input_event(auto_window_id, event);
            }
        }
        HostEvent::Redraw { window_id } => {
            render_windows.redraw_host_window(state, backend, window_id);
        }
        HostEvent::CloseRequested { window_id } => {
            state.close_host_window(window_id);
        }
        HostEvent::Focus { window_id, focused } => {
            let Some(auto_window_id) = render_windows.auto_window_id(window_id) else {
                return;
            };
            if focused {
                state.focus_auto_window(auto_window_id);
            } else {
                state.blur_auto_window(auto_window_id);
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
) {
    if let Err(err) = backend.add_window(window) {
        eprintln!("AutoWC failed to initialize host window renderer: {err}");
        return;
    }

    render_windows.add_host_window(
        state,
        host_window_id,
        auto_window_id,
        size,
        scale_factor,
    );
    backend.window(host_window_id).request_redraw();
}
