//! The Task Manager window: title bar, menus, the five tabs and status bar.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{ColumnSort, DataTable, TableEvent, TableState};
use gpui_kit::component::{Sizable as _, Size};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::about;
use crate::apps::{self, AppTask, Priority};
use crate::graph::{self, Series};
use crate::mac;
use crate::stats::{Sampler, Snapshot};
use crate::table::{self, ColumnSpec, Key, ListView, Row, RowMenu};
use crate::tray::{Tray, TrayEvent};
use crate::xp::{self, Bevel, color};
use futures::StreamExt as _;

actions!(
    task_manager,
    [
        NewTask,
        Exit,
        AlwaysOnTop,
        MinimizeOnUse,
        HideWhenMinimized,
        Minimize,
        RefreshNow,
        SpeedHigh,
        SpeedNormal,
        SpeedLow,
        SpeedPaused,
        OneGraphAllCpus,
        OneGraphPerCpu,
        ShowKernelTimes,
        About,
        // Applications row menu.
        SwitchTo,
        BringToFront,
        MinimizeTask,
        MaximizeTask,
        Cascade,
        TileHorizontally,
        TileVertically,
        EndTask,
        GoToProcess,
        // Processes row menu.
        EndProcess,
        EndProcessTree,
        DebugProcess,
        PriorityRealtime,
        PriorityHigh,
        PriorityAboveNormal,
        PriorityNormal,
        PriorityBelowNormal,
        PriorityLow,
    ]
);

fn priority_action(priority: Priority) -> Box<dyn Action> {
    match priority {
        Priority::Realtime => Box::new(PriorityRealtime),
        Priority::High => Box::new(PriorityHigh),
        Priority::AboveNormal => Box::new(PriorityAboveNormal),
        Priority::Normal => Box::new(PriorityNormal),
        Priority::BelowNormal => Box::new(PriorityBelowNormal),
        Priority::Low => Box::new(PriorityLow),
    }
}

/// The Applications tab's row menu. The bold item is the double-click action.
fn app_menu(focus: FocusHandle) -> RowMenu {
    RowMenu::new(move |_, menu, _, _| {
        menu.action_context(focus.clone())
            .menu_element(Box::new(SwitchTo), |_, _| {
                div().font_weight(FontWeight::BOLD).child("Switch To")
            })
            .menu("Bring To Front", Box::new(BringToFront))
            .separator()
            .menu("Minimize", Box::new(MinimizeTask))
            .menu("Maximize", Box::new(MaximizeTask))
            // XP enables these only with several tasks selected.
            .menu_with_disabled("Cascade", Box::new(Cascade), true)
            .menu_with_disabled("Tile Horizontally", Box::new(TileHorizontally), true)
            .menu_with_disabled("Tile Vertically", Box::new(TileVertically), true)
            .separator()
            .menu("End Task", Box::new(EndTask))
            .menu("Go To Process", Box::new(GoToProcess))
    })
}

/// The Processes tab's row menu; Set Priority checks the current class.
fn process_menu(focus: FocusHandle) -> RowMenu {
    RowMenu::new(move |pid, menu, window, cx| {
        let idle = pid == IDLE_PROCESS_ID;
        let current = if idle {
            None
        } else {
            apps::priority(pid as u32)
        };
        let f = focus.clone();
        let priorities = PopupMenu::build(window, cx, move |m, _, _| {
            Priority::ALL
                .into_iter()
                .fold(m.action_context(f.clone()), |m, p| {
                    m.menu_with_check(p.label(), current == Some(p), priority_action(p))
                })
        });
        // A realistic max width keeps Set Priority opening to the right (see
        // the View menu in `render_menu_bar`).
        menu.action_context(focus.clone())
            .max_w(px(200.))
            .menu_with_disabled("End Process", Box::new(EndProcess), idle)
            .menu_with_disabled("End Process Tree", Box::new(EndProcessTree), idle)
            .menu_with_disabled("Debug", Box::new(DebugProcess), true)
            .separator()
            .item(PopupMenuItem::submenu("Set Priority", priorities).disabled(idle))
    })
}

const TABS: [&str; 5] = [
    "Applications",
    "Processes",
    "Performance",
    "Networking",
    "Users",
];
const HISTORY: usize = 600;
const IDLE_PROCESS_ID: u64 = u64::MAX;

/// How often the counters are refreshed, from View > Update Speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Speed {
    High,
    Normal,
    Low,
    Paused,
}

impl Speed {
    fn interval(self) -> Duration {
        match self {
            Speed::High => Duration::from_millis(500),
            Speed::Normal | Speed::Paused => Duration::from_secs(1),
            Speed::Low => Duration::from_secs(4),
        }
    }
}

