use std::{collections::HashMap, io::Error as IoError, path::PathBuf, sync::Arc, time::Duration};

use smithay::{
    backend::{
        egl::{
            context::{GlAttributes, PixelFormatRequirements},
            native, EGLContext, EGLDisplay, EGLSurface, Error as EglError,
        },
        input::{
            AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
            DeviceCapability, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, Keycode,
            PointerAxisEvent, PointerButtonEvent, PointerMotionAbsoluteEvent, UnusedEvent,
        },
        renderer::{gles::GlesRenderer, Bind as _, RendererSuper},
        SwapBuffersError,
    },
    reexports::{
        calloop::{
            self, generic::Generic, EventSource, Interest, Mode, PostAction, Readiness, Token,
        },
        winit::{
            application::ApplicationHandler,
            dpi::PhysicalPosition,
            event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent},
            event_loop::{ActiveEventLoop, EventLoop as WinitEventLoop},
            platform::{pump_events::EventLoopExtPumpEvents, scancode::PhysicalKeyExtScancode},
            raw_window_handle::{HasWindowHandle, RawWindowHandle},
            window::{Window as WinitWindow, WindowAttributes, WindowId},
        },
    },
    utils::{Clock, Monotonic, Physical, Rectangle, Size},
};

pub fn init_from_attributes(
    attributes: WindowAttributes,
) -> Result<(HostGraphicsBackend, HostEventLoop), Box<dyn std::error::Error>> {
    let event_loop = WinitEventLoop::builder().build()?;

    #[allow(deprecated)]
    let window = Arc::new(event_loop.create_window(attributes)?);
    let (display, context, egl_surface, is_x11) = create_egl_surface(&window)?;

    let renderer = unsafe { GlesRenderer::new(context)? };
    let damage_tracking = display.supports_damage();

    event_loop.set_control_flow(smithay::reexports::winit::event_loop::ControlFlow::Poll);
    let scale_factor = window.scale_factor();
    let startup_window_id = window.id();
    let event_loop = Generic::new(event_loop, Interest::READ, Mode::Level);

    let mut render_windows = HashMap::new();
    render_windows.insert(
        startup_window_id,
        HostRenderWindow {
            window: window.clone(),
            egl_surface,
            bind_size: None,
        },
    );

    let mut event_windows = HashMap::new();
    event_windows.insert(
        startup_window_id,
        HostEventWindow {
            window,
            is_x11,
            scale_factor,
        },
    );

    Ok((
        HostGraphicsBackend {
            renderer,
            _display: display,
            windows: render_windows,
            startup_window_id,
            damage_tracking,
        },
        HostEventLoop {
            inner: HostEventLoopInner {
                windows: event_windows,
                startup_window_id,
                clock: Clock::<Monotonic>::new(),
                key_counter: 0,
            },
            fake_token: None,
            pending_events: Vec::new(),
            event_loop,
        },
    ))
}

fn create_egl_surface(
    window: &Arc<WinitWindow>,
) -> Result<(EGLDisplay, EGLContext, EGLSurface, bool), Box<dyn std::error::Error>> {
    let display = unsafe { EGLDisplay::new(window.clone())? };

    let gl_attributes = GlAttributes {
        version: (3, 0),
        profile: None,
        debug: cfg!(debug_assertions),
        vsync: false,
    };
    let context =
        EGLContext::new_with_config(&display, gl_attributes, PixelFormatRequirements::_10_bit())
            .or_else(|_| {
                EGLContext::new_with_config(
                    &display,
                    gl_attributes,
                    PixelFormatRequirements::_8_bit(),
                )
            })?;

    let (egl_surface, is_x11) = match window.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Wayland(handle)) => {
            let size = window.inner_size();
            let surface = unsafe {
                wayland_egl::WlEglSurface::new_from_raw(
                    handle.surface.as_ptr() as *mut _,
                    size.width as i32,
                    size.height as i32,
                )
            }?;
            let egl_surface = unsafe {
                EGLSurface::new(
                    &display,
                    context.pixel_format().unwrap(),
                    context.config_id(),
                    surface,
                )
                .map_err(EglError::CreationFailed)?
            };
            (egl_surface, false)
        }
        Ok(RawWindowHandle::Xlib(handle)) => {
            let egl_surface = unsafe {
                EGLSurface::new(
                    &display,
                    context.pixel_format().unwrap(),
                    context.config_id(),
                    native::XlibWindow(handle.window),
                )
                .map_err(EglError::CreationFailed)?
            };
            (egl_surface, true)
        }
        _ => return Err("only Wayland and X11 host windows are supported".into()),
    };

    let _ = context.unbind();
    Ok((display, context, egl_surface, is_x11))
}

