use smithay::{
    desktop::{Space, Window},
    output::Output,
    reexports::{
        wayland_server::protocol::wl_surface::WlSurface, winit::window::WindowId as HostWindowId,
    },
    utils::{Logical, Physical, Size},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AutoWindowId(u64);

#[derive(Debug, Default)]
pub struct WindowRegistry {
    next_id: u64,
    windows: Vec<AutoWindow>,
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_window(&mut self) -> AutoWindowId {
        self.next_id += 1;
        let id = AutoWindowId(self.next_id);
        self.windows.push(AutoWindow::new(id));
        id
    }

    pub fn get(&self, id: AutoWindowId) -> Option<&AutoWindow> {
        self.windows.iter().find(|window| window.id == id)
    }

    pub fn get_mut(&mut self, id: AutoWindowId) -> Option<&mut AutoWindow> {
        self.windows.iter_mut().find(|window| window.id == id)
    }

    pub fn find_id_by_surface(&self, surface: &WlSurface) -> Option<AutoWindowId> {
        self.windows
            .iter()
            .find(|window| window.contains_surface(surface))
            .map(AutoWindow::id)
    }

    pub fn find_id_by_host_window(&self, host_window_id: HostWindowId) -> Option<AutoWindowId> {
        self.windows
            .iter()
            .find(|window| window.host_window_id() == Some(host_window_id))
            .map(AutoWindow::id)
    }

    pub fn find_window_by_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.windows
            .iter()
            .find_map(|window| window.window_by_surface(surface))
    }

    pub fn is_empty(&self) -> bool {
        self.windows.iter().all(AutoWindow::is_empty)
    }

    pub fn mapped_windows(&self) -> Vec<Window> {
        self.windows
            .iter()
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
    host_size: Option<Size<i32, Physical>>,
    virtual_size: Option<Size<i32, Logical>>,
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
            host_size: None,
            virtual_size: None,
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

    pub fn virtual_size(&self) -> Option<Size<i32, Logical>> {
        self.virtual_size
    }

    pub fn host_size(&self) -> Option<Size<i32, Physical>> {
        self.host_size
    }

    pub fn state(&self) -> AutoWindowState {
        self.state
    }

    pub fn set_state(&mut self, state: AutoWindowState) {
        self.state = state;
    }

    pub fn has_primary_window(&self) -> bool {
        self.primary_window.is_some()
    }

    pub fn set_primary_window(&mut self, window: Window) {
        self.primary_window = Some(window);
    }

    pub fn set_host_window(
        &mut self,
        host_window_id: HostWindowId,
        output: Output,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
    ) {
        self.host_window_id = Some(host_window_id);
        self.output = Some(output);
        self.host_size = Some(host_size);
        self.virtual_size = Some(virtual_size);
    }

    pub fn set_host_size(&mut self, host_size: Size<i32, Physical>) {
        self.host_size = Some(host_size);
    }

    pub fn set_virtual_size(&mut self, virtual_size: Size<i32, Logical>) {
        self.virtual_size = Some(virtual_size);
    }

    pub fn take_primary_window(&mut self) -> Option<Window> {
        self.primary_window.take()
    }

    pub fn push_overlay_window(&mut self, window: Window) {
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
