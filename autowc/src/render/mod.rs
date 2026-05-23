use std::{collections::HashMap, time::Duration};

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
    reexports::winit::window::WindowId as HostWindowId,
    utils::{Buffer, Logical, Physical, Rectangle, Size, Transform},
};
use tracing::{debug, error, info, trace, warn};

use crate::{host, protocol::ControlResponse, screenshot, window::AutoWindowId, AutoWC};

struct RenderWindow {
    output: Output,
    virtual_framebuffer: Option<VirtualFramebuffer>,
    host_size: Size<i32, Physical>,
    host_scale_factor: f64,
    host_fullscreen: bool,
    damage_tracker: OutputDamageTracker,
}

impl RenderWindow {
    fn new(
        output: Output,
        host_size: Size<i32, Physical>,
        host_scale_factor: f64,
        host_fullscreen: bool,
    ) -> Self {
        Self {
            output,
            virtual_framebuffer: None,
            host_size,
            host_scale_factor,
            host_fullscreen,
            damage_tracker: OutputDamageTracker::new(host_size, 1.0, Transform::Flipped180),
        }
    }

    fn resize_host(
        &mut self,
        state: &mut AutoWC,
        auto_window_id: AutoWindowId,
        size: Size<i32, Physical>,
        scale_factor: f64,
        fullscreen: bool,
    ) {
        let host = HostGeometry::new(size, scale_factor);
        debug!(
            ?auto_window_id,
            ?size,
            scale_factor,
            fullscreen,
            normalized_scale_factor = host.scale_factor,
            "resizing render host"
        );
        self.host_size = host.size;
        self.host_scale_factor = host.scale_factor;
        self.host_fullscreen = fullscreen;

        let virtual_size =
            state.virtual_size_for_host_resize(auto_window_id, host.size, host.scale_factor);
        let (_, output_scale) = state.output_mode_for_window(
            auto_window_id,
            host.size,
            virtual_size,
            host.scale_factor,
        );
        state.resize_window_host(
            auto_window_id,
            host.size,
            virtual_size,
            output_scale,
            fullscreen,
        );
        self.damage_tracker = OutputDamageTracker::new(host.size, 1.0, Transform::Flipped180);
    }
}

#[derive(Default)]
pub(crate) struct RenderWindows {
    by_host_window: HashMap<HostWindowId, AutoWindowId>,
    by_auto_window: HashMap<AutoWindowId, RenderWindow>,
}