/// The root view.
pub struct TaskManager {
    focus: FocusHandle,
    tab: usize,
    snap: Snapshot,
    tick: usize,
    cpu_hist: Vec<f32>,
    kernel_hist: Vec<f32>,
    core_hist: Vec<Vec<f32>>,
    commit_hist: Vec<f32>,
    net_hist: HashMap<String, Vec<f32>>,
    speed: Speed,
    per_core: bool,
    show_kernel: bool,
    all_users: bool,
    on_top: bool,
    minimize_on_use: bool,
    hide_when_minimized: bool,
    /// XP's "tiny footprint" mode: no caption, menu, tabs or status bar.
    compact: bool,
    tray: Option<Tray>,
    about: Option<AnyWindowHandle>,
    apps: Vec<AppTask>,
    app_list: Entity<TableState<ListView>>,
    proc_list: Entity<TableState<ListView>>,
    net_list: Entity<TableState<ListView>>,
    user_list: Entity<TableState<ListView>>,
    sampler: Arc<Mutex<Sampler>>,
    _poll: Task<()>,
    _tray_events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for TaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskManager")
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

fn col(name: &'static str, width: f32, right: bool) -> ColumnSpec {
    ColumnSpec { name, width, right }
}

fn list(
    columns: Vec<ColumnSpec>,
    sort: Option<(usize, ColumnSort)>,
    menu: Option<RowMenu>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TableState<ListView>> {
    let mut view = ListView::new(columns, sort);
    if let Some(menu) = menu {
        view = view.with_row_menu(menu);
    }
    // Header clicks only sort (see `ListView::render_th`); XP lists select
    // whole rows, never columns.
    cx.new(|cx| {
        TableState::new(view, window, cx)
            .col_movable(false)
            .col_selectable(false)
    })
}

impl TaskManager {
    /// Builds the view and starts sampling.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let app_list = list(
            vec![col("Task", 300., false), col("Status", 90., false)],
            Some((0, ColumnSort::Ascending)),
            Some(app_menu(focus.clone())),
            window,
            cx,
        );
        let proc_list = list(
            vec![
                col("Image Name", 130., false),
                col("PID", 50., true),
                col("User Name", 85., false),
                col("CPU", 36., true),
                col("Mem Usage", 80., true),
            ],
            Some((0, ColumnSort::Ascending)),
            Some(process_menu(focus.clone())),
            window,
            cx,
        );
        let net_list = list(
            vec![
                col("Adapter Name", 140., false),
                col("Network Utilization", 115., true),
                col("Link Speed", 75., true),
                col("State", 80., false),
            ],
            None,
            None,
            window,
            cx,
        );
        let user_list = list(
            vec![
                col("User", 110., false),
                col("ID", 35., true),
                col("Status", 70., false),
                col("Client Name", 90., false),
                col("Session", 80., false),
            ],
            None,
            None,
            window,
            cx,
        );

        let mut this = Self {
            focus,
            tab: std::env::var("XP_TASKMGR_TAB")
                .ok()
                .and_then(|t| t.parse().ok())
                .unwrap_or(2)
                .min(4),
            snap: Snapshot::default(),
            tick: 0,
            cpu_hist: Vec::new(),
            kernel_hist: Vec::new(),
            core_hist: Vec::new(),
            commit_hist: Vec::new(),
            net_hist: HashMap::new(),
            speed: Speed::Normal,
            // XP defaults to one graph per CPU on multiprocessor machines.
            per_core: true,
            show_kernel: false,
            all_users: false,
            on_top: false,
            minimize_on_use: false,
            // XP shipped with Hide When Minimized on.
            hide_when_minimized: true,
            compact: false,
            tray: None,
            about: None,
            apps: Vec::new(),
            app_list,
            proc_list,
            net_list,
            user_list,
            sampler: Arc::new(Mutex::new(Sampler::new())),
            _poll: Task::ready(()),
            _tray_events: Task::ready(()),
            _subscriptions: Vec::new(),
        };
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        this.tray = Tray::new(tx);
        this._tray_events = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = rx.next().await {
                let applied = this.update_in(cx, |t, window, cx| match event {
                    TrayEvent::Restore => t.restore(window, cx),
                    TrayEvent::Exit => cx.quit(),
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        let lists = [
            &this.app_list,
            &this.proc_list,
            &this.net_list,
            &this.user_list,
        ];
        this._subscriptions = lists
            .into_iter()
            .map(|list| {
                cx.subscribe_in(list, window, |t, list, event: &TableEvent, window, cx| {
                    match event {
                        TableEvent::SelectRow(ix) => {
                            list.update(cx, |s, cx| {
                                s.delegate_mut().select_index(*ix);
                                cx.notify();
                            });
                            cx.notify();
                        }
                        TableEvent::SelectColumn(_) => {
                            list.update(cx, table::reselect);
                        }
                        // As in XP, right-clicking a row selects it. Selecting
                        // forgets the right-clicked row, which the row menu
                        // needs, so it is set back.
                        TableEvent::RightClickedRow(Some(ix)) => list.update(cx, |s, cx| {
                            s.set_selected_row(*ix, cx);
                            s.set_right_clicked_row(Some(*ix), cx);
                        }),
                        // Double-click runs the row menu's bold item.
                        TableEvent::DoubleClickedRow(_) if *list == t.app_list => {
                            t.switch_to_selected(window, cx);
                        }
                        _ => {}
                    }
                })
            })
            .collect();
        this._poll = Self::poll(cx);
        this.focus.focus(window, cx);
        this
    }

    fn poll(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(speed) = this.read_with(cx, |t, _| t.speed) else {
                    break;
                };
                if speed != Speed::Paused && !Self::sample_once(&this, cx).await {
                    break;
                }
                cx.background_executor().timer(speed.interval()).await;
            }
        })
    }

    /// Takes one sample on the background executor; false once the view is gone.
    async fn sample_once(this: &WeakEntity<Self>, cx: &mut AsyncApp) -> bool {
        let Ok(sampler) = this.read_with(cx, |t, _| t.sampler.clone()) else {
            return false;
        };
        let snap = cx
            .background_executor()
            .spawn(async move {
                let mut sampler = sampler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                sampler.sample()
            })
            .await;
        this.update(cx, |t, cx| t.apply(snap, cx)).is_ok()
    }

