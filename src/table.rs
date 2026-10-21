//! A generic sortable list view backed by gpui-component's `DataTable`.

use std::cmp::Ordering;
use std::rc::Rc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::xp::{self, Bevel, color};

/// Right padding gpui-component reserves in each header cell for a sort
/// icon: the cell padding of the table's custom `Size`.
const SORT_ICON_GAP: f32 = 8.0;

/// A sortable cell value.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Text(String),
    Num(f64),
}

impl Key {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Key::Num(a), Key::Num(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Key::Text(a), Key::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
            (Key::Num(_), Key::Text(_)) => Ordering::Less,
            (Key::Text(_), Key::Num(_)) => Ordering::Greater,
        }
    }
}

/// One list row: a stable identity, the displayed text and sort keys.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: u64,
    pub cells: Vec<SharedString>,
    pub keys: Vec<Key>,
}

/// Column layout of a list.
#[derive(Debug, Clone)]
pub struct ColumnSpec {
    pub name: &'static str,
    pub width: f32,
    pub right: bool,
}

/// Builds a row's right-click menu from the row's id.
#[derive(Clone)]
pub struct RowMenu(Rc<dyn Fn(u64, PopupMenu, &mut Window, &mut App) -> PopupMenu>);

impl RowMenu {
    pub fn new(f: impl Fn(u64, PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static) -> Self {
        Self(Rc::new(f))
    }
}

impl std::fmt::Debug for RowMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RowMenu")
    }
}

/// The table delegate shared by every tab.
#[derive(Debug)]
pub struct ListView {
    columns: Vec<ColumnSpec>,
    rows: Vec<Row>,
    sort: Option<(usize, ColumnSort)>,
    /// The selected row's id, kept here so cells can draw selected text.
    selected: Option<u64>,
    row_menu: Option<RowMenu>,
}

impl ListView {
    /// Creates an empty list, optionally sorted by `sort`.
    pub fn new(columns: Vec<ColumnSpec>, sort: Option<(usize, ColumnSort)>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            sort,
            selected: None,
            row_menu: None,
        }
    }

    /// Gives the rows a right-click menu.
    #[must_use]
    pub fn with_row_menu(mut self, menu: RowMenu) -> Self {
        self.row_menu = Some(menu);
        self
    }

    /// Records the row at `ix` as selected; call on `TableEvent::SelectRow`.
    pub fn select_index(&mut self, ix: usize) {
        self.selected = self.rows.get(ix).map(|r| r.id);
    }

    fn apply_sort(&mut self) {
        let Some((col, dir)) = self.sort else { return };
        if dir == ColumnSort::Default {
            return;
        }
        self.rows.sort_by(|a, b| {
            let o = a.keys[col].cmp(&b.keys[col]);
            if dir == ColumnSort::Descending {
                o.reverse()
            } else {
                o
            }
        });
    }

    fn sort_by_column(&mut self, col_ix: usize) {
        self.sort = Some(match self.sort {
            Some((c, ColumnSort::Ascending)) if c == col_ix => (col_ix, ColumnSort::Descending),
            _ => (col_ix, ColumnSort::Ascending),
        });
        self.apply_sort();
    }

    fn sort_indicator(&self, col_ix: usize) -> Option<ColumnSort> {
        match self.sort {
            Some((c, ColumnSort::Ascending)) if c == col_ix => Some(ColumnSort::Ascending),
            Some((c, ColumnSort::Descending)) if c == col_ix => Some(ColumnSort::Descending),
            _ => None,
        }
    }
}

/// Replaces a list's rows, keeping the selected row selected.
pub fn set_rows(
    state: &mut TableState<ListView>,
    rows: Vec<Row>,
    cx: &mut Context<TableState<ListView>>,
) {
    let delegate = state.delegate_mut();
    delegate.rows = rows;
    delegate.apply_sort();
    reselect(state, cx);
    cx.notify();
}

/// Selects the row with `id`, or clears the selection when there is none.
pub fn select_id(
    state: &mut TableState<ListView>,
    id: u64,
    cx: &mut Context<TableState<ListView>>,
) {
    state.delegate_mut().selected = Some(id);
    reselect(state, cx);
    cx.notify();
}

/// Moves the table's selection to wherever the selected id now sits.
///
/// Also undoes a column selection: the table still selects columns on the
/// left/right arrow keys even when they are not selectable.
pub fn reselect(state: &mut TableState<ListView>, cx: &mut Context<TableState<ListView>>) {
    if state.selected_col().is_some() {
        state.clear_selection(cx);
    }
    let id = state.delegate().selected;
    let new_ix = id.and_then(|id| state.delegate().rows.iter().position(|r| r.id == id));
    match new_ix {
        Some(ix) if state.selected_row() != Some(ix) => state.set_selected_row(ix, cx),
        Some(_) => {}
        None => {
            state.delegate_mut().selected = None;
            if state.selected_row().is_some() {
                state.clear_selection(cx);
            }
        }
    }
}

/// The id of the selected row.
pub fn selected_id(state: &TableState<ListView>) -> Option<u64> {
    state.delegate().selected
}

