use std::time::Duration;

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
        winit::{dpi::LogicalSize, window::Window as WinitWindow},
    },
    utils::{Buffer, Logical, Physical, Rectangle, Size, Transform},
};

use crate::{
    host_winit::{self, HostEvent},
    screenshot, AutoWC,
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
        .with_title("AutoWC")
        .with_visible(true);
    let (mut backend, host_events) = host_winit::init_from_attributes(window_attributes)?;
    let initial_host = HostGeometry::new(backend.window_size(), backend.scale_factor());
    state.set_host_size(initial_host.size);
    if state.dynamic_resize {
        state.resize_virtual_output(initial_host.virtual_size());
    }

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
        },
    );
    let _global = output.create_global::<AutoWC>(&state.display_handle);
    update_output_mode(
        &output,
        initial_host.output_mode_size(state),
        initial_host.output_scale(state),
    );

    state.space.map_output(&output, (0, 0));

    let mut virtual_framebuffer: Option<VirtualFramebuffer> = None;
    let mut host_scale_factor = initial_host.scale_factor;
    let mut damage_tracker =
        OutputDamageTracker::new(backend.window_size(), 1.0, Transform::Flipped180);

    event_loop
        .handle()
        .insert_source(host_events, move |event, _, state| {
            match event {
                HostEvent::Resized {
                    window_id,
                    size,
                    scale_factor,
                    ..
                } => {
                    let _ = window_id;
                    handle_host_resize(
                        state,
                        &output,
                        &mut damage_tracker,
                        &mut host_scale_factor,
                        size,
                        scale_factor,
                    );
                }
                HostEvent::Input { window_id, event } => {
                    let _ = window_id;
                    state.process_input_event(event)
                }
                HostEvent::Redraw { window_id } => {
                    let _ = window_id;
                    let size = backend.window_size();
                    let scale_factor = backend.scale_factor();
                    if size != state.host_size || scale_factor != host_scale_factor {
                        handle_host_resize(
                            state,
                            &output,
                            &mut damage_tracker,
                            &mut host_scale_factor,
                            size,
                            scale_factor,
                        );
                    }
                    let damage = Rectangle::from_size(size);

                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let virtual_scale = if state.dynamic_resize {
                            host_scale_factor
                        } else {
                            1.0
                        };
                        let virtual_buffer_size = buffer_size(state.virtual_size, virtual_scale);

                        let recreate_virtual_framebuffer = virtual_framebuffer
                            .as_ref()
                            .map(|framebuffer| framebuffer.texture.size() != virtual_buffer_size)
                            .unwrap_or(true);
                        if recreate_virtual_framebuffer {
                            virtual_framebuffer = Some(VirtualFramebuffer {
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

                        let render_elements = space_render_elements::<_, Window, _>(
                            renderer,
                            [&state.space],
                            &output,
                            1.0,
                        )
                        .unwrap();

                        let virtual_framebuffer = virtual_framebuffer.as_mut().unwrap();
                        {
                            let mut virtual_target =
                                renderer.bind(&mut virtual_framebuffer.texture).unwrap();

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

                        while let Some(request) = state.pending_screenshots.pop_front() {
                            let result = renderer
                                .copy_texture(
                                    &virtual_framebuffer.texture,
                                    Rectangle::from_size(virtual_buffer_size),
                                    Fourcc::Abgr8888,
                                )
                                .and_then(|mapping| {
                                    renderer.map_texture(&mapping).map(|pixels| pixels.to_vec())
                                })
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

                        let virtual_texture = TextureBuffer::from_texture(
                            renderer,
                            virtual_framebuffer.texture.clone(),
                            1,
                            Transform::Normal,
                            Some(vec![Rectangle::from_size(virtual_buffer_size)]),
                        );
                        let presentation_size = final_pass_logical_size(state);
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

                        damage_tracker
                            .render_output(
                                renderer,
                                &mut framebuffer,
                                0,
                                &render_elements,
                                [0.0, 0.0, 0.0, 1.0],
                            )
                            .unwrap();
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = state.display_handle.flush_clients();

                    // Ask for redraw to schedule new frame.
                    backend.window().request_redraw();
                }
                HostEvent::CloseRequested { window_id } => {
                    let _ = window_id;
                    state.request_shutdown();
                }
                HostEvent::Focus { window_id, focused } => {
                    let _ = (window_id, focused);
                }
            };
        })?;

    Ok(())
}

struct VirtualFramebuffer {
    texture: GlesTexture,
    damage_tracker: OutputDamageTracker,
}

fn handle_host_resize(
    state: &mut AutoWC,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    host_scale_factor: &mut f64,
    size: Size<i32, Physical>,
    scale_factor: f64,
) {
    let host = HostGeometry::new(size, scale_factor);
    state.set_host_size(host.size);
    *host_scale_factor = host.scale_factor;
    if state.dynamic_resize {
        state.resize_virtual_output(host.virtual_size());
        update_output_mode(
            output,
            host.output_mode_size(state),
            host.output_scale(state),
        );
    }
    *damage_tracker = OutputDamageTracker::new(host.size, 1.0, Transform::Flipped180);
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

    fn output_mode_size(self, state: &AutoWC) -> Size<i32, Physical> {
        if state.dynamic_resize {
            self.size
        } else {
            state.virtual_size.to_physical(1)
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

fn final_pass_logical_size(state: &AutoWC) -> Size<i32, Logical> {
    // The final pass renders into the host framebuffer at scale 1, so dynamic
    // mode presents the already-scaled offscreen buffer at host pixel size.
    if state.dynamic_resize {
        state.host_size.to_logical(1)
    } else {
        state.virtual_size
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
