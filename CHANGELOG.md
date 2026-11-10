# Changelog

## 1.4.6 - 2026-11-10

- Added an automatic update check. The About box now has a "Download and install updates automatically" option, off by default and remembered between runs: when it is on, a release is looked for every 10 minutes and downloaded in the background, then a dialog asks before restarting and installs on its own after 15 seconds unless you cancel.
- Menu options are now remembered between runs: Always On Top, Minimize On Use, Hide When Minimized, Show Kernel Times and the CPU History choice. Update Speed is still reset to Normal on start.

## 1.4.5 - 2026-11-10

- Fixed a further crash (`CFRelease() called with NULL`) in GPU process sampling. A registry entry without an `AppUsage` or `IOUserClientCreator` property is now treated as a missing reading instead of aborting the app; the 1.4.4 fix covered the key string but not the property value.

## 1.4.4 - 2026-11-10

- Fixed a crash (`CFRelease() called with NULL`) in GPU sampling. A failed CoreFoundation string allocation is now treated as a missing reading instead of being released.

## 1.4.3 - 2026-11-04

- Double-clicking the Networking tab now toggles tiny footprint mode, as on the Performance and GPU tabs, showing only the adapter graphs.

## 1.4.2 - 2026-11-04

- Double-clicking the GPU tab now toggles tiny footprint mode, as on the Performance tab, showing only the GPU usage graph.

## 1.4.1 - 2026-11-04

- Numeric columns (PID, CPU, GPU, Mem Usage and others) are now right-aligned in list views, matching their headers.

## 1.4.0 - 2026-11-02

- The Processes tab now includes a sortable **GPU** column showing per-process GPU utilization.

## 1.3.0 - 2026-10-25

- The tray icon now shows two bar meters side by side: **CPU** on the left and **GPU** on the right. The tooltip reports both percentages.

## 1.2.0 - 2026-10-23

- New **GPU** tab: GPU usage and GPU memory meters with history graphs, renderer and tiler utilization, memory in use and allocated, and the GPU model and core count.

## 1.1.1 - 2026-10-21

- One Graph Per CPU now tiles in full, even rows (8 CPUs show as 4x2 instead of 5+3).

## 1.1.0 - 2026-10-21

- About box links to the GitHub project.
- New **Check for Updates** button: checks the latest release and upgrades the app in place, then relaunches.
- Fixed Classic table sort indicators, header ellipsis, and hover and pressed feedback on column headers.

## 1.0.0 - 2026-10-19

The first release: the Windows XP Task Manager, rebuilt natively for macOS with GPUI and gpui-component.

- Applications, Processes, Performance, Networking and Users tabs on live macOS data
- One CPU history graph per core, tiled to fit, plus tiny footprint mode (double-click the graphs)
- CPU meter in the menu bar, with Hide When Minimized
- Windows Classic look with square corners, XP menus and an XP-style About box
- XP right-click menus on Applications and Processes rows: Switch To (also on double-click), Bring To Front, Minimize, Maximize, End Task, Go To Process, End Process, End Process Tree and Set Priority
- Click a column header to sort; Cmd+Q quits