    fn apply(&mut self, snap: Snapshot, cx: &mut Context<Self>) {
        fn push(hist: &mut Vec<f32>, v: f32) {
            hist.push(v);
            if hist.len() > HISTORY {
                hist.drain(..hist.len() - HISTORY);
            }
        }
        self.tick += 1;
        push(&mut self.cpu_hist, snap.cpu_total / 100.0);
        push(&mut self.kernel_hist, snap.cpu_kernel / 100.0);
        self.core_hist.resize_with(snap.per_cpu.len(), Vec::new);
        for (hist, v) in self.core_hist.iter_mut().zip(&snap.per_cpu) {
            push(hist, v / 100.0);
        }
        let commit = snap.commit_total as f32 / snap.commit_limit.max(1) as f32;
        push(&mut self.commit_hist, commit);
        for a in &snap.adapters {
            push(
                self.net_hist.entry(a.device.clone()).or_default(),
                a.utilization() as f32 / 100.0,
            );
        }
        if let Some(tray) = &mut self.tray {
            tray.set_cpu(snap.cpu_total);
        }
        self.snap = snap;
        self.apps = apps::running_apps();
        self.refresh_lists(cx);
        cx.notify();
    }

    fn refresh_lists(&self, cx: &mut Context<Self>) {
        let app_rows = self
            .apps
            .iter()
            .map(|a| Row {
                id: a.pid as u64,
                cells: vec![
                    a.name.clone().into(),
                    if a.responding {
                        "Running"
                    } else {
                        "Not Responding"
                    }
                    .into(),
                ],
                keys: vec![Key::Text(a.name.clone()), Key::Text(String::new())],
            })
            .collect();
        self.app_list
            .update(cx, |s, cx| table::set_rows(s, app_rows, cx));

        // SAFETY: getuid has no preconditions.
        let me = unsafe { libc::getuid() };
        let mut proc_rows: Vec<Row> = self
            .snap
            .processes
            .iter()
            .filter(|p| self.all_users || p.uid == me)
            .map(|p| {
                let cpu = p.cpu.round() as u32;
                Row {
                    id: u64::from(p.pid),
                    cells: vec![
                        p.name.clone().into(),
                        p.pid.to_string().into(),
                        p.user.clone().into(),
                        format!("{cpu:02}").into(),
                        format!("{} K", thousands(p.memory / 1024)).into(),
                    ],
                    keys: vec![
                        Key::Text(p.name.clone()),
                        Key::Num(f64::from(p.pid)),
                        Key::Text(p.user.clone()),
                        Key::Num(f64::from(p.cpu)),
                        Key::Num(p.memory as f64),
                    ],
                }
            })
            .collect();
        if self.all_users {
            let idle = (100.0 - self.snap.cpu_total).clamp(0.0, 100.0);
            proc_rows.push(Row {
                id: IDLE_PROCESS_ID,
                cells: vec![
                    "System Idle Process".into(),
                    "0".into(),
                    "SYSTEM".into(),
                    format!("{:02}", idle.round() as u32).into(),
                    "16 K".into(),
                ],
                keys: vec![
                    Key::Text("System Idle Process".into()),
                    Key::Num(-1.0),
                    Key::Text("SYSTEM".into()),
                    Key::Num(f64::from(idle)),
                    Key::Num(16.0 * 1024.0),
                ],
            });
        }
        self.proc_list
            .update(cx, |s, cx| table::set_rows(s, proc_rows, cx));

        let net_rows = self
            .snap
            .adapters
            .iter()
            .enumerate()
            .map(|(ix, a)| {
                let util = a.utilization();
                Row {
                    id: ix as u64,
                    cells: vec![
                        a.name.clone().into(),
                        if util < 1.0 {
                            format!("{util:.2} %")
                        } else {
                            format!("{util:.0} %")
                        }
                        .into(),
                        link_speed(a.link_speed).into(),
                        if a.connected {
                            "Connected"
                        } else {
                            "Disconnected"
                        }
                        .into(),
                    ],
                    keys: vec![
                        Key::Text(a.name.clone()),
                        Key::Num(util),
                        Key::Num(a.link_speed as f64),
                        Key::Text(String::new()),
                    ],
                }
            })
            .collect();
        self.net_list
            .update(cx, |s, cx| table::set_rows(s, net_rows, cx));

        let user_rows = self
            .snap
            .sessions
            .iter()
            .map(|u| Row {
                id: u64::from(u.id),
                cells: vec![
                    u.user.clone().into(),
                    u.id.to_string().into(),
                    "Active".into(),
                    u.client.clone().into(),
                    u.session.clone().into(),
                ],
                keys: vec![
                    Key::Text(u.user.clone()),
                    Key::Num(f64::from(u.id)),
                    Key::Text(String::new()),
                    Key::Text(u.client.clone()),
                    Key::Text(u.session.clone()),
                ],
            })
            .collect();
        self.user_list
            .update(cx, |s, cx| table::set_rows(s, user_rows, cx));
    }

