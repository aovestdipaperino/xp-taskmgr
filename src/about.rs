//! The About box, laid out like XP's "About Windows Task Manager".
//!
//! The original banner carried Microsoft's logos and wordmarks; this one keeps
//! the layout (blue gradient banner with an orange rule, icon, version block,
//! license line, memory footer) with this project's own text instead.

use std::sync::{Arc, LazyLock};

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::mac;
use crate::taskmgr::thousands;
use crate::update;
use crate::xp::{self, Bevel, color};

const LICENSE_URL: &str = "https://opensource.org/license/mit";

/// Facts shown in the box, gathered once when it opens.
#[derive(Debug, Clone)]
struct Info {
    version: String,
    owner: String,
    computer: String,
    memory_kb: u64,
}

fn command_output(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

impl Info {
    fn gather(memory_kb: u64) -> Self {
        let os = command_output("/usr/bin/sw_vers", &["-productVersion"]);
        let build = command_output("/usr/bin/sw_vers", &["-buildVersion"]);
        Self {
            version: format!(
                "Version {} (macOS {os} Build {build})",
                env!("CARGO_PKG_VERSION")
            ),
            owner: command_output("/usr/bin/id", &["-F"]),
            computer: command_output("/usr/sbin/scutil", &["--get", "ComputerName"]),
            memory_kb,
        }
    }
}

/// Where the update check has got to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Installing,
    Failed(String),
}

impl Status {
    fn message(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Checking => "Checking for updates...".into(),
            Self::UpToDate => "You have the latest version.".into(),
            Self::Available(v) => format!("Version {v} is available."),
            Self::Installing => "Downloading and installing...".into(),
            Self::Failed(e) => e.clone(),
        }
    }
}

/// The About window's root view.
#[derive(Debug)]
pub struct AboutBox {
    focus: FocusHandle,
    info: Info,
    status: Status,
}

/// Opens the About box centered on screen.
pub fn open(memory_kb: u64, cx: &mut App) -> Option<AnyWindowHandle> {
    let bounds = Bounds::centered(None, size(px(486.), px(372.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("About macOS Task Manager".into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        is_minimizable: false,
        ..WindowOptions::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        mac::make_square(window, false);
        cx.new(|cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            AboutBox {
                focus,
                info: Info::gather(memory_kb),
                status: Status::Idle,
            }
        })
    })
    .ok()
    .map(|(handle, _)| handle)
}

fn about_icon() -> impl IntoElement {
    static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/about-icon.png").to_vec(),
        ))
    });
    img(ICON.clone()).size(px(36.))
}

/// The blue banner: the slot where XP showed its logo now shows our name.
fn banner() -> impl IntoElement {
    let white = color::highlight();
    div()
        .relative()
        .h(px(78.))
        .flex_none()
        .bg(linear_gradient(
            90.,
            linear_color_stop(rgb(0x5A7EDC), 0.),
            linear_color_stop(rgb(0xA6C4F4), 1.),
        ))
        .child(
            div()
                .absolute()
                .top(px(10.))
                .left(px(150.))
                .flex()
                .flex_col()
                .font_family("Trebuchet MS")
                .text_color(white)
                .child(
                    div()
                        .text_size(px(34.))
                        .line_height(px(36.))
                        .font_weight(FontWeight::BOLD)
                        .child("Task Manager"),
                )
                .child(
                    div()
                        .pl(px(4.))
                        .text_size(px(17.))
                        .line_height(px(20.))
                        .text_color(hsla(0., 0., 1., 0.9))
                        .child("for macOS"),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(12.))
                .bottom(px(10.))
                .text_size(px(9.5))
                .line_height(px(11.))
                .text_color(hsla(0., 0., 1., 0.9))
                .child("Copyright © 2026")
                .child("Enzo Lombardi"),
        )
        .child(
            div()
                .absolute()
                .right(px(10.))
                .bottom(px(9.))
                .font_family("Trebuchet MS")
                .font_weight(FontWeight::BOLD)
                .italic()
                .text_size(px(11.))
                .text_color(white)
                .child("Built with GPUI"),
        )
        // The orange rule under the banner, fading into the blue.
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(4.))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(0x7FA0E8), 0.),
                    linear_color_stop(rgb(0xF0A050), 0.55),
                )),
        )
}

