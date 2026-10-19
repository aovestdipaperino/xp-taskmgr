//! The black and green history graphs and LED meters.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::xp::{self, color};

/// Grid cell size in pixels, as on XP.
const GRID: f32 = 12.0;
/// Horizontal distance between two samples.
const STEP: f32 = 3.0;

/// One polyline in a history graph, values in 0..=1, oldest first.
#[derive(Debug, Clone)]
pub struct Series {
    pub values: Vec<f32>,
    pub color: Hsla,
}

/// A scrolling history graph. `tick` counts samples taken so the grid
/// scrolls left together with the data.
pub fn history(series: Vec<Series>, tick: usize) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| paint_history(window, bounds, &series, tick),
    )
    .size_full()
}

/// One history graph per CPU, tiled like XP's "One Graph Per CPU".
///
/// The grid shape is picked at paint time from the real bounds so every
/// cell comes out as close to square as possible: two CPUs sit side by side
/// in a wide pane and stack in a tall one, many CPUs form rows.
pub fn history_grid(cores: Vec<Vec<f32>>, tick: usize) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let n = cores.len();
            if n == 0 {
                return;
            }
            // Gray gaps between the boxes, as in the multi-CPU layout.
            window.paint_quad(fill(bounds, color::face()));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let (cols, rows) = grid_shape(n, w, h);
            let cell_w = (w - GAP * (cols - 1) as f32) / cols as f32;
            let cell_h = (h - GAP * (rows - 1) as f32) / rows as f32;
            for (ix, values) in cores.iter().enumerate() {
                let (col, row) = (ix % cols, ix / cols);
                let cell = Bounds::new(
                    bounds.origin
                        + point(
                            px((col as f32 * (cell_w + GAP)).round()),
                            px((row as f32 * (cell_h + GAP)).round()),
                        ),
                    size(px(cell_w.floor()), px(cell_h.floor())),
                );
                // Each CPU gets its own sunken box around a black graph.
                xp::paint_edges(window, cell, 0., color::shadow(), color::highlight());
                xp::paint_edges(window, cell, 1., color::dark_shadow(), color::face());
                let inner = Bounds::new(
                    cell.origin + point(px(2.), px(2.)),
                    size(cell.size.width - px(4.), cell.size.height - px(4.)),
                );
                let series = [Series {
                    values: values.clone(),
                    color: color::graph_line(),
                }];
                paint_history(window, inner, &series, tick);
            }
        },
    )
    .size_full()
}

/// Gray gap between neighbouring per-CPU boxes.
const GAP: f32 = 4.0;

/// Columns and rows for `n` cells in a `w` x `h` area, cells nearest square.
fn grid_shape(n: usize, w: f32, h: f32) -> (usize, usize) {
    (1..=n)
        .map(|cols| (cols, n.div_ceil(cols)))
        .min_by(|a, b| {
            let badness = |(c, r): (usize, usize)| {
                let aspect = (w / c as f32) / (h / r as f32);
                // Penalize empty cells so 18 CPUs prefer 6x3 over 5x4.
                aspect.max(1.0 / aspect) * (1.0 + (c * r - n) as f32 / n as f32)
            };
            badness(*a).total_cmp(&badness(*b))
        })
        .unwrap_or((1, 1))
}

fn paint_history(window: &mut Window, bounds: Bounds<Pixels>, series: &[Series], tick: usize) {
    window.paint_quad(fill(bounds, color::graph_bg()));
    let origin = bounds.origin;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));

    let line = |window: &mut Window, b: Bounds<Pixels>| {
        window.paint_quad(fill(b, color::graph_grid()));
    };
    let mut y = h - 1.0;
    while y >= 0.0 {
        line(
            window,
            Bounds::new(
                origin + point(px(0.), px(y)),
                size(bounds.size.width, px(1.)),
            ),
        );
        y -= GRID;
    }
    let scroll = (tick as f32 * STEP) % GRID;
    let mut x = w - 1.0 - (GRID - scroll) % GRID;
    while x >= 0.0 {
        line(
            window,
            Bounds::new(
                origin + point(px(x), px(0.)),
                size(px(1.), bounds.size.height),
            ),
        );
        x -= GRID;
    }

    for s in series {
        let visible = ((w / STEP) as usize + 2).min(s.values.len());
        if visible < 2 {
            continue;
        }
        let start = s.values.len() - visible;
        let mut path = PathBuilder::stroke(px(1.));
        for (i, v) in s.values[start..].iter().enumerate() {
            let px_x = w - 1.0 - (visible - 1 - i) as f32 * STEP;
            let px_y = (h - 1.0) - v.clamp(0.0, 1.0) * (h - 2.0);
            let p = origin + point(px(px_x), px(px_y));
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, s.color);
        }
    }
}

/// The two column LED bar meter from the CPU Usage and PF Usage boxes.
pub fn meter(fraction: f32, label: impl Into<SharedString>) -> impl IntoElement {
    let label = label.into();
    div()
        .size_full()
        .bg(color::graph_bg())
        .flex()
        .flex_col()
        .items_center()
        .py(px(6.))
        .gap(px(4.))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    let rows = (h / 3.0).floor() as usize;
                    let lit = (fraction.clamp(0.0, 1.0) * rows as f32).round() as usize;
                    // At least one bar is always lit, like the original.
                    let lit = lit.max(1);
                    let col_w = (w - 2.0) / 2.0;
                    for row in 0..rows {
                        let y = h - 2.0 - row as f32 * 3.0;
                        let on = row < lit;
                        for col in 0..2 {
                            let x = col as f32 * (col_w + 2.0);
                            let b = Bounds::new(
                                bounds.origin + point(px(x), px(y)),
                                size(px(col_w), px(2.)),
                            );
                            if on {
                                window.paint_quad(fill(b, color::graph_line()));
                            } else {
                                // Unlit bars are drawn as a dotted dim line.
                                let mut dx = 0.0;
                                while dx < col_w {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            bounds.origin + point(px(x + dx), px(y)),
                                            size(px(1.), px(1.)),
                                        ),
                                        color::led_off(),
                                    ));
                                    dx += 2.0;
                                }
                            }
                        }
                    }
                },
            )
            .w(px(38.))
            .flex_1(),
        )
        .child(div().text_color(color::graph_line()).child(label))
}

#[cfg(test)]
mod tests {
    use super::grid_shape;

    #[test]
    fn picks_square_cells() {
        // Wide pane: two CPUs side by side; tall pane: stacked.
        assert_eq!(grid_shape(2, 600., 180.), (2, 1));
        assert_eq!(grid_shape(2, 400., 500.), (1, 2));
        assert_eq!(grid_shape(1, 600., 180.), (1, 1));
        let (c, r) = grid_shape(18, 600., 180.);
        assert!(c * r >= 18 && c >= r);
    }
}