    /// Wires focus and every menu action onto the root element.
    fn with_actions(&self, root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        root.track_focus(&self.focus)
            .key_context("TaskManager")
            .on_action(cx.listener(|t, _: &NewTask, window, cx| t.open_run_dialog(window, cx)))
            .on_action(|_: &Exit, _, cx| cx.quit())
            .on_action(cx.listener(|t, _: &AlwaysOnTop, window, cx| {
                t.on_top = !t.on_top;
                mac::set_always_on_top(window, t.on_top);
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &HideWhenMinimized, _, cx| {
                t.hide_when_minimized = !t.hide_when_minimized;
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &Minimize, window, _| t.minimize(window)))
            .on_action(cx.listener(|t, _: &MinimizeOnUse, _, cx| {
                t.minimize_on_use = !t.minimize_on_use;
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &RefreshNow, _, cx| t.refresh_now(cx)))
            .on_action(cx.listener(|t, _: &SpeedHigh, _, cx| t.set_speed(Speed::High, cx)))
            .on_action(cx.listener(|t, _: &SpeedNormal, _, cx| t.set_speed(Speed::Normal, cx)))
            .on_action(cx.listener(|t, _: &SpeedLow, _, cx| t.set_speed(Speed::Low, cx)))
            .on_action(cx.listener(|t, _: &SpeedPaused, _, cx| t.set_speed(Speed::Paused, cx)))
            .on_action(cx.listener(|t, _: &OneGraphAllCpus, _, cx| {
                t.per_core = false;
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &OneGraphPerCpu, _, cx| {
                t.per_core = true;
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &ShowKernelTimes, _, cx| {
                t.show_kernel = !t.show_kernel;
                cx.notify();
            }))
            .on_action(cx.listener(|t, _: &About, _, cx| t.open_about(cx)))
            .on_action(cx.listener(|t, _: &SwitchTo, window, cx| t.switch_to_selected(window, cx)))
            .on_action(cx.listener(|t, _: &BringToFront, _, cx| {
                t.with_selected_app(cx, apps::bring_to_front);
            }))
            .on_action(cx.listener(|t, _: &MinimizeTask, _, cx| {
                t.with_selected_app(cx, apps::minimize);
            }))
            .on_action(cx.listener(|t, _: &MaximizeTask, _, cx| {
                t.with_selected_app(cx, apps::maximize);
            }))
            .on_action(cx.listener(|t, _: &EndTask, _, cx| t.end_selected_task(cx)))
            .on_action(cx.listener(|t, _: &GoToProcess, _, cx| t.go_to_process(cx)))
            .on_action(cx.listener(|t, _: &EndProcess, window, cx| {
                t.end_selected_process(window, cx);
            }))
            .on_action(cx.listener(|t, _: &EndProcessTree, window, cx| {
                t.end_selected_tree(window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityRealtime, window, cx| {
                t.set_selected_priority(Priority::Realtime, window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityHigh, window, cx| {
                t.set_selected_priority(Priority::High, window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityAboveNormal, window, cx| {
                t.set_selected_priority(Priority::AboveNormal, window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityNormal, window, cx| {
                t.set_selected_priority(Priority::Normal, window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityBelowNormal, window, cx| {
                t.set_selected_priority(Priority::BelowNormal, window, cx);
            }))
            .on_action(cx.listener(|t, _: &PriorityLow, window, cx| {
                t.set_selected_priority(Priority::Low, window, cx);
            }))
    }

    fn set_compact(&mut self, compact: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.compact = compact;
        mac::set_min_size(window, if compact { (160., 100.) } else { (400., 440.) });
        cx.notify();
    }

    fn set_speed(&mut self, speed: Speed, cx: &mut Context<Self>) {
        self.speed = speed;
        // Restart the loop so the new interval applies right away.
        self._poll = Self::poll(cx);
        cx.notify();
    }

    fn refresh_now(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            Self::sample_once(&this, cx).await;
        })
        .detach();
    }

    fn open_run_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx));
        let error: Entity<Option<String>> = cx.new(|_| None);
        let first = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                input.focus_handle(cx).focus(window, cx);
            }
            let run = {
                let input = input.clone();
                let error = error.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    let target = input.read(cx).value().to_string();
                    match apps::run(&target) {
                        Ok(()) => window.close_dialog(cx),
                        Err(e) => error.update(cx, |err, cx| {
                            *err = Some(e);
                            cx.notify();
                        }),
                    }
                }
            };
            let message = error.read(cx).clone();
            dialog
                .title("Create New Task")
                .w(px(360.))
                .bg(color::face())
                .rounded_none()
                .text_color(color::text())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(
                            "Type the name of a program, folder, document, or Internet resource, \
                             and macOS will open it for you.",
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child("Open:")
                                .child(div().flex_1().child(Input::new(&input).small())),
                        )
                        .when_some(message, |el, m| {
                            el.child(div().text_color(rgb(0xC00000)).child(m))
                        })
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(px(6.))
                                .child(xp::button("run-ok", "OK", true, run))
                                .child(xp::button(
                                    "run-cancel",
                                    "Cancel",
                                    true,
                                    |_, window, cx| {
                                        window.close_dialog(cx);
                                    },
                                )),
                        ),
                )
        });
    }

    fn open_about(&mut self, cx: &mut Context<Self>) {
        // Only one About box at a time, as on XP.
        if let Some(handle) = self.about
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        self.about = about::open(self.snap.phys_total, cx);
    }

    /// Minimizes, or with Hide When Minimized leaves only the tray meter.
    fn minimize(&self, window: &mut Window) {
        if self.hide_when_minimized && self.tray.is_some() {
            mac::hide_to_tray(window);
        } else {
            window.minimize_window();
        }
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        mac::restore_from_tray(window);
        cx.activate(true);
        window.activate_window();
    }

    fn switch_to_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pid) = table::selected_id(self.app_list.read(cx)) {
            apps::switch_to(pid as i32);
            if self.minimize_on_use {
                self.minimize(window);
            }
        }
    }

    fn end_selected_task(&mut self, cx: &mut Context<Self>) {
        if let Some(pid) = table::selected_id(self.app_list.read(cx)) {
            apps::end_task(pid as i32);
            self.refresh_now(cx);
        }
    }

    fn with_selected_app(&self, cx: &Context<Self>, f: impl FnOnce(i32)) {
        if let Some(pid) = table::selected_id(self.app_list.read(cx)) {
            f(pid as i32);
        }
    }

    /// Shows the selected application's process on the Processes tab.
    fn go_to_process(&mut self, cx: &mut Context<Self>) {
        let Some(pid) = table::selected_id(self.app_list.read(cx)) else {
            return;
        };
        self.tab = 1;
        self.proc_list
            .update(cx, |s, cx| table::select_id(s, pid, cx));
        cx.notify();
    }

    /// The selected process's pid and name, unless it is the idle process.
    fn selected_process(&self, cx: &App) -> Option<(u32, String)> {
        let pid = table::selected_id(self.proc_list.read(cx))?;
        if pid == IDLE_PROCESS_ID {
            return None;
        }
        let pid = pid as u32;
        let name = self
            .snap
            .processes
            .iter()
            .find(|p| p.pid == pid)
            .map_or_else(String::new, |p| p.name.clone());
        Some((pid, name))
    }

    fn end_selected_process(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pid, name)) = self.selected_process(cx) else {
            return;
        };
        let this = cx.entity().downgrade();
        confirm(
            window,
            cx,
            "Task Manager Warning",
            format!(
                "WARNING: Terminating a process can cause undesired results including \
                 loss of data and system instability. The process will not be given \
                 the chance to save its state or data before it is terminated. Are you \
                 sure you want to terminate \"{name}\" (PID {pid})?"
            ),
            move |_, cx| {
                apps::kill(pid, true);
                if let Some(this) = this.upgrade() {
                    this.update(cx, TaskManager::refresh_now);
                }
            },
        );
    }