impl TableDelegate for ListView {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        // Our header buttons draw Classic sort triangles instead of the
        // table's built-in modern sort controls.
        let spec = &self.columns[col_ix];
        let col = Column::new(spec.name, spec.name)
            .width(px(spec.width))
            .p_0();
        if spec.right { col.text_right() } else { col }
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let spec = &self.columns[col_ix];
        // Bound the text to the actual cell, independently of its intrinsic
        // width. A flex child alone can grow to fit the caption and never elide.
        let caption = div()
            .id(("th-caption", col_ix))
            .absolute()
            .inset(px(2.))
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(4.))
            .px(px(3.))
            .text_color(color::text())
            .overflow_hidden()
            .group_active("xp-header", |s| s.top(px(3.)).left(px(3.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(spec.right, gpui_kit::Styled::text_right)
                    .child(spec.name),
            )
            .when_some(self.sort_indicator(col_ix), |el, direction| {
                el.child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), window, _| {
                            // Classic list headers use a muted 7 x 4 pixel
                            // triangle, not a font-dependent Unicode glyph.
                            for row in 0..4 {
                                let inset = if direction == ColumnSort::Ascending {
                                    3 - row
                                } else {
                                    row
                                };
                                window.paint_quad(fill(
                                    Bounds::new(
                                        bounds.origin + point(px(inset as f32), px(row as f32)),
                                        size(px((7 - inset * 2) as f32), px(1.)),
                                    ),
                                    rgb(0xACA899),
                                ));
                            }
                        },
                    )
                    .w(px(7.))
                    .h(px(4.))
                    .flex_shrink_0(),
                )
            });
        // The group must be on the stateful click target so GPUI can track
        // its active state. Keep the whole cell clickable, including the icon.
        let button = div()
            .id(("th", col_ix))
            .group("xp-header")
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right(px(-SORT_ICON_GAP))
            .bg(color::face())
            .hover(|s| s.bg(rgb(0xE2DED6)))
            .active(|s| s.bg(color::face()))
            .child(
                div()
                    .id(("th-top-left", col_ix))
                    .absolute()
                    .inset_0()
                    .border_t_1()
                    .border_l_1()
                    .border_color(color::highlight())
                    .group_active("xp-header", |s| s.border_color(color::shadow())),
            )
            .child(
                div()
                    .id(("th-bottom-right", col_ix))
                    .absolute()
                    .inset_0()
                    .border_b_1()
                    .border_r_1()
                    .border_color(color::shadow())
                    .group_active("xp-header", |s| s.border_color(color::highlight())),
            )
            .child(caption)
            .on_click(cx.listener(move |state, _, _, cx| {
                cx.stop_propagation();
                state.delegate_mut().sort_by_column(col_ix);
                reselect(state, cx);
                cx.notify();
            }));
        div().relative().size_full().child(button)
    }

    fn render_header(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        // A blank raised button behind the whole header row: past the last
        // column XP's list header continues as one empty button.
        div().id("header").relative().child(
            xp::bevel(Bevel::Raised, div())
                .absolute()
                .inset_0()
                .bg(color::face()),
        )
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        match (&self.row_menu, self.rows.get(row_ix)) {
            (Some(build), Some(row)) => (build.0)(row.id, menu, window, cx),
            _ => menu,
        }
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let selected = self
            .rows
            .get(row_ix)
            .is_some_and(|r| Some(r.id) == self.selected);
        div()
            .id(("row", row_ix))
            .when(selected, |el| el.bg(color::selection()))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = self.rows.get(row_ix);
        let selected = row.is_some_and(|r| Some(r.id) == self.selected);
        let text = row
            .and_then(|r| r.cells.get(col_ix))
            .cloned()
            .unwrap_or_default();
        div()
            .w_full()
            .min_w_0()
            .px(px(4.))
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .text_color(if selected {
                color::highlight()
            } else {
                color::text()
            })
            .child(text)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.rows
            .get(row_ix)
            .and_then(|r| r.cells.get(col_ix))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Prefer Rust's unit-test attribute over the GPUI test macro.
    use core::prelude::v1::test;

    fn list(sort: Option<(usize, ColumnSort)>) -> ListView {
        let mut list = ListView::new(
            vec![
                ColumnSpec {
                    name: "Name",
                    width: 100.,
                    right: false,
                },
                ColumnSpec {
                    name: "CPU",
                    width: 50.,
                    right: true,
                },
            ],
            sort,
        );
        list.rows = vec![
            Row {
                id: 1,
                cells: vec![],
                keys: vec![Key::Text("beta".into()), Key::Num(2.)],
            },
            Row {
                id: 2,
                cells: vec![],
                keys: vec![Key::Text("Alpha".into()), Key::Num(10.)],
            },
        ];
        list
    }

    #[test]
    fn indicator_tracks_sorted_column_and_direction() {
        let mut list = list(Some((0, ColumnSort::Ascending)));
        list.apply_sort();
        assert_eq!(list.rows[0].id, 2);
        assert_eq!(list.sort_indicator(0), Some(ColumnSort::Ascending));
        assert_eq!(list.sort_indicator(1), None);

        list.sort_by_column(0);
        assert_eq!(list.rows[0].id, 1);
        assert_eq!(list.sort_indicator(0), Some(ColumnSort::Descending));

        list.sort_by_column(0);
        assert_eq!(list.rows[0].id, 2);
        assert_eq!(list.sort_indicator(0), Some(ColumnSort::Ascending));

        list.sort_by_column(1);
        assert_eq!(list.rows[0].id, 1);
        assert_eq!(list.sort_indicator(0), None);
        assert_eq!(list.sort_indicator(1), Some(ColumnSort::Ascending));

        list.sort_by_column(1);
        assert_eq!(list.rows[0].id, 2);
        assert_eq!(list.sort_indicator(1), Some(ColumnSort::Descending));
    }

    #[test]
    fn unsorted_columns_have_no_indicator() {
        for sort in [None, Some((0, ColumnSort::Default))] {
            let mut list = list(sort);
            assert_eq!(list.sort_indicator(0), None);
            assert_eq!(list.sort_indicator(1), None);
            list.sort_by_column(0);
            assert_eq!(list.rows[0].id, 2);
            assert_eq!(list.sort_indicator(0), Some(ColumnSort::Ascending));
        }
    }
}
