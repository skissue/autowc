mod compositor;
mod xdg_shell;

use crate::AutoWC;

//
// Wl Seat
//

use smithay::input::dnd::{DndGrabHandler, GrabType, Source};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::{delegate_data_device, delegate_output, delegate_seat};

impl SeatHandler for AutoWC {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<AutoWC> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

delegate_seat!(AutoWC);

//
// Wl Data Device
//

impl SelectionHandler for AutoWC {
    type SelectionUserData = ();
}

impl DataDeviceHandler for AutoWC {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for AutoWC {}
impl WaylandDndGrabHandler for AutoWC {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        _seat: Seat<Self>,
        _serial: Serial,
        _type_: GrabType,
    ) {
        source.cancel();
    }
}

delegate_data_device!(AutoWC);

//
// Wl Output & Xdg Output
//

impl OutputHandler for AutoWC {}
delegate_output!(AutoWC);
