use std::{collections::HashMap, sync::Arc, time::Duration};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{
                texture::{TextureBuffer, TextureRenderElement},
                utils::{constrain_render_elements, ConstrainAlign, ConstrainScaleBehavior},
                Kind,
            },
            gles::GlesTexture,
            Bind as _, ExportMem as _, Offscreen as _, Texture as _,
        },
    },
    desktop::{space::space_render_elements, Window},
    output::Output,
    reexports::{
        calloop::EventLoop,
        winit::{
            dpi::LogicalSize,
            window::{Window as WinitWindow, WindowId as HostWindowId},
        },
    },
    utils::{Buffer, Logical, Physical, Rectangle, Size, Transform},
};

use crate::{
    host_winit::{self, HostEvent},
    screenshot,
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
        host_winit::init_from_attributes(window_attributes)?;
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
    backend: &mut host_winit::HostGraphicsBackend,
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
            let Some((auto_window_id, render_window)) =
                render_windows.get_by_host_window_mut(window_id)
            else {
                return;
            };
            render_window.resize_host(state, auto_window_id, size, scale_factor);
        }
        HostEvent::Input { window_id, event } => {
            if let Some(auto_window_id) = render_windows.auto_window_id(window_id) {
                state.process_input_event(auto_window_id, event);
            }
        }
        HostEvent::Redraw { window_id } => {
            handle_redraw(state, backend, render_windows, window_id);
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
    backend: &mut host_winit::HostGraphicsBackend,
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
        HostGeometry::new(size, scale_factor),
    );
    backend.window(host_window_id).request_redraw();
}

fn handle_redraw(
    state: &mut AutoWC,
    backend: &mut host_winit::HostGraphicsBackend,
    render_windows: &mut RenderWindows,
    host_window_id: HostWindowId,
) {
    let size = backend.window_size(host_window_id);
    let scale_factor = backend.scale_factor(host_window_id);
    let Some((auto_window_id, render_window)) =
        render_windows.get_by_host_window_mut(host_window_id)
    else {
        return;
    };

    if size != render_window.host_size || scale_factor != render_window.host_scale_factor {
        render_window.resize_host(state, auto_window_id, size, scale_factor);
    }

    render_host_window(
        state,
        backend,
        host_window_id,
        auto_window_id,
        render_window,
        size,
    );
}

struct RenderWindow {
    output: Output,
    virtual_framebuffer: Option<VirtualFramebuffer>,
    host_size: Size<i32, Physical>,
    host_scale_factor: f64,
    damage_tracker: OutputDamageTracker,
}

impl RenderWindow {
    fn new(output: Output, host_size: Size<i32, Physical>, host_scale_factor: f64) -> Self {
        Self {
            output,
            virtual_framebuffer: None,
            host_size,
            host_scale_factor,
            damage_tracker: OutputDamageTracker::new(host_size, 1.0, Transform::Flipped180),
        }
    }

    fn resize_host(
        &mut self,
        state: &mut AutoWC,
        auto_window_id: AutoWindowId,
        size: Size<i32, Physical>,
        scale_factor: f64,
    ) {
        let host = HostGeometry::new(size, scale_factor);
        self.host_size = host.size;
        self.host_scale_factor = host.scale_factor;

        let virtual_size = host.virtual_size_for(state);
        state.resize_window_host(
            auto_window_id,
            host.size,
            virtual_size,
            host.output_scale(state),
        );
        self.damage_tracker = OutputDamageTracker::new(host.size, 1.0, Transform::Flipped180);
    }
}

#[derive(Default)]
struct RenderWindows {
    by_host_window: HashMap<HostWindowId, AutoWindowId>,
    by_auto_window: HashMap<AutoWindowId, RenderWindow>,
}

impl RenderWindows {
    fn new() -> Self {
        Self::default()
    }

    fn add_host_window(
        &mut self,
        state: &mut AutoWC,
        host_window_id: HostWindowId,
        auto_window_id: AutoWindowId,
        host: HostGeometry,
    ) {
        let virtual_size = host.virtual_size_for(state);
        state.bind_host_window(
            auto_window_id,
            host_window_id,
            host.size,
            virtual_size,
            host.output_scale(state),
        );
        let Some(output) = state.output_for_window(auto_window_id) else {
            return;
        };
        self.insert(
            host_window_id,
            auto_window_id,
            RenderWindow::new(output, host.size, host.scale_factor),
        );
    }