    /// Kills the selected process and all of its descendants, leaves first.
    fn end_selected_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pid, name)) = self.selected_process(cx) else {
            return;
        };
        let mut tree = vec![pid];
        let mut ix = 0;
        while let Some(&parent) = tree.get(ix) {
            let children: Vec<u32> = self
                .snap
                .processes
                .iter()
                .filter(|p| p.parent == Some(parent) && !tree.contains(&p.pid))
                .map(|p| p.pid)
                .collect();
            tree.extend(children);
            ix += 1;
        }
        let this = cx.entity().downgrade();
        confirm(
            window,
            cx,
            "Task Manager Warning",
            format!(
                "WARNING: Terminating a process tree can cause undesired results including \
                 loss of data and system instability. The processes will not be given \
                 the chance to save their state or data before they are terminated. Are \
                 you sure you want to terminate the process tree of \"{name}\" (PID {pid})?"
            ),
            move |_, cx| {
                for &pid in tree.iter().rev() {
                    apps::kill(pid, true);
                }
                if let Some(this) = this.upgrade() {
                    this.update(cx, TaskManager::refresh_now);
                }
            },
        );
    }

    fn set_selected_priority(
        &mut self,
        priority: Priority,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pid, _)) = self.selected_process(cx) else {
            return;
        };
        confirm(
            window,
            cx,
            "Task Manager Warning",
            "WARNING: Changing the priority class of this process may cause undesired \
             results including system instability. Are you sure you want to change the \
             priority class?"
                .into(),
            move |window, cx| {
                if let Err(err) = apps::set_priority(pid, priority) {
                    let reason = if err.kind() == std::io::ErrorKind::PermissionDenied {
                        "Access is denied.".to_owned()
                    } else {
                        err.to_string()
                    };
                    alert(
                        window,
                        cx,
                        "Unable to Change Priority",
                        format!("The operation could not be completed.\n\n{reason}"),
                    );
                }
            },
        );
    }

    // Rendering ---------------------------------------------------------

    fn render_title_bar(window: &Window) -> impl IntoElement {
        let minimize = div().w(px(6.)).h(px(2.)).mt(px(6.)).bg(color::text());
        let maximize = div()
            .w(px(9.))
            .h(px(8.))
            .border_1()
            .border_t_2()
            .border_color(color::text());
        xp::title_bar(
            window,
            Some(app_icon().into_any_element()),
            "macOS Task Manager",
            vec![
                xp::caption_button("minimize", minimize, |window, cx| {
                    window.dispatch_action(Box::new(Minimize), cx);
                }),
                xp::caption_button("maximize", maximize, |window, _| window.zoom_window()),
                div().w(px(2.)).into_any_element(),
                xp::caption_button("close", xp::close_glyph(), |_, cx| cx.quit()),
            ],
        )
    }

    fn render_menu_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let focus = self.focus.clone();
        let menu_button = |id: &'static str, label: &'static str| {
            Button::new(id)
                .ghost()
                .compact()
                .with_size(Size::Size(px(18.)))
                .rounded_none()
                .px(px(6.))
                .text_color(color::text())
                .label(label)
        };
        let f = focus.clone();
        let file = menu_button("menu-file", "File").dropdown_menu(move |menu, _, _| {
            menu.action_context(f.clone())
                .menu("New Task (Run...)", Box::new(NewTask))
                .separator()
                .menu("Exit Task Manager", Box::new(Exit))
        });
        let (t, f) = (this.clone(), focus.clone());
        let options = menu_button("menu-options", "Options").dropdown_menu(move |menu, _, cx| {
            let s = t.read(cx);
            menu.action_context(f.clone())
                .menu_with_check("Always On Top", s.on_top, Box::new(AlwaysOnTop))
                .menu_with_check(
                    "Minimize On Use",
                    s.minimize_on_use,
                    Box::new(MinimizeOnUse),
                )
                .menu_with_check(
                    "Hide When Minimized",
                    s.hide_when_minimized,
                    Box::new(HideWhenMinimized),
                )
        });
        let (t, f) = (this.clone(), focus.clone());
        let view = menu_button("menu-view", "View").dropdown_menu(move |menu, window, cx| {
            let s = t.read(cx);
            let (speed, per_core, kernel) = (s.speed, s.per_core, s.show_kernel);
            let f2 = f.clone();
            let f3 = f.clone();
            // gpui-component opens submenus to the left when origin + max width
            // overflows the window; the 500px default always does in this
            // narrow window, so cap it at a realistic width.
            menu.action_context(f.clone())
                .max_w(px(220.))
                .menu("Refresh Now", Box::new(RefreshNow))
                .submenu("Update Speed", window, cx, move |m, _, _| {
                    m.action_context(f2.clone())
                        .menu_with_check("High", speed == Speed::High, Box::new(SpeedHigh))
                        .menu_with_check("Normal", speed == Speed::Normal, Box::new(SpeedNormal))
                        .menu_with_check("Low", speed == Speed::Low, Box::new(SpeedLow))
                        .menu_with_check("Paused", speed == Speed::Paused, Box::new(SpeedPaused))
                })
                .separator()
                .submenu("CPU History", window, cx, move |m, _, _| {
                    m.action_context(f3.clone())
                        .menu_with_check(
                            "One Graph, All CPUs",
                            !per_core,
                            Box::new(OneGraphAllCpus),
                        )
                        .menu_with_check("One Graph Per CPU", per_core, Box::new(OneGraphPerCpu))
                })
                .menu_with_check("Show Kernel Times", kernel, Box::new(ShowKernelTimes))
        });
        let f = focus;
        let help = menu_button("menu-help", "Help").dropdown_menu(move |menu, _, _| {
            menu.action_context(f.clone())
                .menu("About Task Manager", Box::new(About))
        });
        div()
            .flex()
            .items_center()
            .h(px(20.))
            .px(px(2.))
            .child(file)
            .child(options)
            .child(view)
            .child(help)
    }

    fn render_list(state: &Entity<TableState<ListView>>) -> impl IntoElement {
        xp::bevel(
            Bevel::Sunken,
            div().flex_1().min_h_0().bg(color::highlight()).child(
                DataTable::new(state)
                    .with_size(Size::Size(px(17.)))
                    .header_row_height(px(18.))
                    .bordered(false)
                    .stripe(false),
            ),
        )
        .flex_1()
        .min_h_0()
    }

    fn render_applications(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_sel = table::selected_id(self.app_list.read(cx)).is_some();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(8.))
            .child(Self::render_list(&self.app_list))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(xp::button(
                        "end-task",
                        "End Task",
                        has_sel,
                        cx.listener(|t, _, _, cx| t.end_selected_task(cx)),
                    ))
                    .child(xp::button(
                        "switch-to",
                        "Switch To",
                        has_sel,
                        cx.listener(|t, _, window, cx| t.switch_to_selected(window, cx)),
                    ))
                    .child(xp::button(
                        "new-task",
                        "New Task...",
                        true,
                        cx.listener(|t, _, window, cx| t.open_run_dialog(window, cx)),
                    )),
            )
    }

    fn render_processes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_sel = table::selected_id(self.proc_list.read(cx)).is_some();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(8.))
            .child(Self::render_list(&self.proc_list))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(xp::checkbox(
                        "all-users",
                        "Show processes from all users",
                        self.all_users,
                        cx.listener(|t, _, _, cx| {
                            t.all_users = !t.all_users;
                            t.refresh_lists(cx);
                            cx.notify();
                        }),
                    ))
                    .child(xp::button(
                        "end-process",
                        "End Process",
                        has_sel,
                        cx.listener(|t, _, window, cx| t.end_selected_process(window, cx)),
                    )),
            )
    }

    fn render_performance(&self) -> AnyElement {
        let s = &self.snap;
        let pf_mb = s.commit_total / 1024;
        let pf_fraction = s.commit_total as f32 / s.commit_limit.max(1) as f32;

        let per_cpu_grid = self.per_core && self.core_hist.len() > 1;
        let cpu_history = if per_cpu_grid {
            graph::history_grid(self.core_hist.clone(), self.tick).into_any_element()
        } else {
            let mut series = vec![Series {
                values: self.cpu_hist.clone(),
                color: color::graph_line(),
            }];
            if self.show_kernel {
                series.push(Series {
                    values: self.kernel_hist.clone(),
                    color: color::graph_kernel(),
                });
            }
            graph::history(series, self.tick).into_any_element()
        };
        let pf_history = graph::history(
            vec![Series {
                values: self.commit_hist.clone(),
                color: color::graph_commit(),
            }],
            self.tick,
        );

        let graph_row = |meter_title: &'static str,
                         meter: AnyElement,
                         history_title: &'static str,
                         history: AnyElement,
                         per_cpu_grid: bool| {
            div()
                .flex()
                .flex_1()
                .min_h(px(80.))
                .gap(px(10.))
                .child(xp::frame(meter_title, xp::bevel(Bevel::Sunken, meter).flex_1()).w(px(84.)))
                .child(
                    xp::frame(
                        history_title,
                        // The per-CPU grid draws a sunken box per graph itself.
                        if per_cpu_grid {
                            div().flex().flex_col().flex_1().child(history)
                        } else {
                            xp::bevel(Bevel::Sunken, history).flex_1()
                        },
                    )
                    .flex_1(),
                )
        };

        let stat_frame = |title: &'static str, rows: [(&'static str, u64); 3]| {
            xp::frame(
                title,
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().map(|(k, v)| {
                        div()
                            .flex()
                            .justify_between()
                            .line_height(px(16.))
                            .child(k)
                            .child(v.to_string())
                    })),
            )
            .flex_1()
        };

        let cpu_row = graph_row(
            "CPU Usage",
            graph::meter(
                s.cpu_total / 100.0,
                format!("{} %", s.cpu_total.round() as u32),
            )
            .into_any_element(),
            "CPU Usage History",
            cpu_history,
            per_cpu_grid,
        );
        // Tiny footprint mode keeps only the CPU row, stretched to the window.
        if self.compact {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(cpu_row)
                .into_any_element();
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(6.))
            .px(px(4.))
            .child(cpu_row)
            .child(graph_row(
                "PF Usage",
                graph::meter(pf_fraction, format!("{pf_mb} MB")).into_any_element(),
                "Page File Usage History",
                pf_history.into_any_element(),
                false,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .child(stat_frame(
                        "Totals",
                        [
                            ("Handles", s.handles),
                            ("Threads", s.threads),
                            ("Processes", s.processes.len() as u64),
                        ],
                    ))
                    .child(stat_frame(
                        "Physical Memory (K)",
                        [
                            ("Total", s.phys_total),
                            ("Available", s.phys_available),
                            ("System Cache", s.system_cache),
                        ],
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .child(stat_frame(
                        "Commit Charge (K)",
                        [
                            ("Total", s.commit_total),
                            ("Limit", s.commit_limit),
                            ("Peak", s.commit_peak),
                        ],
                    ))
                    .child(stat_frame(
                        "Kernel Memory (K)",
                        [
                            ("Total", s.kernel_paged + s.kernel_nonpaged),
                            ("Paged", s.kernel_paged),
                            ("Nonpaged", s.kernel_nonpaged),
                        ],
                    )),
            )
            .into_any_element()
    }

    fn render_networking(&self) -> impl IntoElement {
        // Graph the adapters that matter: known link speed first, then busiest.
        let mut shown: Vec<_> = self.snap.adapters.iter().filter(|a| a.connected).collect();
        shown.sort_by(|a, b| {
            (b.link_speed > 0)
                .cmp(&(a.link_speed > 0))
                .then(b.bits_per_sec.total_cmp(&a.bits_per_sec))
        });
        let graphs = shown.into_iter().take(2).map(|a| {
            let hist = self.net_hist.get(&a.device).cloned().unwrap_or_default();
            let peak = hist.iter().rev().take(200).copied().fold(0.0f32, f32::max) * 100.0;
            let scale = [0.1f32, 0.5, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0]
                .into_iter()
                .find(|s| *s >= peak)
                .unwrap_or(100.0);
            let values = hist.iter().map(|v| v * 100.0 / scale).collect();
            let fmt = |v: f32| {
                if v < 1.0 {
                    format!("{v} %")
                } else {
                    format!("{v:.0} %")
                }
            };
            let axis = div()
                .w(px(44.))
                .flex()
                .flex_col()
                .justify_between()
                .items_end()
                .text_size(px(10.))
                .child(fmt(scale))
                .child(fmt(scale / 2.0))
                .child("0 %");
            xp::frame(
                a.name.clone(),
                div().flex().flex_1().gap(px(4.)).child(axis).child(
                    xp::bevel(
                        Bevel::Sunken,
                        graph::history(
                            vec![Series {
                                values,
                                color: color::graph_line(),
                            }],
                            self.tick,
                        ),
                    )
                    .flex_1(),
                ),
            )
            .flex_1()
            .min_h(px(90.))
        });
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(6.))
            .children(graphs)
            .child(
                div()
                    .h(px(110.))
                    .flex()
                    .flex_col()
                    .child(Self::render_list(&self.net_list)),
            )
    }

    fn render_users(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(8.))
            .child(Self::render_list(&self.user_list))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(xp::button("disconnect", "Disconnect", false, |_, _, _| {}))
                    .child(xp::button("logoff", "Logoff", false, |_, _, _| {}))
                    .child(xp::button(
                        "send-message",
                        "Send Message...",
                        false,
                        |_, _, _| {},
                    )),
            )
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let s = &self.snap;
        let pane = |text: String, w: Option<f32>| {
            let content = div()
                .px(px(3.))
                .whitespace_nowrap()
                .overflow_hidden()
                .child(text);
            let pane = xp::bevel(Bevel::Thin, content).h(px(18.)).justify_center();
            match w {
                Some(w) => pane.w(px(w)),
                None => pane.flex_1(),
            }
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(2.))
            .py(px(2.))
            .child(pane(
                format!("Processes: {}", s.processes.len()),
                Some(110.),
            ))
            .child(pane(
                format!("CPU Usage: {}%", s.cpu_total.round() as u32),
                Some(130.),
            ))
            .child(pane(
                format!("Commit Charge: {}K / {}K", s.commit_total, s.commit_limit),
                None,
            ))
            .child(size_grip())
    }
}

