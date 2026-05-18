use std::time::Duration;

use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::utils::{constrain_render_elements, ConstrainAlign, ConstrainScaleBehavior},
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent},
    },
    desktop::{space::space_render_elements, Window},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        winit::{dpi::LogicalSize, window::Window as WinitWindow},
    },
    utils::{Rectangle, Transform},
};

use crate::AutoWC;

pub fn init_winit(
    event_loop: &mut EventLoop<AutoWC>,
    state: &mut AutoWC,
) -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Make the initial host window size configurable. Defaulting to
    // the virtual output size keeps early manual testing predictable.
    let window_attributes = WinitWindow::default_attributes()
        .with_inner_size(LogicalSize::new(
            state.virtual_size.w as f64,
            state.virtual_size.h as f64,
        ))
        .with_title("AutoWC")
        .with_visible(true);
    let (mut backend, winit) = winit::init_from_attributes::<GlesRenderer>(window_attributes)?;
    state.set_host_size(backend.window_size());

    let mode = Mode {
        size: state.virtual_size.to_physical(1),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<AutoWC>(&state.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

    let mut damage_tracker =
        OutputDamageTracker::new(backend.window_size(), 1.0, Transform::Flipped180);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| {
            match event {
                WinitEvent::Resized { size, .. } => {
                    state.set_host_size(size);
                    damage_tracker = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let size = backend.window_size();
                    if size != state.host_size {
                        state.set_host_size(size);
                        damage_tracker = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
                    }
                    let damage = Rectangle::from_size(size);

                    {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        let render_elements = space_render_elements::<_, Window, _>(
                            renderer,
                            [&state.space],
                            &output,
                            1.0,
                        )
                        .unwrap();
                        let render_elements: Vec<_> = constrain_render_elements(
                            render_elements,
                            (0, 0),
                            Rectangle::from_size(size),
                            Rectangle::from_size(state.virtual_size.to_physical(1)),
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
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}