    fn insert(
        &mut self,
        host_window_id: HostWindowId,
        auto_window_id: AutoWindowId,
        render_window: RenderWindow,
    ) {
        self.by_host_window.insert(host_window_id, auto_window_id);
        self.by_auto_window.insert(auto_window_id, render_window);
    }

    fn remove_host_window(&mut self, host_window_id: HostWindowId) {
        if let Some(auto_window_id) = self.by_host_window.remove(&host_window_id) {
            self.by_auto_window.remove(&auto_window_id);
        }
    }

    fn auto_window_id(&self, host_window_id: HostWindowId) -> Option<AutoWindowId> {
        self.by_host_window.get(&host_window_id).copied()
    }

    fn get_by_host_window_mut(
        &mut self,
        host_window_id: HostWindowId,
    ) -> Option<(AutoWindowId, &mut RenderWindow)> {
        let auto_window_id = self.auto_window_id(host_window_id)?;
        let render_window = self.by_auto_window.get_mut(&auto_window_id)?;
        Some((auto_window_id, render_window))
    }
}

struct VirtualFramebuffer {
    texture: GlesTexture,
    damage_tracker: OutputDamageTracker,
}

fn render_host_window(
    state: &mut AutoWC,
    backend: &mut host_winit::HostGraphicsBackend,
    host_window_id: HostWindowId,
    auto_window_id: AutoWindowId,
    render_window: &mut RenderWindow,
    size: Size<i32, Physical>,
) {
    let damage = Rectangle::from_size(size);

    {
        let (renderer, mut framebuffer) = backend.bind(host_window_id).unwrap();
        let virtual_scale = if state.dynamic_resize {
            render_window.host_scale_factor
        } else {
            1.0
        };
        let virtual_size = state.window_virtual_size(auto_window_id);
        let virtual_buffer_size = buffer_size(virtual_size, virtual_scale);

        let recreate_virtual_framebuffer = render_window
            .virtual_framebuffer
            .as_ref()
            .map(|framebuffer| framebuffer.texture.size() != virtual_buffer_size)
            .unwrap_or(true);
        if recreate_virtual_framebuffer {
            render_window.virtual_framebuffer = Some(VirtualFramebuffer {
                texture: renderer
                    .create_buffer(Fourcc::Abgr8888, virtual_buffer_size)
                    .unwrap(),
                damage_tracker: OutputDamageTracker::new(
                    buffer_size_as_physical(virtual_buffer_size),
                    virtual_scale,
                    Transform::Normal,
                ),
            });
        }

        let Some(space) = state.window_space(auto_window_id) else {
            return;
        };
        let render_elements =
            space_render_elements::<_, Window, _>(renderer, [space], &render_window.output, 1.0)
                .unwrap();

        let virtual_framebuffer = render_window.virtual_framebuffer.as_mut().unwrap();
        {
            let mut virtual_target = renderer.bind(&mut virtual_framebuffer.texture).unwrap();

            virtual_framebuffer
                .damage_tracker
                .render_output(
                    renderer,
                    &mut virtual_target,
                    0,
                    &render_elements,
                    [0.0, 0.0, 0.0, 1.0],
                )
                .unwrap();
        }

        write_pending_screenshots(
            state,
            renderer,
            virtual_framebuffer,
            auto_window_id,
            virtual_buffer_size,
        );

        let virtual_texture = TextureBuffer::from_texture(
            renderer,
            virtual_framebuffer.texture.clone(),
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size(virtual_buffer_size)]),
        );
        let presentation_size =
            final_pass_logical_size(state.dynamic_resize, render_window.host_size, virtual_size);
        let virtual_element = TextureRenderElement::from_texture_buffer(
            (0.0, 0.0),
            &virtual_texture,
            None,
            None,
            Some(presentation_size),
            Kind::Unspecified,
        );
        let render_elements: Vec<_> = constrain_render_elements(
            [virtual_element],
            (0, 0),
            Rectangle::from_size(size),
            Rectangle::from_size(presentation_size.to_physical(1)),
            ConstrainScaleBehavior::Fit,
            ConstrainAlign::CENTER,
            1.0,
        )
        .collect();

        render_window
            .damage_tracker
            .render_output(
                renderer,
                &mut framebuffer,
                0,
                &render_elements,
                [0.0, 0.0, 0.0, 1.0],
            )
            .unwrap();
    }
    backend.submit(host_window_id, Some(&[damage])).unwrap();

    if let Some(output) = state.output_for_window(auto_window_id) {
        for window in state.mapped_windows_for(auto_window_id) {
            window.send_frame(
                &output,
                state.start_time.elapsed(),
                Some(Duration::ZERO),
                |_, _| Some(output.clone()),
            );
        }
    }

    state.refresh_window_space(auto_window_id);
    state.popups.cleanup();
    let _ = state.display_handle.flush_clients();

    backend.window(host_window_id).request_redraw();
}

