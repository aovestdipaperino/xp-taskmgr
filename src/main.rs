//! A Windows XP style Task Manager for macOS, built on GPUI and gpui-component.

mod about;
mod apps;
mod graph;
mod mac;
mod stats;
mod table;
mod taskmgr;
mod tray;
mod xp;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::xp::color;

/// Restyles gpui-component's theme after Windows Classic.
fn apply_classic_theme(cx: &mut App) {
    Theme::change(ThemeMode::Light, None, cx);
    // `Theme::update` reconciles the edited colors into the tokens that the
    // components actually read; `global_mut` alone would leave them stale.
    Theme::update(cx, |theme| {
        theme.font_family = "Tahoma".into();
        theme.font_size = px(11.5);
        theme.radius = px(0.);
        theme.radius_lg = px(0.);
        theme.shadow = false;

        let white: Hsla = rgb(0xFFFFFF).into();
        theme.background = color::face();
        theme.foreground = color::text();
        theme.border = color::shadow();
        theme.popover = color::face();
        theme.popover_foreground = color::text();
        theme.accent = color::selection();
        theme.accent_foreground = white;
        theme.secondary_hover = color::face();
        theme.muted_foreground = color::gray_text();
        theme.input = color::shadow();
        theme.ring = color::text();
        theme.selection = color::selection();

        theme.table = white;
        theme.table_even = white;
        theme.table_head = color::face();
        theme.table_head_foreground = color::text();
        theme.table_hover = white;
        theme.table_active = color::selection();
        theme.table_active_border = color::selection();
        // Also colors the column resize handles, which XP does not draw.
        theme.table_row_border = gpui_kit::transparent_black();
        theme.list_active = color::selection();
        theme.list_active_border = color::selection();
        theme.list_hover = color::selection();
        theme.scrollbar = color::face();
        theme.scrollbar_thumb = color::shadow();
    });
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            apply_classic_theme(cx);

            let bounds = Bounds::centered(None, size(px(470.), px(520.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("macOS Task Manager".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                app_owns_titlebar_drag: true,
                window_min_size: Some(size(px(400.), px(440.))),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                mac::make_square(window, true);
                cx.new(|cx| taskmgr::TaskManager::new(window, cx))
            })
            .expect("failed to open the Task Manager window");
            cx.activate(true);
        });
}
