//! Windows Classic look: 3D bevels, push buttons, etched frames and tabs.

use gpui_kit::prelude::*;
use gpui_kit::*;

/// The Windows Classic palette.
pub mod color {
    use gpui_kit::{Hsla, rgb};

    pub fn face() -> Hsla {
        rgb(0xD4D0C8).into()
    }
    pub fn highlight() -> Hsla {
        rgb(0xFFFFFF).into()
    }
    pub fn shadow() -> Hsla {
        rgb(0x808080).into()
    }
    pub fn dark_shadow() -> Hsla {
        rgb(0x404040).into()
    }
    pub fn text() -> Hsla {
        rgb(0x000000).into()
    }
    pub fn gray_text() -> Hsla {
        rgb(0x808080).into()
    }
    pub fn selection() -> Hsla {
        rgb(0x0A246A).into()
    }
    pub fn title_start() -> Hsla {
        rgb(0x0A246A).into()
    }
    pub fn title_end() -> Hsla {
        rgb(0xA6CAF0).into()
    }
    pub fn graph_bg() -> Hsla {
        rgb(0x000000).into()
    }
    pub fn graph_grid() -> Hsla {
        rgb(0x008040).into()
    }
    pub fn graph_line() -> Hsla {
        rgb(0x00FF00).into()
    }
    pub fn graph_kernel() -> Hsla {
        rgb(0xFF0000).into()
    }
    pub fn graph_commit() -> Hsla {
        rgb(0xFFFF00).into()
    }
    pub fn led_off() -> Hsla {
        rgb(0x007800).into()
    }
}

/// The four edge colors of a two pixel classic bevel.
#[derive(Debug, Clone, Copy)]
pub enum Bevel {
    /// Buttons and tab pages.
    Raised,
    /// Pressed buttons.
    Pressed,
    /// Text fields, list views, graphs.
    Sunken,
    /// Status bar panes: a single pixel inset.
    Thin,
}

/// Paints one rectangle edge pair: top/left in `tl`, bottom/right in `br`.
pub fn paint_edges(window: &mut Window, b: Bounds<Pixels>, inset: f32, tl: Hsla, br: Hsla) {
    let (x, y) = (b.origin.x + px(inset), b.origin.y + px(inset));
    let (w, h) = (
        b.size.width - px(inset * 2.),
        b.size.height - px(inset * 2.),
    );
    if w <= px(1.) || h <= px(1.) {
        return;
    }
    let one = px(1.);
    window.paint_quad(fill(Bounds::new(point(x, y), size(w, one)), tl));
    window.paint_quad(fill(Bounds::new(point(x, y), size(one, h)), tl));
    window.paint_quad(fill(Bounds::new(point(x, y + h - one), size(w, one)), br));
    window.paint_quad(fill(Bounds::new(point(x + w - one, y), size(one, h)), br));
}

/// The edge colors of a bevel, outermost ring first.
fn bevel_rings(kind: Bevel) -> &'static [(fn() -> Hsla, fn() -> Hsla)] {
    match kind {
        Bevel::Raised => &[
            (color::highlight, color::dark_shadow),
            (color::face, color::shadow),
        ],
        Bevel::Pressed => &[
            (color::dark_shadow, color::highlight),
            (color::shadow, color::face),
        ],
        Bevel::Sunken => &[
            (color::shadow, color::highlight),
            (color::dark_shadow, color::face),
        ],
        Bevel::Thin => &[(color::shadow, color::highlight)],
    }
}

/// Wraps `content` in a classic bevel; style the returned outer div.
///
/// The edges are painted by a canvas overlay rather than nested bordered
/// divs: every extra flex level multiplies Taffy's layout work, and a page
/// full of bevels made layout grind to a halt.
pub fn bevel(kind: Bevel, content: impl IntoElement) -> Div {
    let rings = bevel_rings(kind);
    let pad = rings.len() as f32;
    div()
        .relative()
        .flex()
        .flex_col()
        .p(px(pad))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(content),
        )
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    for (i, (tl, br)) in rings.iter().enumerate() {
                        paint_edges(window, bounds, i as f32, tl(), br());
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
}

/// A classic push button.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let caption = div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .px_2()
        .text_color(if enabled {
            color::text()
        } else {
            color::gray_text()
        })
        .child(label);
    div()
        .id(id.into())
        .h(px(23.))
        .min_w(px(75.))
        .bg(color::face())
        .flex()
        .when(enabled, |el| el.on_click(on_click))
        .child(PressableBevel {
            caption: caption.into_any_element(),
        })
}

/// Renders raised or pressed depending on the hover/active state of the parent.
#[derive(IntoElement)]
struct PressableBevel {
    caption: AnyElement,
}

impl RenderOnce for PressableBevel {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // GPUI cannot restyle nested borders on :active, so draw the pressed
        // bevel as a second layer that only shows while the button is held.
        div()
            .relative()
            .flex()
            .flex_1()
            .group("xp-button")
            .child(bevel(Bevel::Raised, self.caption).flex_1())
            .child(
                div()
                    .id("xp-button-pressed")
                    .absolute()
                    .inset_0()
                    .invisible()
                    .group_active("xp-button", gpui_kit::Styled::visible)
                    .child(bevel(Bevel::Pressed, div()).size_full()),
            )
    }
}