fn write_pending_screenshots(
    state: &mut AutoWC,
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    virtual_framebuffer: &VirtualFramebuffer,
    auto_window_id: AutoWindowId,
    virtual_buffer_size: Size<i32, Buffer>,
) {
    let mut pending = std::mem::take(&mut state.pending_screenshots);
    while let Some(request) = pending.pop_front() {
        if request.window_id != auto_window_id {
            state.pending_screenshots.push_back(request);
            continue;
        }

        let result = renderer
            .copy_texture(
                &virtual_framebuffer.texture,
                Rectangle::from_size(virtual_buffer_size),
                Fourcc::Abgr8888,
            )
            .and_then(|mapping| renderer.map_texture(&mapping).map(|pixels| pixels.to_vec()))
            .map_err(|err| err.to_string())
            .and_then(|pixels| {
                screenshot::write_png(
                    &request.path,
                    &pixels,
                    virtual_buffer_size.w as u32,
                    virtual_buffer_size.h as u32,
                )
            });

        match result {
            Ok(()) => state
                .protocol
                .send_screenshot(screenshot::display_path(&request.path)),
            Err(err) => state.protocol.send_error(err),
        }
    }
}

#[derive(Clone, Copy)]
struct HostGeometry {
    size: Size<i32, Physical>,
    scale_factor: f64,
}

impl HostGeometry {
    fn new(size: Size<i32, Physical>, scale_factor: f64) -> Self {
        Self {
            size,
            scale_factor: normalized_scale_factor(scale_factor),
        }
    }

    fn virtual_size(self) -> Size<i32, Logical> {
        self.size
            .to_f64()
            .to_logical(self.scale_factor)
            .to_i32_ceil()
    }

    fn virtual_size_for(self, state: &AutoWC) -> Size<i32, Logical> {
        if state.dynamic_resize {
            self.virtual_size()
        } else {
            state.initial_virtual_size
        }
    }

    fn output_scale(self, state: &AutoWC) -> f64 {
        if state.dynamic_resize {
            self.scale_factor
        } else {
            1.0
        }
    }
}

fn buffer_size(size: Size<i32, Logical>, scale_factor: f64) -> Size<i32, Buffer> {
    size.to_f64()
        .to_buffer(normalized_scale_factor(scale_factor), Transform::Normal)
        .to_i32_round()
}

fn buffer_size_as_physical(size: Size<i32, Buffer>) -> Size<i32, Physical> {
    Size::from((size.w, size.h))
}

fn final_pass_logical_size(
    dynamic_resize: bool,
    host_size: Size<i32, Physical>,
    virtual_size: Size<i32, Logical>,
) -> Size<i32, Logical> {
    if dynamic_resize {
        host_size.to_logical(1)
    } else {
        virtual_size
    }
}

fn normalized_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_size_from_host_accounts_for_fractional_scale() {
        let host = HostGeometry::new(Size::from((2400, 1350)), 1.25);

        assert_eq!(host.virtual_size(), Size::from((1920, 1080)));
    }

    #[test]
    fn buffer_size_scales_logical_size_to_physical_size() {
        assert_eq!(
            buffer_size(Size::from((1920, 1080)), 1.25),
            Size::from((2400, 1350))
        );
    }

    #[test]
    fn normalized_scale_factor_rejects_invalid_values() {
        assert_eq!(normalized_scale_factor(1.25), 1.25);
        assert_eq!(normalized_scale_factor(0.0), 1.0);
        assert_eq!(normalized_scale_factor(f64::NAN), 1.0);
    }
}
