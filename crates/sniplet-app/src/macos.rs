//! AppKit pieces Rune does not cover: capture overlay focus, login launches, and the login
//! item.
use objc2::{MainThreadMarker, msg_send, rc::Retained};
use objc2_app_kit::{NSCursor, NSEvent, NSView, NSWindow};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use raw_window_handle::RawWindowHandle;
use rune::WindowHandle;

/// Run `f` with the live AppKit window behind `window`, on the UI thread.
fn with_native<R>(window: &WindowHandle, f: impl FnOnce(&NSWindow) -> R) -> Option<R> {
    MainThreadMarker::new()?;
    window
        .with_raw_handle(|handle| {
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return None;
            };
            // Rune owns this live view; the callback runs on its UI thread.
            let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
            let native: Retained<NSWindow> = view.window()?;
            Some(f(&native))
        })
        .ok()
        .flatten()
}

/// Cursor regions follow the key window. Keep the starting overlay key during a drag.
pub fn focus_capture(window: &WindowHandle) {
    with_native(window, |native| {
        native.makeKeyWindow();
        NSCursor::crosshairCursor().set();
    });
}

/// Make the overlay under the pointer the key window, so it shows the crosshair.
pub fn focus_capture_under_pointer(window: &WindowHandle) {
    let under_pointer = with_native(window, |native| {
        let pointer = NSEvent::mouseLocation();
        let frame = native.frame();
        pointer.x >= frame.origin.x
            && pointer.x < frame.origin.x + frame.size.width
            && pointer.y >= frame.origin.y
            && pointer.y < frame.origin.y + frame.size.height
            && !native.isKeyWindow()
    });
    if under_pointer == Some(true) {
        focus_capture(window);
    }
}

/// Whether macOS launched the app as a login item. Its launch Apple event is current while
/// the event loop starts.
pub fn launched_at_login() -> bool {
    let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else {
        return false;
    };
    // Apple Event Manager constants: kAEOpenApplication, keyAEPropData,
    // and keyAELaunchedAsLogInItem. These selectors use 32-bit FourCharCodes.
    unsafe {
        let event_id: u32 = msg_send![&event, eventID];
        let property: Option<Retained<NSAppleEventDescriptor>> =
            msg_send![&event, paramDescriptorForKeyword: u32::from_be_bytes(*b"prdt")];
        event_id == u32::from_be_bytes(*b"oapp")
            && property.is_some_and(|value| value.enumCodeValue() == u32::from_be_bytes(*b"lgit"))
    }
}

pub fn launch_at_startup() -> bool {
    // SMAppService manages the current app bundle, not an external executable.
    objc2::rc::autoreleasepool(|_| unsafe {
        SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled
    })
}

/// Return true only when macOS allows the registered app to launch at login.
pub fn set_launch_at_startup(enabled: bool) -> Result<bool, String> {
    // ServiceManagement may wait on its daemon. Run this on a worker.
    objc2::rc::autoreleasepool(|_| unsafe {
        let service = SMAppService::mainAppService();
        let status = service.status();
        if enabled {
            if status == SMAppServiceStatus::NotRegistered || status == SMAppServiceStatus::NotFound
            {
                service
                    .registerAndReturnError()
                    .map_err(|error| error.localizedDescription().to_string())?;
            }
            if service.status() == SMAppServiceStatus::RequiresApproval {
                SMAppService::openSystemSettingsLoginItems();
            }
        } else if status != SMAppServiceStatus::NotRegistered {
            service
                .unregisterAndReturnError()
                .map_err(|error| error.localizedDescription().to_string())?;
        }
        Ok(service.status() == SMAppServiceStatus::Enabled)
    })
}