impl Focusable for TaskManager {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TaskManager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.tab {
            0 => self.render_applications(cx).into_any_element(),
            1 => self.render_processes(cx).into_any_element(),
            2 => self.render_performance(),
            3 => self.render_networking().into_any_element(),
            _ => self.render_users().into_any_element(),
        };

        // Double-clicking the Performance page toggles tiny footprint mode, as
        // on XP; in that mode a plain drag anywhere moves the window.
        let toggles = self.tab == 2;
        let page = div()
            .id("page")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |t, e: &MouseDownEvent, window, cx| {
                    if toggles && e.click_count == 2 {
                        t.set_compact(!t.compact, window, cx);
                    } else if t.compact {
                        window.start_window_move();
                    }
                }),
            )
            .child(page);

        if self.compact {
            let content = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .p(px(6.))
                .child(page);
            return self
                .with_actions(div().id("task-manager"), cx)
                .size_full()
                .flex()
                .flex_col()
                .bg(color::face())
                .text_color(color::text())
                .font_family("Tahoma")
                .text_size(px(11.5))
                .child(xp::bevel(Bevel::Raised, content).size_full());
        }

        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(px(6.))
            .pt(px(6.))
            .pb(px(4.))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(
                        xp::bevel(
                            Bevel::Raised,
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h_0()
                                .p(px(10.))
                                .child(page),
                        )
                        .mt(px(xp::TAB_STRIP_HEIGHT))
                        .flex_1()
                        .min_h_0(),
                    )
                    .child(
                        xp::tab_strip(&TABS, self.tab, cx, |t: &mut Self, ix, cx| {
                            t.tab = ix;
                            cx.notify();
                        })
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0(),
                    ),
            );

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .p(px(2.))
            .child(Self::render_title_bar(window))
            .child(self.render_menu_bar(cx))
            .child(div().h(px(1.)).bg(color::shadow()))
            .child(div().h(px(1.)).bg(color::highlight()))
            .child(body)
            .child(self.render_status_bar());

        self.with_actions(div().id("task-manager"), cx)
            .size_full()
            .flex()
            .flex_col()
            .bg(color::face())
            .text_color(color::text())
            .font_family("Tahoma")
            .text_size(px(11.5))
            .child(xp::bevel(Bevel::Raised, content).size_full())
    }
}