/// An etched group box whose title sits on the top border.
pub fn frame(title: impl Into<SharedString>, content: impl IntoElement) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .pt(px(15.))
        .px(px(10.))
        .pb(px(8.))
        .child(
            canvas(
                |_, _, _| {},
                |bounds, (), window, _| {
                    // The etched line starts at the middle of the title text.
                    let b = Bounds::new(
                        bounds.origin + point(px(0.), px(7.)),
                        size(bounds.size.width, bounds.size.height - px(7.)),
                    );
                    paint_edges(window, b, 0., color::shadow(), color::highlight());
                    paint_edges(window, b, 1., color::highlight(), color::shadow());
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(content),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(7.))
                .px(px(2.))
                .bg(color::face())
                .line_height(px(14.))
                .child(title.into()),
        )
}

/// A classic check box with a label.
pub fn checkbox(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mark = div()
        .size_full()
        .bg(color::highlight())
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .when(checked, |el| el.child("✓"));
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(5.))
        .on_click(on_toggle)
        .child(bevel(Bevel::Sunken, mark).size(px(13.)))
        .child(label.into())
}

/// Height of the tab strip; the page below starts at this offset.
pub const TAB_STRIP_HEIGHT: f32 = 22.0;

/// The tab strip of a classic property sheet.
///
/// Render it after the page and absolutely positioned on top of it, so the
/// selected tab can paint over the page's top highlight and join the page.
pub fn tab_strip<V: 'static>(
    labels: &[&'static str],
    selected: usize,
    cx: &Context<V>,
    on_select: impl Fn(&mut V, usize, &mut Context<V>) + 'static + Clone,
) -> Div {
    let tabs = labels.iter().enumerate().map(|(ix, label)| {
        let active = ix == selected;
        let on_select = on_select.clone();
        div()
            .id(("tab", ix))
            .relative()
            .flex()
            .flex_col()
            // The selected tab is 2px taller and 2px wider on each side. It only
            // grows leftwards (painting over its left neighbor); on the right its
            // neighbor gives up 2px instead, since later siblings paint on top.
            .when(active, |el| el.h(px(22.)).when(ix > 0, |el| el.ml(px(-2.))))
            .when(!active, |el| el.h(px(18.)))
            .bg(color::face())
            .border_t_1()
            .border_l_1()
            .border_color(color::highlight())
            .rounded_tl(px(2.))
            .rounded_tr(px(2.))
            .on_click(cx.listener(move |this, _, _, cx| on_select(this, ix, cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .border_r_1()
                    .border_color(color::dark_shadow())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .h_full()
                            .items_center()
                            .pl(px(if ix == selected + 1 { 4. } else { 6. }))
                            .pr(px(if active { 8. } else { 6. }))
                            .when(active && ix == 0, |el| el.pl(px(6.)))
                            .border_r_1()
                            .border_color(color::shadow())
                            .child(*label),
                    ),
            )
            // Hide the page's top highlight under the active tab, and stop the
            // tab's outer dark edge one pixel short so no black dot is left
            // sitting on the page's top line.
            .when(active, |el| {
                el.child(
                    div()
                        .absolute()
                        .bottom(px(-1.))
                        .left(px(1.))
                        .right(px(2.))
                        .h(px(2.))
                        .bg(color::face()),
                )
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .right_0()
                        .size(px(1.))
                        .bg(color::face()),
                )
            })
    });
    div()
        .flex()
        .items_end()
        .pl(px(2.))
        .h(px(TAB_STRIP_HEIGHT))
        .children(tabs)
}

/// A caption button (minimize, maximize, close) for [`title_bar`].
pub fn caption_button(
    id: &'static str,
    glyph: impl IntoElement,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .w(px(16.))
        .h(px(14.))
        .flex()
        .bg(color::face())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(
            bevel(
                Bevel::Raised,
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(glyph),
            )
            .flex_1(),
        )
        .into_any_element()
}

/// The close button glyph.
pub fn close_glyph() -> impl IntoElement {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .line_height(px(10.))
        .child("✕")
}

/// The classic gradient caption; dragging it moves the window.
pub fn title_bar(
    window: &Window,
    icon: Option<AnyElement>,
    title: impl Into<SharedString>,
    buttons: Vec<AnyElement>,
) -> Div {
    let (start, end) = if window.is_window_active() {
        (color::title_start(), color::title_end())
    } else {
        (rgb(0x808080).into(), rgb(0xC0C0C0).into())
    };
    div()
        .h(px(20.))
        .flex()
        .flex_none()
        .items_center()
        .px(px(2.))
        .gap(px(3.))
        .bg(linear_gradient(
            90.,
            linear_color_stop(start, 0.),
            linear_color_stop(end, 1.),
        ))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .children(icon)
        .child(
            div()
                .flex_1()
                .pl(px(2.))
                .text_color(color::highlight())
                .font_weight(FontWeight::BOLD)
                .child(title.into()),
        )
        .children(buttons)
}
