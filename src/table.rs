//! A generic sortable list view backed by gpui-component's `DataTable`.

use std::cmp::Ordering;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::xp::{self, Bevel, color};

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

/// The table delegate shared by every tab.
#[derive(Debug)]
pub struct ListView {
    columns: Vec<ColumnSpec>,
    rows: Vec<Row>,
    sort: Option<(usize, ColumnSort)>,
    /// The selected row's id, kept here so cells can draw selected text.
    selected: Option<u64>,
}

impl ListView {
    /// Creates an empty list, optionally sorted by `sort`.
    pub fn new(columns: Vec<ColumnSpec>, sort: Option<(usize, ColumnSort)>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            sort,
            selected: None,
        }
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

/// Moves the table's selection to wherever the selected id now sits.
fn reselect(state: &mut TableState<ListView>, cx: &mut Context<TableState<ListView>>) {
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
        // Sorting is driven by our own header buttons: XP's Task Manager draws
        // no sort arrows, so the table's built-in sort affordance stays off.
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
        let caption = div()
            .flex()
            .flex_1()
            .items_center()
            .px(px(4.))
            .when(spec.right, gpui_kit::Styled::justify_end)
            .text_color(color::text())
            .whitespace_nowrap()
            .overflow_hidden()
            .child(spec.name);
        div()
            .id(("th", col_ix))
            .size_full()
            .flex()
            .bg(color::face())
            .on_click(cx.listener(move |state, _, _, cx| {
                let delegate = state.delegate_mut();
                delegate.sort = Some(match delegate.sort {
                    Some((c, ColumnSort::Ascending)) if c == col_ix => {
                        (col_ix, ColumnSort::Descending)
                    }
                    _ => (col_ix, ColumnSort::Ascending),
                });
                delegate.apply_sort();
                reselect(state, cx);
                cx.notify();
            }))
            .child(xp::bevel(Bevel::Raised, caption).flex_1())
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
