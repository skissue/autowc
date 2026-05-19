use std::{
    collections::VecDeque,
    ffi::OsString,
    path::PathBuf,
    process::Child,
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    backend::input::{ButtonState, KeyState},
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

pub struct AutoWC {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,
    pub virtual_size: Size<i32, Logical>,
    pub primary_window: Option<Window>,
    pub overlay_windows: Vec<Window>,
    pub host_size: Size<i32, Physical>,
    pub pointer_in_viewport: bool,
    pub child: Option<Child>,
    pub stay_alive: bool,
    pub pending_screenshots: VecDeque<ScreenshotRequest>,
    pub control_queue: VecDeque<QueuedControlAction>,
    pub next_control_action_at: Option<Instant>,
    screenshot_counter: u64,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<AutoWC>,
    pub data_device_state: DataDeviceState,
    pub popups: PopupManager,

    pub seat: Seat<Self>,
}

impl AutoWC {
    pub fn new(
        event_loop: &mut EventLoop<Self>,
        display: Display<Self>,
        virtual_size: Size<i32, Logical>,
        stay_alive: bool,
    ) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        // Here we initialize implementations of some wayland protocols
        // Some of them require us to implement traits on the AutoWC state,
        // you can find those implementations in the `crate::handlers` module

        // Initialize protocols needed for displaying windows
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        // Data device is responsible for clipboard and drag-and-drop
        let data_device_state = DataDeviceState::new::<Self>(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        // Setup a wayland socket that will be used to accept clients
        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,

            space,
            loop_signal,
            socket_name,
            virtual_size,
            primary_window: None,
            overlay_windows: Vec::new(),
            host_size: virtual_size.to_physical(1),
            pointer_in_viewport: false,
            child: None,
            stay_alive,
            pending_screenshots: VecDeque::new(),
            control_queue: VecDeque::new(),
            next_control_action_at: None,
            screenshot_counter: 0,

            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            popups,
            seat,
        }
    }

    fn init_wayland_listener(
        display: Display<AutoWC>,
        event_loop: &mut EventLoop<Self>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }

    pub fn set_host_size(&mut self, size: Size<i32, Physical>) {
        self.host_size = size;
    }

    pub fn presentation_viewport(&self) -> Rectangle<i32, Physical> {
        let host_size = self.host_size;
        if host_size.w <= 0 || host_size.h <= 0 {
            return Rectangle::from_size((0, 0).into());
        }

        let scale_x = host_size.w as f64 / self.virtual_size.w as f64;
        let scale_y = host_size.h as f64 / self.virtual_size.h as f64;
        let scale = scale_x.min(scale_y);

        let width = (self.virtual_size.w as f64 * scale).round() as i32;
        let height = (self.virtual_size.h as f64 * scale).round() as i32;
        let x = (host_size.w - width) / 2;
        let y = (host_size.h - height) / 2;

        Rectangle::new((x, y).into(), (width, height).into())
    }

    pub fn host_to_virtual(&self, pos: Point<f64, Physical>) -> Option<Point<f64, Logical>> {
        let viewport = self.presentation_viewport();
        if viewport.size.w <= 0 || viewport.size.h <= 0 {
            return None;
        }

        let x = pos.x - viewport.loc.x as f64;
        let y = pos.y - viewport.loc.y as f64;
        if x < 0.0 || y < 0.0 || x >= viewport.size.w as f64 || y >= viewport.size.h as f64 {
            return None;
        }

        let scale_x = viewport.size.w as f64 / self.virtual_size.w as f64;
        let scale_y = viewport.size.h as f64 / self.virtual_size.h as f64;
        Some(Point::from((x / scale_x, y / scale_y)))
    }

    pub fn map_new_toplevel(&mut self, window: Window) {
        if self.primary_window.is_none() {
            self.configure_primary(&window);
            self.space.map_element(window.clone(), (0, 0), false);
            self.primary_window = Some(window.clone());
            self.focus_window(Some(&window));
            return;
        }

        self.configure_overlay(&window);
        self.space.map_element(window.clone(), (0, 0), false);
        self.overlay_windows.push(window.clone());
        self.focus_window(Some(&window));
    }

    pub fn remove_toplevel(&mut self, surface: &WlSurface) {
        if self
            .primary_window
            .as_ref()
            .is_some_and(|window| window.toplevel().unwrap().wl_surface() == surface)
        {
            if let Some(window) = self.primary_window.take() {
                self.space.unmap_elem(&window);
            }
            self.promote_overlay();
            self.maybe_exit_when_empty();
            return;
        }

        if let Some(index) = self
            .overlay_windows
            .iter()
            .position(|window| window.toplevel().unwrap().wl_surface() == surface)
        {
            let window = self.overlay_windows.remove(index);
            self.space.unmap_elem(&window);
            let next_focus = self
                .overlay_windows
                .last()
                .cloned()
                .or_else(|| self.primary_window.clone());
            self.focus_window(next_focus.as_ref());
        }

        self.maybe_exit_when_empty();
    }

    pub fn check_child_exit(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("AutoWC child exited with {status}");
                self.child = None;
                self.maybe_exit_when_empty();
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("AutoWC failed to poll child process: {err}");
                self.child = None;
                self.maybe_exit_when_empty();
            }
        }
    }

    pub fn handle_toplevel_commit(&mut self, surface: &WlSurface) {
        if let Some(window) = self
            .overlay_windows
            .iter()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
        {
            self.center_overlay(&window);
        }
    }

    pub fn configure_primary(&self, window: &Window) {
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.states.set(xdg_toplevel::State::TiledLeft);
            state.states.set(xdg_toplevel::State::TiledRight);
            state.states.set(xdg_toplevel::State::TiledTop);
            state.states.set(xdg_toplevel::State::TiledBottom);
            state.size = Some(self.virtual_size);
        });
        window.set_activated(true);
    }

    pub fn configure_overlay(&self, window: &Window) {
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.states.unset(xdg_toplevel::State::TiledLeft);
            state.states.unset(xdg_toplevel::State::TiledRight);
            state.states.unset(xdg_toplevel::State::TiledTop);
            state.states.unset(xdg_toplevel::State::TiledBottom);
            state.size = None;
        });
        window.set_activated(true);
    }

    pub fn focus_window(&mut self, window: Option<&Window>) {
        let surface = window.map(|window| window.toplevel().unwrap().wl_surface().clone());
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.set_focus(self, surface, serial);
    }

    pub fn queue_screenshot(&mut self, path: Option<PathBuf>) {
        let path = path.unwrap_or_else(|| self.next_screenshot_path());
        self.pending_screenshots
            .push_back(ScreenshotRequest { path });
    }

    fn next_screenshot_path(&mut self) -> PathBuf {
        let pid = std::process::id();

        loop {
            self.screenshot_counter += 1;
            let path = std::env::temp_dir().join(format!(
                "autowc-screenshot-{pid}-{}.png",
                self.screenshot_counter
            ));
            if !path.exists() {
                return path;
            }
        }
    }

    fn promote_overlay(&mut self) {
        let Some(window) = self.overlay_windows.pop() else {
            self.focus_window(None);
            return;
        };

        self.configure_primary(&window);
        self.space.map_element(window.clone(), (0, 0), false);
        self.primary_window = Some(window.clone());
        window.toplevel().unwrap().send_pending_configure();

        let overlays = self.overlay_windows.clone();
        for overlay in overlays {
            self.center_overlay(&overlay);
        }

        self.focus_window(Some(&window));
    }

    fn maybe_exit_when_empty(&mut self) {
        if !self.stay_alive && self.primary_window.is_none() && self.overlay_windows.is_empty() {
            self.loop_signal.stop();
        }
    }

    fn center_overlay(&mut self, window: &Window) {
        let geometry = window.geometry();
        if geometry.size.w <= 0 || geometry.size.h <= 0 {
            return;
        }

        // TODO: Replace this with a real overlay policy. This should eventually
        // consider xdg parent/transient relationships and clamp to the output.
        let x = ((self.virtual_size.w - geometry.size.w) / 2).max(0);
        let y = ((self.virtual_size.h - geometry.size.h) / 2).max(0);
        self.space.map_element(window.clone(), (x, y), false);
    }
}

/// Data associated with a wayland client that connects to AutoWC.
/// One instance of this type per client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

pub struct ScreenshotRequest {
    pub path: PathBuf,
}

pub enum QueuedControlAction {
    Key { code: u32, state: KeyState },
    PointerMove { x: f64, y: f64 },
    PointerButton { button: u32, state: ButtonState },
    Scroll { dx: f64, dy: f64 },
    Screenshot { path: Option<PathBuf> },
    Quit,
    Delay(Duration),
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
