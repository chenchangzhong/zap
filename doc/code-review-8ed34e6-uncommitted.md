# Code Review

- **Scope**: Uncommitted changes (5 files) — `app/src/dsh/pane.rs`, `app/src/dsh/runtime.rs`, `app/src/lib.rs`, `app/src/undo_close/stack.rs`, `app/src/workspace/view.rs`: the dsh (DeepSeek Harness) pane stop-registration + undo-restore fix
- **Commit**: 8ed34e6
- **Date**: 2026-09-17
- **Mode**: local
- **Confidence threshold**: 80

## Summary

The change stops a singleton dsh child from being SIGTERMed by the late teardown of an already-closed pane and lets the New Session menu restore a still-in-grace dsh tab; the confirmed defect is that stop registration now happens only on `DetachType::Closed`, which the window-teardown path never emits, so the dsh child (and its token-bearing local port) leaks until app quit.

## Issues

1. Window teardown never registers a pending stop, so the dsh child leaks until app quit (severity: major, confidence: 85)
   - `app/src/dsh/pane.rs:919` — `DshPane::detach` now calls `mark_stop_pending()` only for `DetachType::Closed`. But closing a **window** only ever produces `HiddenForClose`: `Workspace::on_window_closed` (`app/src/workspace/view.rs:22783-22788`) calls `pane_group.detach_panes()`, which detaches every pane with `DetachType::HiddenForClose` (`app/src/pane_group/mod.rs:5821`). Nothing later emits `Closed` for that window:
     - `ClosedItem::Window::discard` is a no-op once the window is gone — `window_workspace()` returns `None` (`app/src/undo_close/stack.rs:76`, `:419`);
     - `clean_up_pane_group` early-returns on `!ctx.is_window_open(window_id)` (`app/src/undo_close/stack.rs:135-140`).
     So `mark_stop_pending` is **never called** (not merely delayed), and the only remaining stop is the app-exit fallback `on_will_terminate` (`app/src/lib.rs:1899-1902`). The codebase already documents this exact gap one line above the webview fallback: "窗口关闭走 HiddenForClose(仅隐藏,供 undo 恢复),但窗口不恢复时 pane 的 Closed detach 不会发生" (`app/src/lib.rs:2140-2142`) — the webview gets a `cleanup_window` fallback there, the child process gets nothing.
   - **Impact**: a local Node agent service that can execute shell commands stays alive, bound to a port, with a valid access token, while being completely invisible in the UI. It also silently contradicts the close-confirmation dialog, which still tells the user the session will be interrupted (`app/src/quit_warning/mod.rs:206-209`). This is a **regression**: before the change `HiddenForClose` called `request_stop()`, and git history shows commit `c2efed09d` ("fix: 关闭 tab 后 dsh 进程残留(HiddenForClose 未触发停止)") fixed this same class of leak. Trigger: dsh pane as the only tab in a window + `Cmd+W` → `ctx.close_window()`; the app stays alive on macOS with no window.
   - **Suggested fix** (option A — local, preferred): register the pending stop for `HiddenForClose` too. Because the decision is deferred to the per-frame `poll_pending_stop(has_dsh_pane)`, this does **not** reintroduce the original kill-the-new-instance bug: a closed *tab* keeps its pane in the current window's `pane_contents`, so `any_dsh_pane` still reports `true` and the pending stop is cancelled (process preserved for undo/restore), whereas a closed *window* is gone from `window_ids()` → `false` → the child is actually stopped.

     ```rust
     // app/src/dsh/pane.rs, in `detach`
     if matches!(detach_type, DetachType::Closed | DetachType::HiddenForClose) {
         DshRuntime::handle(ctx).update(ctx, |runtime, _ctx| {
             runtime.mark_stop_pending();
         });
     }
     ```

     Option B (align with the existing webview fallback): in `on_window_will_close` (`app/src/lib.rs:2137-2147`), next to `BrowserWebViewManager::cleanup_window`, add a matching dsh fallback that calls `mark_stop_pending()`. A and B are complementary; A is required for the common "close the dsh tab's window" case, B additionally covers windows torn down without a pane `detach`.

## Below-threshold observations (not confirmed, reported for awareness only)

These were raised by lenses but scored below the 80 confidence bar, so they are **not** part of the confirmed findings. They are recorded because two of them are real behavioural risks worth a runtime check before merge.

- **C2 (confidence 50, minor)** — `stop_pending` is consumed only from the per-window `on_frame_drawn` callback (`app/src/lib.rs:1477` → `app/src/dsh/runtime.rs:984`), which is driven by the platform frame callback (`crates/warpui_core/src/core/app.rs:2490`). With no frame drawn (all windows minimized/occluded, or zero windows while the app lives on), a registered stop is postponed indefinitely. The existing forced-redraw fallback (`app/src/lib.rs:1524-1530`) keys only off `dsh::bridge::has_pending_events()` and does not cover `stop_pending`. App exit still stops the child.
- **C3 (confidence 55, minor)** — `any_dsh_pane` (`app/src/dsh/runtime.rs:1055`) iterates `ctx.window_ids()`, i.e. `AppContext.windows.keys()` (`crates/warpui_core/src/core/app.rs:4549`). `handle_window_closed()` eagerly removes the window entry while its views stay alive inside `ClosedWindowData` (`crates/warpui_core/src/core/app.rs:3126-3128`), so a dsh pane inside a closed-but-still-undoable window is invisible to the liveness check. With dsh panes in two windows, a pending stop from the surviving window can then terminate the instance the undo-restored window still needs — the very "pane pointing at a dead service" outcome this change set is meant to remove.
- **Unverified (needs runtime confirmation)** — `poll_pending_stop` clears `stop_pending` unconditionally whenever it still sees a dsh pane (`app/src/dsh/runtime.rs:986-993`), and `PaneGroup::clean_up_panes` does not remove panes from `pane_contents` (`app/src/pane_group/mod.rs:5801-5806`). If the detached pane group's view has not yet been reclaimed by `remove_dropped_items` when the next `on_frame_drawn` runs, `any_dsh_pane` can still see the just-destroyed pane and cancel the pending stop **permanently** — which would make even the primary path (closing the last dsh tab) fail to stop the child. Confirming this requires observing `[dsh] mark_stop_pending` / `pending stop cancelled` / `pending stop confirmed` in `~/Library/Logs/zap.log` after closing the last dsh tab and *not* reopening it.
- Dropped as false positives or out-of-scope: `_ctx` prefix vs `_` in `app/src/dsh/pane.rs:739` (required trait method, parameter cannot be deleted; `_app` already exists at `pane.rs:735`); non-inline log args in `app/src/dsh/runtime.rs:974` (`uninlined_format_args` does not apply to an expression argument, and the adjacent pre-existing line is shaped identically); `data.pane_group.as_ref(ctx)` panic risk in `app/src/undo_close/stack.rs:344-351` (unreachable — `workspace.id() == workspace_id` short-circuits and dead windows have distinct workspace ids); the `tab_index` clamp being applied only at the call site (`app/src/workspace/view.rs:19354`) while the unmodified `restore_closed_tab` does not clamp (pre-existing, unchanged line); the per-frame cost of `any_dsh_pane` (measured as negligible, µs-scale, and the performance lens reported no findings).
