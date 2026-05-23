use std::collections::HashMap;

use smithay::{
    desktop::{Space, Window},
    output::Output,
    reexports::{
        wayland_server::protocol::wl_surface::WlSurface, winit::window::WindowId as HostWindowId,
    },
    utils::{Logical, Physical, Size},
};
use tracing::{debug, trace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSizing {
    Dynamic,
    Fixed { size: Size<i32, Logical> },
}

impl WindowSizing {
    pub fn from_dynamic_resize(dynamic_resize: bool, initial_size: Size<i32, Logical>) -> Self {
        if dynamic_resize {
            Self::Dynamic
        } else {
            Self::Fixed { size: initial_size }
        }
    }

    pub fn is_fixed(self) -> bool {
        matches!(self, Self::Fixed { .. })
    }

    pub fn virtual_size_for_host(
        self,
        host_size: Size<i32, Physical>,
        host_scale_factor: f64,
    ) -> Size<i32, Logical> {
        match self {
            Self::Dynamic => host_size
                .to_f64()
                .to_logical(host_scale_factor)
                .to_i32_ceil(),
            Self::Fixed { size } => size,
        }
    }

    pub fn output_mode(
        self,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
        output_scale: f64,
    ) -> (Size<i32, Physical>, f64) {
        match self {
            Self::Dynamic => (host_size, output_scale),
            Self::Fixed { .. } => (virtual_size.to_physical(1), 1.0),
        }
    }

    pub fn virtual_framebuffer_scale(self, host_scale_factor: f64) -> f64 {
        match self {
            Self::Dynamic => host_scale_factor,
            Self::Fixed { .. } => 1.0,
        }
    }

    pub fn final_pass_logical_size(
        self,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
    ) -> Size<i32, Logical> {
        match self {
            Self::Dynamic => host_size.to_logical(1),
            Self::Fixed { .. } => virtual_size,
        }
    }

    pub fn set_fixed_size(&mut self, size: Size<i32, Logical>) {
        if self.is_fixed() {
            *self = Self::Fixed { size };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeometry {
    sizing: WindowSizing,
    host_size: Option<Size<i32, Physical>>,
    host_scale_factor: f64,
}

impl WindowGeometry {
    pub fn new(sizing: WindowSizing) -> Self {
        Self {
            sizing,
            host_size: None,
            host_scale_factor: 1.0,
        }
    }

    pub fn from_dynamic_resize(dynamic_resize: bool, initial_size: Size<i32, Logical>) -> Self {
        Self::new(WindowSizing::from_dynamic_resize(
            dynamic_resize,
            initial_size,
        ))
    }

    pub fn sizing(&self) -> WindowSizing {
        self.sizing
    }

    pub fn set_sizing(&mut self, sizing: WindowSizing) {
        self.sizing = sizing;
    }

    pub fn is_fixed(&self) -> bool {
        self.sizing.is_fixed()
    }

    pub fn host_size(&self) -> Option<Size<i32, Physical>> {
        self.host_size
    }

    pub fn host_scale_factor(&self) -> f64 {
        self.host_scale_factor
    }

    pub fn set_host(&mut self, size: Size<i32, Physical>, scale_factor: f64) {
        self.host_size = Some(size);
        self.host_scale_factor = scale_factor;
    }

    pub fn set_virtual_size(&mut self, size: Size<i32, Logical>) {
        self.sizing.set_fixed_size(size);
    }

    pub fn virtual_size(&self, fallback_size: Size<i32, Logical>) -> Size<i32, Logical> {
        match self.host_size {
            Some(host_size) => self.virtual_size_for_host(host_size, self.host_scale_factor),
            None => match self.sizing {
                WindowSizing::Dynamic => fallback_size,
                WindowSizing::Fixed { size } => size,
            },
        }
    }

    pub fn virtual_size_for_host(
        &self,
        host_size: Size<i32, Physical>,
        host_scale_factor: f64,
    ) -> Size<i32, Logical> {
        self.sizing
            .virtual_size_for_host(host_size, host_scale_factor)
    }

    pub fn output_mode(
        &self,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
        output_scale: f64,
    ) -> (Size<i32, Physical>, f64) {
        self.sizing
            .output_mode(host_size, virtual_size, output_scale)
    }

    pub fn virtual_framebuffer_scale(&self, host_scale_factor: f64) -> f64 {
        self.sizing.virtual_framebuffer_scale(host_scale_factor)
    }

    pub fn final_pass_logical_size(
        &self,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
    ) -> Size<i32, Logical> {
        self.sizing.final_pass_logical_size(host_size, virtual_size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AutoWindowId(u64);

impl AutoWindowId {
    pub fn from_raw(id: u64) -> Option<Self> {
        (id > 0).then_some(Self(id))
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Default)]
pub struct WindowRegistry {
    next_id: u64,
    windows: HashMap<AutoWindowId, AutoWindow>,
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_window(&mut self) -> AutoWindowId {
        self.next_id += 1;
        let id = AutoWindowId(self.next_id);
        self.windows.insert(id, AutoWindow::new(id));
        debug!(?id, "registered auto window");
        id
    }

    pub fn get(&self, id: AutoWindowId) -> Option<&AutoWindow> {
        self.windows.get(&id)
    }

    pub fn get_mut(&mut self, id: AutoWindowId) -> Option<&mut AutoWindow> {
        self.windows.get_mut(&id)
    }

    pub fn find_id_by_surface(&self, surface: &WlSurface) -> Option<AutoWindowId> {
        self.windows
            .values()
            .find(|window| window.contains_surface(surface))
            .map(AutoWindow::id)
    }

    pub fn find_id_by_host_window(&self, host_window_id: HostWindowId) -> Option<AutoWindowId> {
        self.windows
            .values()
            .find(|window| window.host_window_id() == Some(host_window_id))
            .map(AutoWindow::id)
    }

    pub fn find_window_by_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.windows
            .values()
            .find_map(|window| window.window_by_surface(surface))
    }

    pub fn is_empty(&self) -> bool {
        self.windows.values().all(AutoWindow::is_empty)
    }

    pub fn first_alive_id(&self) -> Option<AutoWindowId> {
        self.windows
            .values()
            .filter(|window| !window.is_empty())
            .map(AutoWindow::id)
            .min_by_key(|id| id.raw())
    }

    pub fn mapped_ids(&self) -> Vec<AutoWindowId> {
        self.windows
            .values()
            .filter(|window| window.state() == AutoWindowState::Mapped && !window.is_empty())
            .map(AutoWindow::id)
            .collect()
    }

    pub fn mapped_windows(&self) -> Vec<Window> {
        self.windows
            .values()
            .flat_map(AutoWindow::mapped_windows)
            .collect()
    }
}

#[derive(Debug)]
pub struct AutoWindow {
    id: AutoWindowId,
    space: Space<Window>,
    host_window_id: Option<HostWindowId>,
    output: Option<Output>,
    geometry: WindowGeometry,
    host_fullscreen: bool,
    state: AutoWindowState,
    primary_window: Option<Window>,
    overlay_windows: Vec<Window>,
}

impl AutoWindow {
    fn new(id: AutoWindowId) -> Self {
        Self {
            id,
            space: Space::default(),
            host_window_id: None,
            output: None,
            geometry: WindowGeometry::new(WindowSizing::Dynamic),
            host_fullscreen: false,
            state: AutoWindowState::Empty,
            primary_window: None,
            overlay_windows: Vec::new(),
        }
    }

    pub fn id(&self) -> AutoWindowId {
        self.id
    }

    pub fn primary_window(&self) -> Option<&Window> {
        self.primary_window.as_ref()
    }

    pub fn space(&self) -> &Space<Window> {
        &self.space
    }

    pub fn space_mut(&mut self) -> &mut Space<Window> {
        &mut self.space
    }

    pub fn host_window_id(&self) -> Option<HostWindowId> {
        self.host_window_id
    }

    pub fn output(&self) -> Option<&Output> {
        self.output.as_ref()
    }

    pub fn geometry(&self) -> &WindowGeometry {
        &self.geometry
    }

    pub fn set_geometry(&mut self, geometry: WindowGeometry) {
        trace!(id = ?self.id, ?geometry, "setting window geometry");
        self.geometry = geometry;
    }

    pub fn sizing(&self) -> WindowSizing {
        self.geometry.sizing()
    }

    pub fn set_sizing(&mut self, sizing: WindowSizing) {
        trace!(id = ?self.id, ?sizing, "setting window sizing");
        self.geometry.set_sizing(sizing);
    }

    pub fn is_fixed(&self) -> bool {
        self.geometry.is_fixed()
    }

    pub fn virtual_size(&self) -> Option<Size<i32, Logical>> {
        match (self.geometry.host_size(), self.sizing()) {
            (Some(_), _) | (None, WindowSizing::Fixed { .. }) => {
                Some(self.geometry.virtual_size(Size::from((0, 0))))
            }
            (None, WindowSizing::Dynamic) => None,
        }
    }

    pub fn host_size(&self) -> Option<Size<i32, Physical>> {
        self.geometry.host_size()
    }

    pub fn host_fullscreen(&self) -> bool {
        self.host_fullscreen
    }

    pub fn state(&self) -> AutoWindowState {
        self.state
    }

    pub fn set_state(&mut self, state: AutoWindowState) {
        debug!(id = ?self.id, from = ?self.state, to = ?state, "auto window state changed");
        self.state = state;
    }

    pub fn has_primary_window(&self) -> bool {
        self.primary_window.is_some()
    }

    pub fn set_primary_window(&mut self, window: Window) {
        trace!(id = ?self.id, "setting primary window");
        self.primary_window = Some(window);
    }

    pub fn set_output(&mut self, output: Output) {
        trace!(id = ?self.id, output = output.name(), "setting window output");
        self.output = Some(output);
    }

    pub fn set_host_window(
        &mut self,
        host_window_id: HostWindowId,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
        host_scale_factor: f64,
        host_fullscreen: bool,
    ) {
        trace!(
            id = ?self.id,
            ?host_window_id,
            ?host_size,
            ?virtual_size,
            host_scale_factor,
            host_fullscreen,
            "setting host window"
        );
        self.host_window_id = Some(host_window_id);
        self.geometry.set_host(host_size, host_scale_factor);
        self.geometry.set_virtual_size(virtual_size);
        self.host_fullscreen = host_fullscreen;
    }

    pub fn set_host_geometry(&mut self, host_size: Size<i32, Physical>, scale_factor: f64) {
        trace!(id = ?self.id, ?host_size, scale_factor, "setting host geometry");
        self.geometry.set_host(host_size, scale_factor);
    }

    pub fn set_virtual_size(&mut self, virtual_size: Size<i32, Logical>) {
        trace!(id = ?self.id, ?virtual_size, "setting virtual size");
        self.geometry.set_virtual_size(virtual_size);
    }

    pub fn set_host_fullscreen(&mut self, fullscreen: bool) -> bool {
        if self.host_fullscreen == fullscreen {
            return false;
        }
        debug!(
            id = ?self.id,
            from = self.host_fullscreen,
            to = fullscreen,
            "host fullscreen state changed"
        );
        self.host_fullscreen = fullscreen;
        true
    }

    pub fn take_primary_window(&mut self) -> Option<Window> {
        trace!(id = ?self.id, "taking primary window");
        self.primary_window.take()
    }

    pub fn push_overlay_window(&mut self, window: Window) {
        debug!(id = ?self.id, "adding overlay window");
        self.overlay_windows.push(window);
    }

    pub fn overlay_windows(&self) -> impl Iterator<Item = &Window> {
        self.overlay_windows.iter()
    }

    pub fn find_overlay_by_surface(&self, surface: &WlSurface) -> Option<&Window> {
        self.overlay_windows
            .iter()
            .find(|window| window.toplevel().unwrap().wl_surface() == surface)
    }

    pub fn remove_overlay_by_surface(&mut self, surface: &WlSurface) -> Option<Window> {
        let index = self
            .overlay_windows
            .iter()
            .position(|window| window.toplevel().unwrap().wl_surface() == surface)?;
        Some(self.overlay_windows.remove(index))
    }

    pub fn promote_last_overlay(&mut self) -> Option<Window> {
        let window = self.overlay_windows.pop()?;
        debug!(id = ?self.id, "promoting overlay window");
        self.primary_window = Some(window.clone());
        Some(window)
    }

    pub fn next_focus_window(&self) -> Option<Window> {
        self.overlay_windows
            .last()
            .cloned()
            .or_else(|| self.primary_window.clone())
    }

    pub fn mapped_windows(&self) -> Vec<Window> {
        self.primary_window
            .iter()
            .chain(self.overlay_windows.iter())
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.primary_window.is_none() && self.overlay_windows.is_empty()
    }

    fn contains_surface(&self, surface: &WlSurface) -> bool {
        self.window_by_surface(surface).is_some()
    }

    fn window_by_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.primary_window
            .as_ref()
            .filter(|window| window.toplevel().unwrap().wl_surface() == surface)
            .cloned()
            .or_else(|| {
                self.overlay_windows
                    .iter()
                    .find(|window| window.toplevel().unwrap().wl_surface() == surface)
                    .cloned()
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoWindowState {
    Empty,
    WaitingProbeCommit,
    WaitingHostWindow,
    WaitingFinalCommit,
    Mapped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_sizing_tracks_legacy_dynamic_resize_flag() {
        assert_eq!(
            WindowSizing::from_dynamic_resize(true, Size::from((800, 600))),
            WindowSizing::Dynamic
        );
        assert_eq!(
            WindowSizing::from_dynamic_resize(false, Size::from((800, 600))),
            WindowSizing::Fixed {
                size: Size::from((800, 600))
            }
        );
    }

    #[test]
    fn dynamic_window_geometry_uses_host_size() {
        let mut geometry = WindowGeometry::new(WindowSizing::Dynamic);
        geometry.set_host(Size::from((2400, 1350)), 1.25);

        assert!(!geometry.is_fixed());
        assert_eq!(
            geometry.virtual_size(Size::from((800, 600))),
            Size::from((1920, 1080))
        );
        assert_eq!(
            geometry.output_mode(Size::from((2400, 1350)), Size::from((1920, 1080)), 1.25),
            (Size::from((2400, 1350)), 1.25)
        );
        assert_eq!(geometry.virtual_framebuffer_scale(1.25), 1.25);
        assert_eq!(
            geometry.final_pass_logical_size(Size::from((2400, 1350)), Size::from((1920, 1080))),
            Size::from((2400, 1350))
        );
    }

    #[test]
    fn fixed_window_geometry_preserves_virtual_size() {
        let mut geometry = WindowGeometry::new(WindowSizing::Fixed {
            size: Size::from((800, 600)),
        });
        geometry.set_host(Size::from((2400, 1350)), 1.25);

        assert!(geometry.is_fixed());
        assert_eq!(
            geometry.virtual_size(Size::from((1280, 720))),
            Size::from((800, 600))
        );
        assert_eq!(
            geometry.output_mode(Size::from((2400, 1350)), Size::from((800, 600)), 1.25),
            (Size::from((800, 600)), 1.0)
        );
        assert_eq!(geometry.virtual_framebuffer_scale(1.25), 1.0);
        assert_eq!(
            geometry.final_pass_logical_size(Size::from((2400, 1350)), Size::from((800, 600))),
            Size::from((800, 600))
        );
    }

    #[test]
    fn registry_allocates_stable_distinct_window_ids() {
        let mut registry = WindowRegistry::new();

        let first = registry.create_window();
        let second = registry.create_window();

        assert_ne!(first, second);
        assert_eq!(registry.get(first).unwrap().id(), first);
        assert_eq!(registry.get(second).unwrap().id(), second);
    }

    #[test]
    fn registry_is_empty_when_all_windows_have_no_toplevels() {
        let mut registry = WindowRegistry::new();

        registry.create_window();
        registry.create_window();

        assert!(registry.is_empty());
    }
}
