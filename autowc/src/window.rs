use smithay::{desktop::Window, reexports::wayland_server::protocol::wl_surface::WlSurface};

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

    pub fn is_empty(&self) -> bool {
        self.windows.iter().all(AutoWindow::is_empty)
    }
}

#[derive(Debug)]
pub struct AutoWindow {
    id: AutoWindowId,
    primary_window: Option<Window>,
    overlay_windows: Vec<Window>,
}

impl AutoWindow {
    fn new(id: AutoWindowId) -> Self {
        Self {
            id,
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

    pub fn has_primary_window(&self) -> bool {
        self.primary_window.is_some()
    }

    pub fn set_primary_window(&mut self, window: Window) {
        self.primary_window = Some(window);
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