pub struct HostGraphicsBackend {
    renderer: GlesRenderer,
    _display: EGLDisplay,
    windows: HashMap<WindowId, HostRenderWindow>,
    startup_window_id: WindowId,
    damage_tracking: bool,
}

struct HostRenderWindow {
    window: Arc<WinitWindow>,
    egl_surface: EGLSurface,
    bind_size: Option<Size<i32, Physical>>,
}

impl HostGraphicsBackend {
    pub fn window_size(&self) -> Size<i32, Physical> {
        let (w, h): (i32, i32) = self.startup_window().inner_size().into();
        (w, h).into()
    }

    pub fn scale_factor(&self) -> f64 {
        self.startup_window().scale_factor()
    }

    pub fn window(&self) -> &WinitWindow {
        self.startup_window()
    }

    pub fn bind(
        &mut self,
    ) -> Result<
        (
            &mut GlesRenderer,
            <GlesRenderer as RendererSuper>::Framebuffer<'_>,
        ),
        SwapBuffersError,
    > {
        let window_id = self.startup_window_id;
        let Self {
            renderer, windows, ..
        } = self;
        let window = windows
            .get_mut(&window_id)
            .expect("startup host window is missing");
        let window_size = window.window_size();
        if Some(window_size) != window.bind_size {
            window
                .egl_surface
                .resize(window_size.w, window_size.h, 0, 0);
        }
        window.bind_size = Some(window_size);

        let framebuffer = renderer.bind(&mut window.egl_surface)?;

        Ok((renderer, framebuffer))
    }

    pub fn submit(
        &mut self,
        damage: Option<&[Rectangle<i32, Physical>]>,
    ) -> Result<(), SwapBuffersError> {
        let window = self
            .windows
            .get_mut(&self.startup_window_id)
            .expect("startup host window is missing");
        let mut damage = match damage {
            Some(damage) if self.damage_tracking && !damage.is_empty() => {
                let bind_size = window
                    .bind_size
                    .expect("submitting without ever binding the renderer");
                let damage = damage
                    .iter()
                    .map(|rect| {
                        Rectangle::new(
                            (rect.loc.x, bind_size.h - rect.loc.y - rect.size.h).into(),
                            rect.size,
                        )
                    })
                    .collect::<Vec<_>>();
                Some(damage)
            }
            _ => None,
        };

        window.window.pre_present_notify();
        window.egl_surface.swap_buffers(damage.as_deref_mut())?;
        Ok(())
    }

    fn startup_window(&self) -> &WinitWindow {
        &self
            .windows
            .get(&self.startup_window_id)
            .expect("startup host window is missing")
            .window
    }
}

impl HostRenderWindow {
    fn window_size(&self) -> Size<i32, Physical> {
        let (w, h): (i32, i32) = self.window.inner_size().into();
        (w, h).into()
    }
}

#[derive(Debug)]
pub struct HostEventLoop {
    inner: HostEventLoopInner,
    fake_token: Option<Token>,
    pending_events: Vec<HostEvent>,
    event_loop: Generic<WinitEventLoop<()>>,
}

