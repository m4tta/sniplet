use gpui_kit::{DisplayId, Window};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions, NSCursor, NSEvent,
    NSScreen, NSView, NSWindow, NSWindowAnimationBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSNumber, NSPoint, ns_string};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // GPUI owns this live AppKit view; these functions run on its UI thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

pub fn trace_capture_geometry(window: &Window) {
    if let Some(native) = native_window(window) {
        eprintln!(
            "capture: native frame={:?}; content frame={:?}; style={:?}",
            native.frame(),
            native.contentView().map(|view| view.frame()),
            native.styleMask()
        );
    }
}

pub fn trace_capture_cursor(window: &Window) {
    let current = NSCursor::currentCursor();
    let crosshair = NSCursor::crosshairCursor();
    eprintln!(
        "capture: key={}; pointer={:?}; crosshair={}",
        native_window(window).is_some_and(|native| native.isKeyWindow()),
        window.mouse_position(),
        current == crosshair,
    );
}

/// Show all capture panels before assigning keyboard focus to one display.
pub fn show_capture_overlay(window: &Window) {
    if let Some(native) = native_window(window) {
        native.orderFront(None);
    }
}

/// Cursor regions follow the key window. Keep the starting panel key during a drag.
pub fn focus_capture(window: &Window) {
    if let Some(native) = native_window(window) {
        native.makeKeyWindow();
        NSCursor::crosshairCursor().set();
    }
}

pub fn focus_capture_under_pointer(window: &Window) {
    let Some(native) = native_window(window) else {
        return;
    };
    let pointer = NSEvent::mouseLocation();
    let frame = native.frame();
    if pointer.x >= frame.origin.x
        && pointer.x < frame.origin.x + frame.size.width
        && pointer.y >= frame.origin.y
        && pointer.y < frame.origin.y + frame.size.height
        && !native.isKeyWindow()
    {
        focus_capture(window);
    }
}

define_class!(
    // This view supplies cursor rectangles while GPUI handles drawing and input.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    struct CaptureCursorView;

    impl CaptureCursorView {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::crosshairCursor());
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<&NSView> {
            None
        }
    }
);

/// Give every display a native cursor region, including inactive capture panels.
pub struct CaptureCursor(Retained<CaptureCursorView>);

impl CaptureCursor {
    pub fn new(window: &Window) -> Option<Self> {
        let native = native_window(window)?;
        let content = native.contentView()?;
        let main_thread = MainThreadMarker::new()?;
        // NSView's initializer is inherited with the same signature.
        let view: Retained<CaptureCursorView> = unsafe {
            msg_send![CaptureCursorView::alloc(main_thread), initWithFrame: content.bounds()]
        };
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        content.addSubview(&view);
        native.invalidateCursorRectsForView(&view);
        Some(Self(view))
    }
}

impl Drop for CaptureCursor {
    fn drop(&mut self) {
        self.0.removeFromSuperview();
        NSCursor::arrowCursor().set();
        if std::env::var_os("SNIPLET_CAPTURE_TRACE").is_some() {
            eprintln!(
                "capture: cursor restored on overlay release; arrow={}",
                NSCursor::currentCursor() == NSCursor::arrowCursor()
            );
        }
    }
}

/// Cover the complete display, including the menu bar, without AppKit's titlebar inset.
pub fn prepare_capture_overlay(window: &mut Window, display_id: Option<DisplayId>) {
    let Some(native) = native_window(window) else {
        return;
    };
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let screen = NSScreen::screens(main_thread)
        .iter()
        .find(|screen| {
            screen
                .deviceDescription()
                .objectForKey(ns_string!("NSScreenNumber"))
                .and_then(|number| number.downcast::<NSNumber>().ok())
                .is_some_and(|number| {
                    display_id.is_some_and(|id| u64::from(id) as u32 == number.unsignedIntValue())
                })
        })
        .or_else(|| native.screen());
    let Some(screen) = screen else { return };
    native.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    native.setHasShadow(false);
    native.setAnimationBehavior(NSWindowAnimationBehavior::None);
    native.setFrame_display(screen.frame(), false);
}

/// Keep the editor and its document alive while the app lives in the menu bar.
pub fn hide_editor(window: &Window) -> bool {
    let Some(native) = native_window(window) else {
        return false;
    };
    let Some(main_thread) = MainThreadMarker::new() else {
        return false;
    };
    native.orderOut(None);
    NSApplication::sharedApplication(main_thread)
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    true
}

/// Remove the editor before capture without a minimize animation or Dock changes.
pub fn hide_for_capture(window: &Window) -> bool {
    let Some(native) = native_window(window) else {
        return false;
    };
    let animation = native.animationBehavior();
    native.setAnimationBehavior(NSWindowAnimationBehavior::None);
    native.orderOut(None);
    native.setAnimationBehavior(animation);
    true
}

pub fn show_dock_icon(window: &Window) {
    if native_window(window).is_some()
        && let Some(main_thread) = MainThreadMarker::new()
    {
        NSApplication::sharedApplication(main_thread)
            .setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
}

pub fn launch_at_startup() -> bool {
    // SMAppService manages the current app bundle, not an external executable.
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
}

/// Return true only when macOS allows the registered app to launch at login.
pub fn set_launch_at_startup(enabled: bool) -> Result<bool, String> {
    // All calls use the main app's ServiceManagement registration on the UI thread.
    unsafe {
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
    }
}
