//! `AppKit` window tweaks GPUI does not expose.

use gpui_kit::Window;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSView, NSWindow, NSWindowButton,
    NSWindowStyleMask,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the handle's ns_view is a live NSView owned by this window.
    let view: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

/// Hides the traffic lights; the app draws XP caption buttons instead.
pub fn hide_traffic_lights(window: &Window) {
    let Some(ns) = ns_window(window) else { return };
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
}

/// Floats the window above normal windows, like "Always On Top".
pub fn set_always_on_top(window: &Window, on_top: bool) {
    const NORMAL: isize = 0;
    const FLOATING: isize = 3;
    if let Some(ns) = ns_window(window) {
        ns.setLevel(if on_top { FLOATING } else { NORMAL });
    }
}

/// Hides the window and the Dock icon, leaving only the menu bar meter.
///
/// This is XP's "Hide When Minimized": the taskbar button went away and the
/// tray icon was the only way back.
pub fn hide_to_tray(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(ns) = ns_window(window) {
        ns.orderOut(None);
    }
    NSApplication::sharedApplication(mtm)
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// Brings the window and the Dock icon back.
pub fn restore_from_tray(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    if let Some(ns) = ns_window(window) {
        if ns.isMiniaturized() {
            ns.deminiaturize(None);
        }
        ns.makeKeyAndOrderFront(None);
    }
    app.activate();
    // The traffic lights can reappear when AppKit re-lays out the title bar.
    hide_traffic_lights(window);
}

/// Makes the window borderless so it gets square XP corners.
///
/// macOS rounds the corners of every titled window, so the title bar style
/// is dropped entirely; the app draws its own caption and moves the window
/// with `start_window_move`. GPUI's window class still accepts key status.
pub fn make_square(window: &Window, resizable: bool) {
    let Some(ns) = ns_window(window) else { return };
    let mut mask = NSWindowStyleMask::Borderless | NSWindowStyleMask::Miniaturizable;
    if resizable {
        mask |= NSWindowStyleMask::Resizable;
    }
    ns.setStyleMask(mask);
    ns.setHasShadow(true);
}

/// Changes the smallest content size the user can resize the window to.
pub fn set_min_size(window: &Window, (width, height): (f64, f64)) {
    if let Some(ns) = ns_window(window) {
        ns.setContentMinSize(objc2_foundation::NSSize::new(width, height));
    }
}