impl AboutBox {
    /// The update button: check first, then install what the check found.
    fn on_update_click(&mut self, cx: &mut Context<Self>) {
        match self.status.clone() {
            Status::Checking | Status::Installing => {}
            Status::Available(version) => {
                self.status = Status::Installing;
                cx.notify();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { update::install(&version) })
                        .await;
                    let _ = this.update(cx, |t, cx| match result {
                        Ok(()) => cx.quit(),
                        Err(e) => {
                            t.status = Status::Failed(e);
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            Status::Idle | Status::UpToDate | Status::Failed(_) => {
                self.status = Status::Checking;
                cx.notify();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { update::latest_version() })
                        .await;
                    let _ = this.update(cx, |t, cx| {
                        t.status = match result {
                            Ok(v) if update::is_newer(&v, env!("CARGO_PKG_VERSION")) => {
                                Status::Available(v)
                            }
                            Ok(_) => Status::UpToDate,
                            Err(e) => Status::Failed(e),
                        };
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }
}

impl Render for AboutBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = &self.info;
        let close = |window: &mut Window, _: &mut App| window.remove_window();
        let link = div()
            .id("license")
            .text_color(rgb(0x0000FF))
            .underline()
            .cursor_pointer()
            .on_click(|_, _, _| {
                let _ = std::process::Command::new("/usr/bin/open")
                    .arg(LICENSE_URL)
                    .spawn();
            })
            .child("MIT License");

        let open_url = |url: &'static str| {
            move |_: &ClickEvent, _: &mut Window, _: &mut App| {
                let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
            }
        };
        let github = div()
            .id("github")
            .text_color(rgb(0x0000FF))
            .underline()
            .cursor_pointer()
            .on_click(open_url(update::REPO_URL))
            .child(update::REPO_URL);

        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .line_height(px(16.))
            .child("XP Task Manager for macOS")
            .child(info.version.clone())
            .child("Copyright © 2026 Enzo Lombardi")
            .child(div().h(px(22.)))
            .child("This product is licensed under the terms of")
            .child(div().flex().child("the ").child(link).child(" to:"))
            .child(div().pl(px(16.)).child(info.owner.clone()))
            .child(div().pl(px(16.)).child(info.computer.clone()))
            .child(div().h(px(10.)))
            .child(div().flex().child("Project: ").child(github))
            .child(div().flex_1())
            .child(div().pb(px(6.)).child(self.status.message()))
            // Etched separator.
            .child(div().h(px(1.)).bg(color::shadow()))
            .child(div().h(px(1.)).bg(color::highlight()))
            .child(
                div()
                    .pt(px(8.))
                    .flex()
                    .gap(px(14.))
                    .child("Physical memory available to macOS:")
                    .child(format!("{} KB", thousands(info.memory_kb))),
            );

        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .pt(px(14.))
            .pr(px(14.))
            .child(
                div()
                    .w(px(110.))
                    .flex()
                    .justify_center()
                    .child(about_icon()),
            )
            .child(text);

        let footer = div()
            .flex()
            .flex_none()
            .justify_end()
            .gap(px(8.))
            .px(px(12.))
            .py(px(10.))
            .child(xp::button(
                "about-update",
                match &self.status {
                    Status::Checking => "Checking...".to_owned(),
                    Status::Installing => "Installing...".to_owned(),
                    Status::Available(v) => format!("Update to {v}"),
                    _ => "Check for Updates".to_owned(),
                },
                !matches!(self.status, Status::Checking | Status::Installing),
                cx.listener(|this, _, _, cx| this.on_update_click(cx)),
            ))
            .child(xp::button("about-ok", "OK", true, move |_, window, cx| {
                close(window, cx);
            }));

        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .child(xp::title_bar(
                window,
                None,
                "About macOS Task Manager",
                vec![xp::caption_button("about-close", xp::close_glyph(), close)],
            ))
            .child(banner())
            .child(body)
            .child(footer);

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, _| {
                if matches!(e.keystroke.key.as_str(), "escape" | "enter") {
                    window.remove_window();
                }
            }))
            .size_full()
            .bg(color::face())
            .text_color(color::text())
            .font_family("Tahoma")
            .text_size(px(11.5))
            .child(xp::bevel(Bevel::Raised, content).size_full())
    }
}
