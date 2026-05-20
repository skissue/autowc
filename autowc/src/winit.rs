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
    output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
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
            state.virtual_size.w as f64,
            state.virtual_size.h as f64,
        ))
        .with_title("AutoWC Bootstrap")
        .with_visible(false);
    let (mut backend, host_events, requester) =
        host_winit::init_from_attributes(window_attributes)?;
    state.set_host_window_requester(requester);
    init_probe_output(state);

    let mut render_windows = HashMap::<AutoWindowId, RenderWindow>::new();
    let mut host_windows = HashMap::<HostWindowId, AutoWindowId>::new();
    let mut next_output_id = 1u64;

    event_loop
        .handle()
        .insert_source(host_events, move |event, _, state| match event {
            HostEvent::WindowCreated {
                auto_window_id,
                window_id,
                window,
                size,
                scale_factor,
            } => {
                if let Err(err) = backend.add_window(window) {
                    eprintln!("AutoWC failed to initialize host window renderer: {err}");
                    return;
                }

                let host = HostGeometry::new(size, scale_factor);
                let virtual_size = if state.dynamic_resize {
                    host.virtual_size()
                } else {
                    state.virtual_size
                };
                let output = Output::new(
                    format!("winit-{next_output_id}"),
                    PhysicalProperties {
                        size: (0, 0).into(),
                        subpixel: Subpixel::Unknown,
                        make: "Smithay".into(),
                        model: "Winit".into(),
                    },
                );
                next_output_id += 1;
                let _global = output.create_global::<AutoWC>(&state.display_handle);
                update_output_mode(
                    &output,
                    host.output_mode_size(state.dynamic_resize, virtual_size),
                    host.output_scale(state.dynamic_resize),
                );
                state.map_output_for_window(auto_window_id, &output);
                state.bind_host_window(
                    auto_window_id,
                    window_id,
                    output.clone(),
                    host.size,
                    virtual_size,
                );
                if auto_window_id == state.default_window_id {
                    state.set_host_size(host.size);
                }

                host_windows.insert(window_id, auto_window_id);
                render_windows.insert(
                    auto_window_id,
                    RenderWindow {
                        output,
                        virtual_framebuffer: None,
                        host_size: host.size,
                        host_scale_factor: host.scale_factor,
                        damage_tracker: OutputDamageTracker::new(
                            host.size,
                            1.0,
                            Transform::Flipped180,
                        ),
                    },
                );

                backend.window(window_id).request_redraw();
            }
            HostEvent::WindowCreateFailed {
                auto_window_id,
                error,
            } => {
                eprintln!("AutoWC failed to create host window {auto_window_id:?}: {error}");
            }
            HostEvent::WindowClosed { window_id } => {
                backend.remove_window(window_id);
                if let Some(auto_window_id) = host_windows.remove(&window_id) {
                    render_windows.remove(&auto_window_id);
                }
            }
            HostEvent::Resized {
                window_id,
                size,
                scale_factor,
                ..
            } => {
                let Some(auto_window_id) = host_windows.get(&window_id).copied() else {
                    return;
                };
                let Some(render_window) = render_windows.get_mut(&auto_window_id) else {
                    return;
                };
                handle_host_resize(state, auto_window_id, render_window, size, scale_factor);
            }
            HostEvent::Input { window_id, event } => {
                if let Some(auto_window_id) = host_windows.get(&window_id).copied() {
                    state.process_input_event(auto_window_id, event);
                }
            }
            HostEvent::Redraw { window_id } => {
                let Some(auto_window_id) = host_windows.get(&window_id).copied() else {
                    return;
                };
                let Some(render_window) = render_windows.get_mut(&auto_window_id) else {
                    return;
                };

                let size = backend.window_size(window_id);
                let scale_factor = backend.scale_factor(window_id);
                if size != render_window.host_size
                    || scale_factor != render_window.host_scale_factor
                {
                    handle_host_resize(state, auto_window_id, render_window, size, scale_factor);
                }

                render_host_window(
                    state,
                    &mut backend,
                    window_id,
                    auto_window_id,
                    render_window,
                    size,
                );
            }
            HostEvent::CloseRequested { window_id } => {
                state.close_host_window(window_id);
            }
            HostEvent::Focus { window_id, focused } => {
                let Some(auto_window_id) = host_windows.get(&window_id).copied() else {
                    return;
                };
                if focused {
                    state.focus_auto_window(auto_window_id);
                } else {
                    state.blur_auto_window(auto_window_id);
                }
            }
        })?;

    Ok(())
}

fn init_probe_output(state: &mut AutoWC) {
    let output = Output::new(
        "autowc-probe".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "AutoWC".into(),
            model: "Probe".into(),
        },
    );
    let _global = output.create_global::<AutoWC>(&state.display_handle);
    update_output_mode(&output, state.virtual_size.to_physical(1), 1.0);
}

struct RenderWindow {
    output: Output,
    virtual_framebuffer: Option<VirtualFramebuffer>,
    host_size: Size<i32, Physical>,
    host_scale_factor: f64,
    damage_tracker: OutputDamageTracker,
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

        if auto_window_id == state.default_window_id {
            write_pending_screenshots(
                state,
                renderer,
                virtual_framebuffer,
                auto_window_id,
                virtual_buffer_size,
            );
        }

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
    while let Some(request) = state.pending_screenshots.pop_front() {
        let result = if request.window_id == auto_window_id {
            renderer
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
                })
        } else {
            Err(format!("unknown AutoWC window {:?}", request.window_id))
        };

        match result {
            Ok(()) => state
                .protocol
                .send_screenshot(screenshot::display_path(&request.path)),
            Err(err) => state.protocol.send_error(err),
        }
    }
}

fn handle_host_resize(
    state: &mut AutoWC,
    auto_window_id: AutoWindowId,
    render_window: &mut RenderWindow,
    size: Size<i32, Physical>,
    scale_factor: f64,
) {
    let host = HostGeometry::new(size, scale_factor);
    render_window.host_size = host.size;
    render_window.host_scale_factor = host.scale_factor;
    if auto_window_id == state.default_window_id {
        state.set_host_size(host.size);
    }

    let virtual_size = if state.dynamic_resize {
        host.virtual_size()
    } else {
        state.virtual_size
    };
    state.resize_window_host(auto_window_id, host.size, virtual_size);
    update_output_mode(
        &render_window.output,
        host.output_mode_size(state.dynamic_resize, virtual_size),
        host.output_scale(state.dynamic_resize),
    );
    render_window.damage_tracker = OutputDamageTracker::new(host.size, 1.0, Transform::Flipped180);
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

    fn output_mode_size(
        self,
        dynamic_resize: bool,
        virtual_size: Size<i32, Logical>,
    ) -> Size<i32, Physical> {
        if dynamic_resize {
            self.size
        } else {
            virtual_size.to_physical(1)
        }
    }

    fn output_scale(self, dynamic_resize: bool) -> f64 {
        if dynamic_resize {
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

fn update_output_mode(output: &Output, size: Size<i32, Physical>, scale_factor: f64) {
    let scale_factor = normalized_scale_factor(scale_factor);
    let mode = Mode {
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
