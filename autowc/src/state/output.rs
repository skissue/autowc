use smithay::{
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
    reexports::wayland_server::DisplayHandle,
    utils::{Logical, Physical, Rectangle, Size, Transform},
};

use crate::{state::AutoWC, window::AutoWindowId};

impl AutoWC {
    fn map_output_for_window(&mut self, window_id: AutoWindowId, output: &Output) {
        self.windows
            .get_mut(window_id)
            .expect("AutoWC window is missing")
            .space_mut()
            .map_output(output, (0, 0));
    }

    fn create_output(&mut self) -> Output {
        create_output(
            &self.display_handle,
            self.initial_virtual_size,
            &mut self.next_output_id,
        )
    }

    pub(super) fn create_output_for_window(&mut self, window_id: AutoWindowId) {
        let next_pending_output = self.create_output();
        let output = std::mem::replace(&mut self.pending_output, next_pending_output);
        self.windows
            .get_mut(window_id)
            .expect("AutoWC window is missing")
            .set_output(output.clone());
        self.map_output_for_window(window_id, &output);
    }

    pub fn output_geometry_for_window(
        &self,
        window_id: AutoWindowId,
        output: &Output,
    ) -> Option<Rectangle<i32, Logical>> {
        self.windows.get(window_id)?.space().output_geometry(output)
    }

    pub(super) fn update_window_output(
        &self,
        window_id: AutoWindowId,
        host_size: Size<i32, Physical>,
        virtual_size: Size<i32, Logical>,
        output_scale: f64,
    ) {
        let Some(output) = self.output_for_window(window_id) else {
            return;
        };
        let (mode_size, scale) = if self.dynamic_resize {
            (host_size, output_scale)
        } else {
            (virtual_size.to_physical(1), 1.0)
        };
        update_output_mode(&output, mode_size, scale);
    }

    pub fn output_for_window(&self, window_id: AutoWindowId) -> Option<Output> {
        self.windows
            .get(window_id)
            .and_then(|window| window.output().cloned())
    }

    pub fn window_virtual_size(&self, window_id: AutoWindowId) -> Size<i32, Logical> {
        self.windows
            .get(window_id)
            .and_then(|window| window.virtual_size())
            .unwrap_or(self.initial_virtual_size)
    }
}

pub(super) fn create_output(
    display_handle: &DisplayHandle,
    initial_virtual_size: Size<i32, Logical>,
    next_output_id: &mut u64,
) -> Output {
    let output = Output::new(
        format!("winit-{}", *next_output_id),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
        },
    );
    *next_output_id += 1;

    let _global = output.create_global::<AutoWC>(display_handle);
    update_output_mode(&output, initial_virtual_size.to_physical(1), 1.0);
    output
}

fn update_output_mode(output: &Output, size: Size<i32, Physical>, scale_factor: f64) {
    let scale_factor = normalized_scale_factor(scale_factor);
    let mode = OutputMode {
        size,
        refresh: 60_000,
    };
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        Some(OutputScale::Fractional(scale_factor)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
}

fn normalized_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}