impl HostEventLoop {
    fn dispatch_new_events<F>(
        &mut self,
        callback: F,
    ) -> smithay::reexports::winit::platform::pump_events::PumpStatus
    where
        F: FnMut(HostEvent),
    {
        let event_loop = unsafe { self.event_loop.get_mut() };

        event_loop.pump_app_events(
            Some(Duration::ZERO),
            &mut HostEventLoopApp {
                inner: &mut self.inner,
                callback,
            },
        )
    }
}

#[derive(Debug)]
struct HostEventLoopInner {
    windows: HashMap<WindowId, HostEventWindow>,
    startup_window_id: WindowId,
    clock: Clock<Monotonic>,
    key_counter: u32,
}

#[derive(Debug)]
struct HostEventWindow {
    window: Arc<WinitWindow>,
    is_x11: bool,
    scale_factor: f64,
}

struct HostEventLoopApp<'a, F: FnMut(HostEvent)> {
    inner: &'a mut HostEventLoopInner,
    callback: F,
}

impl<F: FnMut(HostEvent)> HostEventLoopApp<'_, F> {
    fn timestamp(&self) -> u64 {
        self.inner.clock.now().as_micros()
    }

    fn window(&self, window_id: WindowId) -> Option<&HostEventWindow> {
        self.inner.windows.get(&window_id)
    }

    fn window_mut(&mut self, window_id: WindowId) -> Option<&mut HostEventWindow> {
        self.inner.windows.get_mut(&window_id)
    }
}

impl<F: FnMut(HostEvent)> ApplicationHandler for HostEventLoopApp<'_, F> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        (self.callback)(HostEvent::Input {
            window_id: self.inner.startup_window_id,
            event: InputEvent::DeviceAdded {
                device: HostVirtualDevice,
            },
        });
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                let Some(window) = self.window(window_id) else {
                    return;
                };
                let scale_factor = window.scale_factor;
                let (w, h): (i32, i32) = size.into();
                (self.callback)(HostEvent::Resized {
                    window_id,
                    size: (w, h).into(),
                    scale_factor,
                });
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: new_scale_factor,
                ..
            } => {
                let Some(window) = self.window_mut(window_id) else {
                    return;
                };
                window.scale_factor = new_scale_factor;
                let scale_factor = window.scale_factor;
                let (w, h): (i32, i32) = window.window.inner_size().into();
                (self.callback)(HostEvent::Resized {
                    window_id,
                    size: (w, h).into(),
                    scale_factor,
                });
            }
            WindowEvent::RedrawRequested => {
                (self.callback)(HostEvent::Redraw { window_id });
            }
            WindowEvent::CloseRequested => {
                (self.callback)(HostEvent::CloseRequested { window_id });
            }
            WindowEvent::Focused(focused) => {
                (self.callback)(HostEvent::Focus { window_id, focused });
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } if !is_synthetic && !event.repeat => {
                match event.state {
                    ElementState::Pressed => self.inner.key_counter += 1,
                    ElementState::Released => {
                        self.inner.key_counter = self.inner.key_counter.saturating_sub(1);
                    }
                };

                let event = InputEvent::Keyboard {
                    event: HostKeyboardInputEvent {
                        time: self.timestamp(),
                        key: event.physical_key.to_scancode().unwrap_or(0),
                        count: self.inner.key_counter,
                        state: key_state(event.state),
                    },
                };
                (self.callback)(HostEvent::Input { window_id, event });
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(window) = self.window(window_id) else {
                    return;
                };
                let size = window.window.inner_size();
                let x = position.x / size.width as f64;
                let y = position.y / size.height as f64;
                let event = InputEvent::PointerMotionAbsolute {
                    event: HostMouseMovedEvent {
                        time: self.timestamp(),
                        position: RelativePosition::new(x, y),
                        global_position: position,
                    },
                };
                (self.callback)(HostEvent::Input { window_id, event });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let event = InputEvent::PointerAxis {
                    event: HostMouseWheelEvent {
                        time: self.timestamp(),
                        delta,
                    },
                };
                (self.callback)(HostEvent::Input { window_id, event });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(window) = self.window(window_id) else {
                    return;
                };
                let event = InputEvent::PointerButton {
                    event: HostMouseInputEvent {
                        time: self.timestamp(),
                        button,
                        state: button_state(state),
                        is_x11: window.is_x11,
                    },
                };
                (self.callback)(HostEvent::Input { window_id, event });
            }
            WindowEvent::DroppedFile(_)
            | WindowEvent::Destroyed
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Ime(_)
            | WindowEvent::Moved(_)
            | WindowEvent::Occluded(_)
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::ThemeChanged(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::ActivationTokenDone { .. }
            | WindowEvent::Touch(_) => (),
        }
    }
}

