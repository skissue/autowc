mod compositor;
mod xdg_shell;

use crate::AutoWC;

//
// Wl Seat
//

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::{
    wl_data_source::WlDataSource, wl_surface::WlSurface,
};
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
    ServerDndGrabHandler,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
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

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        seat: Seat<Self>,
    ) {
        self.sync_clipboard_to_host(ty, source, seat);
    }
}

impl DataDeviceHandler for AutoWC {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for AutoWC {
    fn started(
        &mut self,
        source: Option<WlDataSource>,
        _icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        if let Some(source) = source {
            source.cancelled();
        }
    }
}
impl ServerDndGrabHandler for AutoWC {}

delegate_data_device!(AutoWC);

//
// Wl Output & Xdg Output
//

impl OutputHandler for AutoWC {}
delegate_output!(AutoWC);