/// The app icon, shown small in the caption like XP's window icon.
pub fn app_icon() -> impl IntoElement {
    static ICON: std::sync::LazyLock<Arc<Image>> = std::sync::LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../assets/titlebar-icon.png").to_vec(),
        ))
    });
    img(ICON.clone()).size(px(16.))
}

/// The diagonal hatching in the bottom right corner.
fn size_grip() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, (), window, _| {
            let o = bounds.origin;
            let s = f32::from(bounds.size.width);
            for i in 0..3 {
                let off = 3.0 + i as f32 * 4.0;
                for (dx, c) in [
                    (0.0, color::highlight()),
                    (1.0, color::shadow()),
                    (2.0, color::shadow()),
                ] {
                    let mut path = PathBuilder::stroke(px(1.));
                    path.move_to(o + point(px(s - off + dx), px(s)));
                    path.line_to(o + point(px(s), px(s - off + dx)));
                    if let Ok(p) = path.build() {
                        window.paint_path(p, c);
                    }
                }
            }
        },
    )
    .size(px(14.))
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn link_speed(bps: u64) -> String {
    match bps {
        0 => "-".into(),
        b if b >= 1_000_000_000 && b % 1_000_000_000 == 0 => format!("{} Gbps", b / 1_000_000_000),
        b if b >= 1_000_000 => format!("{} Mbps", b / 1_000_000),
        b => format!("{} Kbps", b / 1_000),
    }
}