impl EventSource for HostEventLoop {
    type Event = HostEvent;
    type Metadata = ();
    type Ret = ();
    type Error = IoError;

    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;

    fn before_sleep(&mut self) -> calloop::Result<Option<(Readiness, Token)>> {
        let mut pending_events = std::mem::take(&mut self.pending_events);
        let callback = |event| {
            pending_events.push(event);
        };
        self.dispatch_new_events(callback);
        self.pending_events = pending_events;
        if self.pending_events.is_empty() {
            Ok(None)
        } else {
            Ok(Some((Readiness::EMPTY, self.fake_token.unwrap())))
        }
    }

    fn process_events<F>(
        &mut self,
        _readiness: Readiness,
        _token: Token,
        mut callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut callback = |event| callback(event, &mut ());
        for event in self.pending_events.drain(..) {
            callback(event);
        }
        Ok(match self.dispatch_new_events(callback) {
            smithay::reexports::winit::platform::pump_events::PumpStatus::Continue => {
                PostAction::Continue
            }
            smithay::reexports::winit::platform::pump_events::PumpStatus::Exit(_) => {
                PostAction::Remove
            }
        })
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.fake_token = Some(token_factory.token());
        self.event_loop.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.event_loop.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> calloop::Result<()> {
        self.event_loop.unregister(poll)
    }
}

#[derive(Debug)]
pub enum HostEvent {
    Resized {
        window_id: WindowId,
        size: Size<i32, Physical>,
        scale_factor: f64,
    },
    Focus {
        window_id: WindowId,
        focused: bool,
    },
    Input {
        window_id: WindowId,
        event: InputEvent<HostInput>,
    },
    CloseRequested {
        window_id: WindowId,
    },
    Redraw {
        window_id: WindowId,
    },
}

#[derive(Debug)]
pub struct HostInput;

impl InputBackend for HostInput {
    type Device = HostVirtualDevice;
    type KeyboardKeyEvent = HostKeyboardInputEvent;
    type PointerAxisEvent = HostMouseWheelEvent;
    type PointerButtonEvent = HostMouseInputEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = HostMouseMovedEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HostVirtualDevice;

impl Device for HostVirtualDevice {
    fn id(&self) -> String {
        String::from("host-winit")
    }

