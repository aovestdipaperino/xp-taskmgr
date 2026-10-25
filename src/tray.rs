//! The notification area CPU meter, as a macOS menu bar status item.
//!
//! XP's Task Manager put a tiny green bar meter in the tray showing current
//! CPU usage, with a "CPU Usage: N%" tooltip and a menu to restore the window.
//! Here the icon holds two meters side by side, CPU on the left and GPU on
//! the right.

use block2::RcBlock;
use futures::channel::mpsc::UnboundedSender;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Commands sent from the status item menu to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Restore,
    Exit,
}

/// Ivars of the Objective-C menu target.
#[derive(Debug)]
pub struct TargetIvars {
    events: UnboundedSender<TrayEvent>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and the class has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "XPTaskManagerTrayTarget"]
    #[ivars = TargetIvars]
    #[derive(Debug)]
    pub struct TrayTarget;

    impl TrayTarget {
        #[unsafe(method(restore:))]
        fn restore(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().events.unbounded_send(TrayEvent::Restore);
        }

        #[unsafe(method(exit:))]
        fn exit(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().events.unbounded_send(TrayEvent::Exit);
        }
    }
);

impl TrayTarget {
    fn new(events: UnboundedSender<TrayEvent>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { events });
        // SAFETY: NSObject's init is always safe to call on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// The status item and the objects that must outlive it.
#[derive(Debug)]
pub struct Tray {
    item: Retained<NSStatusItem>,
    _target: Retained<TrayTarget>,
    _menu: Retained<NSMenu>,
    last: Option<(u32, u32)>,
}

/// Width of one meter, the gap between meters, and the image height in points.
const METER_W: f64 = 10.0;
const GAP: f64 = 2.0;
const ICON_W: f64 = METER_W * 2.0 + GAP;
const ICON_H: f64 = 16.0;

impl Tray {
    /// Adds the meter to the menu bar; `None` off the main thread.
    pub fn new(events: UnboundedSender<TrayEvent>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let target = TrayTarget::new(events, mtm);

        let menu = NSMenu::new(mtm);
        let add = |title: &str, action| {
            // SAFETY: the selector names a method TrayTarget implements, and the
            // target is kept alive by `Tray` for as long as the menu exists.
            unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(action),
                    &NSString::from_str(""),
                );
                item.setTarget(Some(&target));
                menu.addItem(&item);
            }
        };
        add("Restore Task Manager", sel!(restore:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add("Exit Task Manager", sel!(exit:));

        // NSVariableStatusItemLength is -1.
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
        item.setMenu(Some(&menu));
        let mut tray = Self {
            item,
            _target: target,
            _menu: menu,
            last: None,
        };
        tray.set_usage(0.0, 0.0);
        Some(tray)
    }

    /// Redraws both meters for CPU and GPU usage in percent.
    pub fn set_usage(&mut self, cpu: f32, gpu: f32) {
        let pct = |v: f32| v.round().clamp(0.0, 100.0) as u32;
        let (cpu, gpu) = (pct(cpu), pct(gpu));
        if self.last == Some((cpu, gpu)) {
            return;
        }
        self.last = Some((cpu, gpu));
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let image = meter_image(cpu, gpu);
        if let Some(button) = self.item.button(mtm) {
            button.setImage(Some(&image));
            button.setToolTip(Some(&NSString::from_str(&format!(
                "CPU Usage: {cpu}%\nGPU Usage: {gpu}%"
            ))));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

/// Draws two XP tray meters side by side: CPU left, GPU right.
///
/// The drawing handler reruns for each backing scale, so the meters stay
/// crisp on Retina menu bars.
fn meter_image(cpu: u32, gpu: u32) -> Retained<NSImage> {
    let draw = RcBlock::new(move |_: NSRect| -> Bool {
        draw_meter(0.0, cpu);
        draw_meter(METER_W + GAP, gpu);
        Bool::YES
    });
    let image =
        NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(ICON_W, ICON_H), false, &draw);
    image.setTemplate(false);
    image
}

/// One meter: a black well with green bars filling from below, at x offset `x0`.
fn draw_meter(x0: f64, percent: u32) {
    let bars = 6u32;
    let lit = (percent * bars).div_ceil(100).max(u32::from(percent > 0));
    let rgb = |r: f64, g: f64, b: f64| NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
    let rect =
        |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x0 + x, y), NSSize::new(w, h));
    // Dark green frame around a black well, as in the XP tray icon.
    rgb(0.0, 0.31, 0.0).setFill();
    NSBezierPath::fillRect(rect(0.0, 1.0, METER_W, ICON_H - 2.0));
    rgb(0.0, 0.0, 0.0).setFill();
    NSBezierPath::fillRect(rect(1.0, 2.0, METER_W - 2.0, ICON_H - 4.0));
    // Six 1pt bars with 1pt gaps inside the 12pt tall well.
    for i in 0..bars {
        let y = 2.5 + f64::from(i) * 2.0;
        if i < lit {
            rgb(0.0, 1.0, 0.0).setFill();
        } else {
            rgb(0.0, 0.35, 0.0).setFill();
        }
        NSBezierPath::fillRect(rect(2.0, y, METER_W - 4.0, 1.0));
    }
}