impl RenderWindows {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_host_window(
        &mut self,
        state: &mut AutoWC,
        host_window_id: HostWindowId,
        auto_window_id: AutoWindowId,
        size: Size<i32, Physical>,
        scale_factor: f64,
        fullscreen: bool,
    ) {
        let host = HostGeometry::new(size, scale_factor);
        let virtual_size =
            state.virtual_size_for_host_resize(auto_window_id, host.size, host.scale_factor);
        let (_, output_scale) = state.output_mode_for_window(
            auto_window_id,
            host.size,
            virtual_size,
            host.scale_factor,
        );
        debug!(
            ?host_window_id,
            ?auto_window_id,
            ?size,
            scale_factor,
            fullscreen,
            ?virtual_size,
            "adding render host window"
        );
        state.bind_host_window(
            auto_window_id,
            host_window_id,
            host.size,
            virtual_size,
            output_scale,
            fullscreen,
        );
        let Some(output) = state.output_for_window(auto_window_id) else {
            warn!(?auto_window_id, "cannot add render window without output");
            return;
        };
        self.insert(
            host_window_id,
            auto_window_id,
            RenderWindow::new(output, host.size, host.scale_factor, fullscreen),
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

    pub(crate) fn remove_host_window(&mut self, host_window_id: HostWindowId) {
        if let Some(auto_window_id) = self.by_host_window.remove(&host_window_id) {
            debug!(
                ?host_window_id,
                ?auto_window_id,
                "removing render host window"
            );
            self.by_auto_window.remove(&auto_window_id);
        } else {
            warn!(?host_window_id, "cannot remove unknown render host window");
        }
    }

    pub(crate) fn auto_window_id(&self, host_window_id: HostWindowId) -> Option<AutoWindowId> {
        self.by_host_window.get(&host_window_id).copied()
    }

    pub(crate) fn resize_host_window(
        &mut self,
        state: &mut AutoWC,
        host_window_id: HostWindowId,
        size: Size<i32, Physical>,
        scale_factor: f64,
        fullscreen: bool,
    ) {
        let Some((auto_window_id, render_window)) = self.get_by_host_window_mut(host_window_id)
        else {
            warn!(?host_window_id, "cannot resize unknown render host window");
            return;
        };
        render_window.resize_host(state, auto_window_id, size, scale_factor, fullscreen);
    }

    pub(crate) fn sync_host_fullscreen(
        &mut self,
        state: &mut AutoWC,
        host_window_id: HostWindowId,
        fullscreen: bool,
    ) {
        let Some((auto_window_id, render_window)) = self.get_by_host_window_mut(host_window_id)
        else {
            warn!(
                ?host_window_id,
                "cannot sync fullscreen for unknown render host window"
            );
            return;
        };
        render_window.host_fullscreen = fullscreen;
        state.set_window_host_fullscreen(auto_window_id, fullscreen);
    }

    pub(crate) fn redraw_host_window(
        &mut self,
        state: &mut AutoWC,
        backend: &mut host::HostGraphicsBackend,
        host_window_id: HostWindowId,
    ) {
        let size = backend.window_size(host_window_id);
        let scale_factor = backend.scale_factor(host_window_id);
        let Some((auto_window_id, render_window)) = self.get_by_host_window_mut(host_window_id)
        else {
            warn!(?host_window_id, "cannot redraw unknown render host window");
            return;
        };

        if size != render_window.host_size || scale_factor != render_window.host_scale_factor {
            let fullscreen = backend.fullscreen(host_window_id);
            render_window.resize_host(state, auto_window_id, size, scale_factor, fullscreen);
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
    backend: &mut host::HostGraphicsBackend,
    host_window_id: HostWindowId,
    auto_window_id: AutoWindowId,
    render_window: &mut RenderWindow,
    size: Size<i32, Physical>,
) {
    trace!(
        ?host_window_id,
        ?auto_window_id,
        ?size,
        "rendering host window"
    );
    let damage = Rectangle::from_size(size);

    {
        let (renderer, mut framebuffer) = backend.bind(host_window_id).unwrap();
        let virtual_scale = state
            .window_resize_policy(auto_window_id)
            .virtual_framebuffer_scale(render_window.host_scale_factor);
        let virtual_size = state.window_virtual_size(auto_window_id);
        let virtual_buffer_size = buffer_size(virtual_size, virtual_scale);

        let recreate_virtual_framebuffer = render_window
            .virtual_framebuffer
            .as_ref()
            .map(|framebuffer| framebuffer.texture.size() != virtual_buffer_size)
            .unwrap_or(true);
        if recreate_virtual_framebuffer {
            debug!(
                ?auto_window_id,
                ?virtual_buffer_size,
                "creating virtual framebuffer"
            );
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
            warn!(?auto_window_id, "cannot render window without space");
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
            state.final_pass_logical_size(auto_window_id, render_window.host_size, virtual_size);
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

        debug!(?auto_window_id, path = %request.path.display(), "writing screenshot");
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
            Ok(()) => {
                info!(?auto_window_id, path = %request.path.display(), "screenshot written");
                state.complete_control_response(
                    request.response,
                    ControlResponse::Screenshot {
                        path: screenshot::display_path(&request.path),
                    },
                );
            }
            Err(err) => {
                error!(?auto_window_id, path = %request.path.display(), error = %err, "failed to write screenshot");
                state.complete_control_response(request.response, ControlResponse::Error(err));
            }
        }
        state.flush_control_responses();
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
}

fn buffer_size(size: Size<i32, Logical>, scale_factor: f64) -> Size<i32, Buffer> {
    size.to_f64()
        .to_buffer(normalized_scale_factor(scale_factor), Transform::Normal)
        .to_i32_round()
}

fn buffer_size_as_physical(size: Size<i32, Buffer>) -> Size<i32, Physical> {
    Size::from((size.w, size.h))
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