    fn name(&self) -> String {
        String::from("host winit virtual input")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(
            capability,
            DeviceCapability::Keyboard | DeviceCapability::Pointer
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostKeyboardInputEvent {
    time: u64,
    key: u32,
    count: u32,
    state: KeyState,
}

impl Event<HostInput> for HostKeyboardInputEvent {
    fn time(&self) -> u64 {
        self.time
    }

    fn device(&self) -> HostVirtualDevice {
        HostVirtualDevice
    }
}

impl KeyboardKeyEvent<HostInput> for HostKeyboardInputEvent {
    fn key_code(&self) -> Keycode {
        (self.key + 8).into()
    }

    fn state(&self) -> KeyState {
        self.state
    }

    fn count(&self) -> u32 {
        self.count
    }
}

#[derive(Debug, Clone)]
pub struct HostMouseMovedEvent {
    time: u64,
    position: RelativePosition,
    global_position: PhysicalPosition<f64>,
}

impl Event<HostInput> for HostMouseMovedEvent {
    fn time(&self) -> u64 {
        self.time
    }

    fn device(&self) -> HostVirtualDevice {
        HostVirtualDevice
    }
}

impl PointerMotionAbsoluteEvent<HostInput> for HostMouseMovedEvent {}
impl AbsolutePositionEvent<HostInput> for HostMouseMovedEvent {
    fn x(&self) -> f64 {
        self.global_position.x
    }

    fn y(&self) -> f64 {
        self.global_position.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        f64::max(self.position.x * width as f64, 0.0)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        f64::max(self.position.y * height as f64, 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostMouseWheelEvent {
    time: u64,
    delta: MouseScrollDelta,
}

impl Event<HostInput> for HostMouseWheelEvent {
    fn time(&self) -> u64 {
        self.time
    }

    fn device(&self) -> HostVirtualDevice {
        HostVirtualDevice
    }
}

impl PointerAxisEvent<HostInput> for HostMouseWheelEvent {
    fn source(&self) -> AxisSource {
        match self.delta {
            MouseScrollDelta::LineDelta(_, _) => AxisSource::Wheel,
            MouseScrollDelta::PixelDelta(_) => AxisSource::Continuous,
        }
    }

    fn amount(&self, axis: Axis) -> Option<f64> {
        match (axis, self.delta) {
            (Axis::Horizontal, MouseScrollDelta::PixelDelta(delta)) => Some(-delta.x),
            (Axis::Vertical, MouseScrollDelta::PixelDelta(delta)) => Some(-delta.y),
            (_, MouseScrollDelta::LineDelta(_, _)) => None,
        }
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        match (axis, self.delta) {
            (Axis::Horizontal, MouseScrollDelta::LineDelta(x, _)) => Some(-x as f64 * 120.0),
            (Axis::Vertical, MouseScrollDelta::LineDelta(_, y)) => Some(-y as f64 * 120.0),
            (_, MouseScrollDelta::PixelDelta(_)) => None,
        }
    }

    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostMouseInputEvent {
    time: u64,
    button: WinitMouseButton,
    state: ButtonState,
    is_x11: bool,
}

impl Event<HostInput> for HostMouseInputEvent {
    fn time(&self) -> u64 {
        self.time
    }

    fn device(&self) -> HostVirtualDevice {
        HostVirtualDevice
    }
}

impl PointerButtonEvent<HostInput> for HostMouseInputEvent {
    fn button_code(&self) -> u32 {
        match self.button {
            WinitMouseButton::Left => 0x110,
            WinitMouseButton::Right => 0x111,
            WinitMouseButton::Middle => 0x112,
            WinitMouseButton::Forward => 0x115,
            WinitMouseButton::Back => 0x116,
            WinitMouseButton::Other(button) => {
                if self.is_x11 {
                    xorg_mouse_to_libinput(button as u32)
                } else {
                    button as u32
                }
            }
        }
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RelativePosition {
    x: f64,
    y: f64,
}

impl RelativePosition {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

fn key_state(state: ElementState) -> KeyState {
    match state {
        ElementState::Pressed => KeyState::Pressed,
        ElementState::Released => KeyState::Released,
    }
}

fn button_state(state: ElementState) -> ButtonState {
    match state {
        ElementState::Pressed => ButtonState::Pressed,
        ElementState::Released => ButtonState::Released,
    }
}

fn xorg_mouse_to_libinput(xorg: u32) -> u32 {
    match xorg {
        0 => 0,
        1 => 0x110,
        2 => 0x112,
        3 => 0x111,
        _ => xorg - 8 + 0x113,
    }
}