/// An XP Yes/No warning dialog; `on_yes` runs once it has closed.
fn confirm(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    message: String,
    on_yes: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_yes = Rc::new(on_yes);
    window.open_dialog(cx, move |dialog, _, _| {
        let on_yes = on_yes.clone();
        message_dialog(
            dialog,
            title,
            message.clone(),
            vec![
                xp::button("confirm-yes", "Yes", true, move |_, window, cx| {
                    window.close_dialog(cx);
                    on_yes(window, cx);
                })
                .into_any_element(),
                xp::button("confirm-no", "No", true, |_, window, cx| {
                    window.close_dialog(cx);
                })
                .into_any_element(),
            ],
        )
    });
}

/// An XP error dialog with a single OK button.
fn alert(window: &mut Window, cx: &mut App, title: &'static str, message: String) {
    window.open_dialog(cx, move |dialog, _, _| {
        message_dialog(
            dialog,
            title,
            message.clone(),
            vec![
                xp::button("alert-ok", "OK", true, |_, window, cx| {
                    window.close_dialog(cx);
                })
                .into_any_element(),
            ],
        )
    });
}

fn message_dialog(
    dialog: Dialog,
    title: &'static str,
    message: String,
    buttons: Vec<AnyElement>,
) -> Dialog {
    dialog
        .title(title)
        .w(px(400.))
        .bg(color::face())
        .rounded_none()
        .text_color(color::text())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(message)
                .child(div().flex().justify_center().gap(px(6.)).children(buttons)),
        )
}

#[cfg(test)]
mod tests {
    use super::{link_speed, thousands};

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn formats_link_speed() {
        assert_eq!(link_speed(0), "-");
        assert_eq!(link_speed(1_000_000_000), "1 Gbps");
        assert_eq!(link_speed(866_000_000), "866 Mbps");
        assert_eq!(link_speed(100_000_000), "100 Mbps");
    }
}
