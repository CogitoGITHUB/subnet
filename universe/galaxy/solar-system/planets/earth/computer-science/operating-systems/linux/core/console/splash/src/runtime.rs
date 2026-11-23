use std::collections::HashMap;
use std::io::{self, Stdout, stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend, TestBackend},
    buffer::Buffer,
    crossterm::{
        cursor::{Hide, Show},
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Alignment, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheBackend, CacheEntry, CacheEntryKind, MemCache};
use crate::config::{Config, General, WidgetConfig};
use crate::daemon;
use crate::fetcher::{self, FetchContext, Registry, ShapeMismatch};
use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::{self, RenderSpec, Shape};
use crate::theme::Theme;
use crate::trust::{self, TrustStore};

/// Split trust-passed widgets into cached (disk-backed, fetched async via daemon) and realtime
/// (recomputed on every frame, no disk I/O). Keeps the rest of the runtime simple: cache-aware
/// code only sees cached widgets, per-frame code only sees realtime widgets.
fn split_by_fetcher_kind(
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    widgets
        .iter()
        .cloned()
        .partition(|w| registry.get_cached(&w.fetcher).is_some())
}

/// Derives the shape each widget should ask its fetcher for. Renderers are single-shape today
/// (every builtin's `accepts()` returns one element), so specifying `render = "..."` uniquely
/// determines the shape. Widgets without an explicit renderer fall back to the fetcher's
/// [`fetcher::Fetcher::default_shape`]. Unknown fetcher / renderer names drop out here — we skip
/// them and let the downstream dispatch / render paths surface the mismatch as placeholders.
pub fn derive_shapes(
    widgets: &[WidgetConfig],
    fetchers: &Registry,
    renderers: &render::Registry,
) -> HashMap<WidgetId, Shape> {
    widgets
        .iter()
        .filter_map(|w| derive_shape(w, fetchers, renderers).map(|s| (w.id.clone(), s)))
        .collect()
}

fn derive_shape(
    w: &WidgetConfig,
    fetchers: &Registry,
    renderers: &render::Registry,
) -> Option<Shape> {
    if let Some(spec) = &w.render
        && let Some(renderer) = renderers.get(spec.renderer_name())
    {
        let accepted = renderer.accepts();
        // Prefer the fetcher's own shape ordering intersected with what the renderer
        // accepts — fetchers list their most-natural shape first (e.g. `project` lists
        // `TextBlock` ahead of `Text`), and that preference should survive even when the
        // renderer accepts multiple shapes. Falls back to the renderer's first accept when
        // the fetcher is unknown or has no intersection, so the old "first accept wins"
        // behaviour still holds for placeholder / mismatch paths.
        if let Some(fetcher) = fetchers.get(&w.fetcher)
            && let Some(shape) = fetcher.shapes().into_iter().find(|s| accepted.contains(s))
        {
            return Some(shape);
        }
        if let Some(&shape) = accepted.first() {
            return Some(shape);
        }
    }
    fetchers.get(&w.fetcher).map(|f| f.default_shape())
}

/// Partitions widgets into (known-fetcher, unknown-fetcher). Unknown fetcher names — typo,
/// renamed widget, or a config newer than the installed binary — would otherwise drop silently
/// in the realtime/cached dispatch and leave the slot blank. Diverting them at the runtime
/// boundary mirrors the renderer side of the contract: an unknown name renders an in-band
/// `⚠ unknown fetcher: <name>` placeholder so the user can see what to fix.
fn partition_by_known_fetcher(
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    widgets
        .iter()
        .cloned()
        .partition(|w| registry.get(&w.fetcher).is_some())
}

/// Partitions widgets into (shape-valid, shape-invalid-with-reason). A widget whose renderer-
/// derived shape isn't in the fetcher's `shapes()` is diverted to a placeholder rather than
/// being dispatched — protects the same "never crash the splash" invariant as unknown-renderer
/// handling in [`render::render_payload`]. Unknown fetcher names are filtered out before this
/// runs, so the unknown-fetcher branch is a defensive no-op.
fn partition_by_shape_support(
    widgets: &[WidgetConfig],
    shapes: &HashMap<WidgetId, Shape>,
    fetchers: &Registry,
) -> (Vec<WidgetConfig>, Vec<(WidgetConfig, ShapeMismatch)>) {
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for w in widgets {
        let Some(fetcher) = fetchers.get(&w.fetcher) else {
            valid.push(w.clone());
            continue;
        };
        let Some(&shape) = shapes.get(&w.id) else {
            valid.push(w.clone());
            continue;
        };
        if fetcher.shapes().contains(&shape) {
            valid.push(w.clone());
        } else {
            invalid.push((
                w.clone(),
                ShapeMismatch {
                    fetcher: fetcher.name().to_string(),
                    requested: shape,
                },
            ));
        }
    }
    (valid, invalid)
}

fn compute_realtime_payloads(
    registry: &Registry,
    widgets: &[WidgetConfig],
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
) -> HashMap<WidgetId, Payload> {
    let mut out = HashMap::new();
    compute_realtime_payloads_into(registry, widgets, general, shapes, &mut out);
    out
}

/// Recomputes every realtime widget directly into `out`, overwriting existing keys. The
/// allocation-free variant `watch` uses on every realtime tick — building a fresh `HashMap`
/// every 200ms and `extend`-ing it into `payloads` was wasted work.
fn compute_realtime_payloads_into(
    registry: &Registry,
    widgets: &[WidgetConfig],
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
    out: &mut HashMap<WidgetId, Payload>,
) {
    for w in widgets {
        let Some(fetcher) = registry.get_realtime(&w.fetcher) else {
            continue;
        };
        let ctx = fetch_context(
            w,
            general,
            shapes.get(&w.id).copied(),
            Duration::from_secs(0),
        );
        out.insert(w.id.clone(), fetcher.compute(&ctx));
    }
}

/// Widgets whose data we're still waiting on. Two cases are both treated as "loading":
///
/// 1. No payload at all (first run, cleared cache, brand-new widget) — always loading.
/// 2. Under `--wait`, a stale cache entry whose TTL has expired — the daemon is on the
///    critical path and will refetch, so the displayed payload is about to change. Showing a
///    spinner telegraphs that intent instead of rendering last run's data as if it were
///    authoritative. Without `--wait` we prefer the stale payload to a spinner because the
///    refetch is best-effort (background daemon) and may not land in this invocation.
///
/// Recomputed each iteration so the spinner is swapped off the moment the daemon fills or
/// refreshes the slot. Shape is carried through so future per-shape skeletons can branch on
/// it in `render_loading`.
fn compute_loading(
    cached: &[WidgetConfig],
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    shapes: &HashMap<WidgetId, Shape>,
    wait: bool,
) -> HashMap<WidgetId, Shape> {
    cached
        .iter()
        .filter(|w| is_loading(w, payloads, entries, wait))
        .filter_map(|w| shapes.get(&w.id).copied().map(|s| (w.id.clone(), s)))
        .collect()
}

fn is_loading(
    w: &WidgetConfig,
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    wait: bool,
) -> bool {
    if !payloads.contains_key(&w.id) {
        return true;
    }
    wait && entries.get(&w.id).is_some_and(|e| !e.is_fresh())
}

fn has_loading_widget(
    cached: &[WidgetConfig],
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    wait: bool,
) -> bool {
    cached
        .iter()
        .any(|w| is_loading(w, payloads, entries, wait))
}

/// Loop exit condition factored out so the "wait for both load and animation axes to settle"
/// invariant can be tested without spinning up a real daemon. Matches the inline check in
/// `run()` exactly — hitting the hard deadline always exits, otherwise both the daemon
/// (load axis) and the animation (render axis) must be done.
fn loop_should_exit(daemon_done: bool, animation_pending: bool, deadline_reached: bool) -> bool {
    deadline_reached || (daemon_done && !animation_pending)
}

/// Whether the loop should still be holding for the animation window. `animation_due` is the
/// animation deadline check (`now < deadline`), `real_animated` is true when a configured
/// renderer genuinely wants to animate, `loading_pending` is true when at least one cached
/// widget still has no fresh payload. The animation window is only *binding* while something
/// needs to animate — loading spinners are the common case for minimal presets, and once the
/// daemon populates their cache entries the window stops blocking exit.
fn animation_still_due(animation_due: bool, real_animated: bool, loading_pending: bool) -> bool {
    animation_due && (real_animated || loading_pending)
}

const FAST_DEADLINE: Duration = Duration::from_millis(1500);
const WAIT_DEADLINE: Duration = Duration::from_secs(5);
const DAEMON_DEADLINE: Duration = Duration::from_secs(30);
/// Frame interval while inside the multi-frame render loop. Animated renderers rely on this
/// cadence to produce motion; sub-50ms would waste CPU, above 100ms would look choppy.
const FRAME_TICK: Duration = Duration::from_millis(50);
/// How long to run the render loop when the only reason for looping is animation (no wait).
/// Long enough for `animated_typewriter` to finish typing; short enough to not noticeably
/// delay the shell on cache-warm startups.
const ANIMATION_WINDOW: Duration = Duration::from_secs(2);
const DEFAULT_VIEWPORT_LINES: u16 = 16;

pub async fn run(
    config: &Config,
    source: &crate::config::DashboardSource,
    config_ident: Option<(&Path, &str)>,
    wait: bool,
) -> io::Result<()> {
    let layout = config.to_layout();
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let specs = render_specs(&config.widgets);
    let theme = Theme::from_style();
    let padding = config.general.padding.map(|p| p.xy()).unwrap_or((0, 0));

    // Split widgets into what we can fetch vs what we must gate behind trust. Gated slots render
    // a canned "🔒 requires trust" placeholder so the layout stays intact and the user can see
    // what needs unlocking.
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    // Unknown fetcher names get diverted here so the silent-drop sites downstream
    // (compute_realtime_payloads / fetch_all / load_entries) only ever see registered names.
    let (fetchable, unknown) = partition_by_known_fetcher(&fetchable, &registry);
    // Derive the renderer-implied shape for each widget once, then reuse it in every downstream
    // step (validation, cache key, fetch, realtime compute) so the fetcher sees a single
    // consistent answer.
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    // Shape validation runs on everything the trust gate let through — a widget whose renderer
    // demands a shape the fetcher can't emit renders a placeholder instead of reaching the
    // fetcher.
    let (fetchable, shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);

    // Cached widgets go through disk cache + async daemon. Realtime widgets recompute each
    // frame and never touch disk.
    let (cached_widgets, realtime_widgets) = split_by_fetcher_kind(&fetchable, &registry);

    let entries = load_entries(
        as_backend(&cache),
        &registry,
        &cached_widgets,
        &config.general,
        &shapes,
    );
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(
        &registry,
        &realtime_widgets,
        &config.general,
        &shapes,
    ));
    for w in &gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for w in &unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
    }
    for (w, err) in &shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }

    // Load axis: decide whether to block on fresh data. If there are any cached widgets with no
    // entry yet, promote to wait so we don't exit with a blank terminal on first run. Realtime
    // widgets don't count — they're always fresh.
    let wait =
        wait || config.general.wait_for_fresh || (!cached_widgets.is_empty() && entries.is_empty());

    // Render axis: decide whether any configured renderer needs repeat frames. Independent of
    // the load decision — an animated widget must animate regardless of cache state. Loading
    // placeholders also animate (spinner glyph) so we force-enable animation whenever a cached
    // widget has no entry yet — otherwise the spinner would appear to freeze mid-draw. We keep
    // the two reasons separate: `real_animated` forces the loop to run the full ANIMATION_WINDOW
    // so a configured effect plays; loading-only animation is dropped the moment the daemon's
    // fresh entries clear the last spinner (checked per iteration inside the loop).
    let real_animated = render::any_widget_animates(&config.widgets, &render_registry);
    let animated = real_animated || has_loading_widget(&cached_widgets, &payloads, &entries, wait);

    let loop_window = match (wait, animated) {
        (false, false) => None,
        (false, true) => Some(ANIMATION_WINDOW),
        (true, _) => Some(WAIT_DEADLINE),
    };

    // Default viewport = sum of configured row heights so every widget fits without manual
    // tuning. Users can still override with `general.height` when they want padding or are
    // using Percentage sizes. Clamped to `terminal_rows - 1` so there's always one row left
    // below the viewport for the shell prompt. When the configured height doesn't fit, the
    // bottom row of the visible viewport is sacrificed for a `… +N rows` clip hint so users
    // aren't silently missing widgets.
    let requested_height = config
        .general
        .height
        .unwrap_or_else(|| config.computed_height().max(DEFAULT_VIEWPORT_LINES));
    let term_rows = ratatui::crossterm::terminal::size()
        .map(|(_, h)| h)
        .unwrap_or(requested_height);
    let max_inline = term_rows.saturating_sub(1).max(1);
    let clipping = requested_height > max_inline;
    let viewport_lines = if clipping {
        max_inline
    } else {
        requested_height
    }
    .max(1);
    let hidden_rows = if clipping {
        requested_height.saturating_sub(viewport_lines.saturating_sub(1))
    } else {
        0
    };
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = make_terminal(backend, viewport_lines)?;

    // Cache-first one-shot: no looping needed, paint what we have and exit.
    if loop_window.is_none() {
        let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
        finalize_draw(
            &mut terminal,
            &layout,
            &payloads,
            &specs,
            &render_registry,
            &theme,
            &config.general,
            padding,
            requested_height,
            viewport_lines,
            &loading,
        )?;
        let _ = daemon::spawn_fetch_daemon(source);
        finalize_splash(&mut terminal);
        return Ok(());
    }

    let window = loop_window.unwrap();
    match daemon::spawn_fetch_daemon(source) {
        Ok(mut child) => {
            // Multi-frame loop: ticks the frame every FRAME_TICK so animated renderers can
            // animate; picks up daemon-written cache entries as they land; stops when the
            // daemon finishes (if we were waiting for it) or the window expires.
            let buckets = WidgetBuckets {
                cached: &cached_widgets,
                gated: &gated,
                unknown: &unknown,
                shape_invalid: &shape_invalid,
            };
            let start = std::time::Instant::now();
            let deadline = start + window;
            let animation_deadline = animated.then(|| start + ANIMATION_WINDOW);
            // `wait = true` means the daemon is on the critical path; flip this once it exits
            // so later iterations skip the `child.wait()` branch. The load-axis and render-axis
            // exit conditions are independent — break only when both are satisfied, or when the
            // hard window deadline expires.
            let mut daemon_done = !wait;
            loop {
                let entries = refresh_payloads(
                    as_backend(&cache),
                    &registry,
                    &buckets,
                    &config.general,
                    &shapes,
                    &mut payloads,
                );
                let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
                draw(
                    &mut terminal,
                    &layout,
                    &payloads,
                    &specs,
                    &render_registry,
                    &theme,
                    &config.general,
                    padding,
                    hidden_rows,
                    &loading,
                )?;
                let now = std::time::Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    if !daemon_done {
                        let _ = child.start_kill();
                    }
                    break;
                }
                // Hold the loop for the animation window only while there's something that
                // actually needs to animate: either a configured animated renderer or a
                // still-loading spinner. A minimal preset on a cold cache sets `animated = true`
                // purely because of spinners; once the daemon writes their entries the loading
                // set empties and we can exit as soon as the daemon signals done, instead of
                // burning the remainder of the 2s window for nothing.
                let animation_pending = animation_still_due(
                    animation_deadline.is_some_and(|d| now < d),
                    real_animated,
                    !loading.is_empty(),
                );
                if loop_should_exit(daemon_done, animation_pending, false) {
                    break;
                }
                let tick = remaining.min(FRAME_TICK);
                if !daemon_done {
                    if tokio::time::timeout(tick, child.wait()).await.is_ok() {
                        daemon_done = true;
                    }
                } else {
                    tokio::time::sleep(tick).await;
                }
            }
            let entries = refresh_payloads(
                as_backend(&cache),
                &registry,
                &buckets,
                &config.general,
                &shapes,
                &mut payloads,
            );
            let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
            finalize_draw(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                requested_height,
                viewport_lines,
                &loading,
            )?;
        }
        Err(_) => {
            // Spawning the daemon failed (OOM, exec permission) — rare. Fall back to inline
            // fetch and a single final draw so the user still sees something useful.
            let fetch_deadline = if wait { WAIT_DEADLINE } else { FAST_DEADLINE };
            let fresh = fetch_all(
                &registry,
                as_backend(&cache),
                &cached_widgets,
                &entries,
                &config.general,
                &shapes,
                fetch_deadline,
            )
            .await;
            for (id, payload) in fresh {
                payloads.insert(id, payload);
            }
            // Realtime values in the fallback path are still fresh at this point — the compute
            // above was microseconds ago. No refresh needed before the single final draw.
            let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
            finalize_draw(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                requested_height,
                viewport_lines,
                &loading,
            )?;
        }
    }

    finalize_splash(&mut terminal);
    Ok(())
}

/// Frame cadence for `watch` mode — the same 50ms as the splash animation loop. Drives input
/// polling and the redraw check; actual repaints are gated by [`watch_should_draw`].
const WATCH_FRAME_TICK: Duration = Duration::from_millis(50);
/// How often realtime fetchers are recomputed inside `watch`. Realtime fetchers run per-frame
/// in the one-shot splash, but a 50ms recompute loop is wasteful for a persistent dashboard —
/// 200ms keeps `clock` / `system_*` visibly live without a frame-rate fetch loop.
const WATCH_REALTIME_TICK: Duration = Duration::from_millis(200);
/// How often the in-process fetch loop checks cached widgets. Each pass only refetches widgets
/// whose cache entry is stale (see `should_refresh`), so this is a poll interval, not a fetch
/// interval — each fetcher's TTL sets the real cadence.
const WATCH_FETCH_TICK: Duration = Duration::from_secs(2);

/// Persistent dashboard mode. Unlike [`run`], which paints once and hands the terminal back to
/// the shell, `watch` holds the alternate screen and keeps the dashboard live: realtime
/// fetchers tick, cached fetchers refresh on their TTL via an in-process fetch loop, and the
/// screen repaints as data lands. Exits on `q` or Ctrl-C.
pub async fn watch(config: &Config, config_ident: Option<(&Path, &str)>) -> io::Result<()> {
    let layout = config.to_layout();
    // Disk is only touched at the boundaries: warm `mem_cache` from disk at the top, flush back
    // at the bottom. The hot path (foreground reads + fetch-task writes) lives entirely in
    // memory — the disk cache exists for the splash/daemon IPC and watch has no daemon.
    let disk_cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let specs = render_specs(&config.widgets);
    let theme = Theme::from_style();
    let padding = config.general.padding.map(|p| p.xy()).unwrap_or((0, 0));

    // Same widget partitioning as `run`: trust gate, unknown-fetcher divert, shape validation,
    // then split cached vs realtime.
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    let (fetchable, unknown) = partition_by_known_fetcher(&fetchable, &registry);
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    let (fetchable, shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);
    let (cached_widgets, realtime_widgets) = split_by_fetcher_kind(&fetchable, &registry);

    // Warm the in-memory cache from disk for the cached widgets we know about. Anything
    // missing on disk simply starts as `loading` and the fetch loop populates it.
    let cache_keys = cache_keys_for(&cached_widgets, &registry, &config.general, &shapes);
    let mem_cache = MemCache::warm_from(disk_cache.as_ref(), cache_keys.iter().map(String::as_str));

    let entries = load_entries(
        Some(&mem_cache),
        &registry,
        &cached_widgets,
        &config.general,
        &shapes,
    );
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(
        &registry,
        &realtime_widgets,
        &config.general,
        &shapes,
    ));
    for w in &gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for w in &unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
    }
    for (w, err) in &shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }

    // In-process replacement for the detached fetch daemon: re-runs the cached fetchers on a
    // loop so the foreground only ever reads from `mem_cache`. Owned clones because the task
    // outlives this stack frame; aborted on exit.
    let fetch_ident = config_ident.map(|(p, h)| (p.to_path_buf(), h.to_string()));
    let fetch_loop = tokio::spawn(watch_fetch_loop(
        config.clone(),
        fetch_ident,
        mem_cache.clone(),
    ));

    // Placeholders for gated / unknown / shape-invalid widgets are inserted into `payloads`
    // above and never change after startup in watch mode (the fetch loop excludes them by
    // construction), so the splash's per-frame re-application via `refresh_payloads` would be
    // wasted work here.
    let real_animated = render::any_widget_animates(&config.widgets, &render_registry);
    // The dashboard is drawn at its natural height, top-anchored on the alternate screen —
    // handing the layout the whole fullscreen area would stretch its rows down to fill the
    // terminal. Same height resolution as the inline splash in `run`.
    let requested_height = config
        .general
        .height
        .unwrap_or_else(|| config.computed_height().max(DEFAULT_VIEWPORT_LINES));

    install_watch_panic_hook();
    let mut terminal = WatchTerminal::enter()?;
    let start = Instant::now();
    let mut last_realtime = Instant::now();
    // Cached entries are carried across iterations and only re-pulled from `mem_cache` when its
    // generation advances. The footer's `next refresh in N` countdown reads from this every
    // frame, so we have to keep it around — but the per-frame work was the wasteful part.
    let mut cached_entries = entries;
    let mut cache_gen = mem_cache.generation();
    let mut prev_status: Option<WatchStatus> = None;

    // Loop body errors are funnelled through `break Err(...)` rather than `?` so the teardown
    // below (`fetch_loop.abort()` + `mem_cache.flush_to(disk)`) always runs. A transient
    // terminal I/O error should still flush, otherwise this session's fetches would be lost.
    let result: io::Result<()> = loop {
        // Drain pending input. `q` / Ctrl-C quit; a resize forces a repaint (ratatui picks up
        // the new size on the next `draw`).
        let mut resized = false;
        let mut quit = false;
        let drain: io::Result<()> = (|| {
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => quit |= is_quit_key(&k),
                    Event::Resize(_, _) => resized = true,
                    _ => {}
                }
            }
            Ok(())
        })();
        if let Err(e) = drain {
            break Err(e);
        }
        if quit {
            break Ok(());
        }

        let realtime_recomputed = last_realtime.elapsed() >= WATCH_REALTIME_TICK;
        if realtime_recomputed {
            compute_realtime_payloads_into(
                &registry,
                &realtime_widgets,
                &config.general,
                &shapes,
                &mut payloads,
            );
            last_realtime = Instant::now();
        }

        // Re-pull cached entries only when the fetch task has actually written something. In
        // steady state this branch is taken once every widget TTL, not once per frame.
        let cur_gen = mem_cache.generation();
        let cache_changed = cur_gen != cache_gen;
        if cache_changed {
            cached_entries = load_entries(
                Some(&mem_cache),
                &registry,
                &cached_widgets,
                &config.general,
                &shapes,
            );
            for (id, entry) in &cached_entries {
                payloads.insert(id.clone(), entry.payload.clone());
            }
            cache_gen = cur_gen;
        }

        let loading = compute_loading(&cached_widgets, &payloads, &cached_entries, &shapes, false);
        let status = WatchStatus {
            next_refresh_secs: next_refresh_secs(&cached_entries),
            loading: loading.len(),
        };

        let animating = real_animated && start.elapsed() < ANIMATION_WINDOW;
        let status_changed = prev_status != Some(status);
        if watch_should_draw(
            resized,
            cache_changed || realtime_recomputed || status_changed,
            animating,
            !loading.is_empty(),
        ) {
            if let Err(e) = draw_watch(
                &mut terminal.inner,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                requested_height,
                &loading,
                &status,
            ) {
                break Err(e);
            }
            prev_status = Some(status);
        }

        tokio::time::sleep(WATCH_FRAME_TICK).await;
    };

    fetch_loop.abort();
    drop(terminal);
    // Best-effort flush. Runs on every exit path that reaches here (`q`, Ctrl-C, panic
    // unwinding past `Drop`, or a terminal I/O error funneled through the loop's `break`).
    // SIGKILL is the only path that skips this, and the cost there is just a refetch next
    // session.
    if let Some(disk) = disk_cache.as_ref() {
        mem_cache.flush_to(disk);
    }
    result
}

/// In-process fetch loop for `watch`. Mirrors the detached `fetch-only` daemon: each pass runs
/// the cached fetchers and writes the cache, then sleeps. `fetch_and_persist` only refetches
/// widgets whose cache entry is stale, so widget TTLs — not `WATCH_FETCH_TICK` — set the real
/// refresh cadence.
async fn watch_fetch_loop(config: Config, ident: Option<(PathBuf, String)>, cache: MemCache) {
    loop {
        let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
        fetch_and_persist(&config, ident_ref, Some(&cache as &dyn CacheBackend)).await;
        tokio::time::sleep(WATCH_FETCH_TICK).await;
    }
}

/// Whether the `watch` loop should repaint this frame. A persistent dashboard on an idle laptop
/// should not redraw 20x/s for nothing, so a repaint happens only when there is a reason: the
/// terminal resized, a payload or loading-set changed, the intro animation is still playing, or
/// a widget is still showing its loading spinner.
fn watch_should_draw(resized: bool, changed: bool, animating: bool, loading_active: bool) -> bool {
    resized || changed || animating || loading_active
}

/// `q` or Ctrl-C exit `watch`. Ctrl-C arrives as a key event rather than SIGINT because raw
/// mode is enabled, so both quit paths are handled here.
fn is_quit_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q'))
        || (matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// RAII wrapper for the alternate-screen session `watch` runs in. `enter` enables raw mode and
/// switches to the alternate screen; `Drop` restores both, so the terminal is left sane whether
/// `watch` returns normally, bails on a `?`, or unwinds on a panic.
struct WatchTerminal {
    inner: Terminal<CrosstermBackend<Stdout>>,
}

impl WatchTerminal {
    fn enter() -> io::Result<Self> {
        // Stepwise so a failure after we've already mutated terminal state rolls each previous
        // step back. `Drop` only fires on a constructed `Self`, so a `?` on the final step
        // would otherwise leave the user in raw mode on the alternate screen with no recovery.
        enable_raw_mode()?;
        if let Err(e) = execute!(stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(e);
        }
        match Terminal::new(CrosstermBackend::new(stdout())) {
            Ok(inner) => Ok(Self { inner }),
            Err(e) => {
                let _ = execute!(stdout(), LeaveAlternateScreen, Show);
                let _ = disable_raw_mode();
                Err(e)
            }
        }
    }
}

impl Drop for WatchTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, Show);
    }
}

/// Restores the terminal from the panic hook so a panic inside the `watch` loop doesn't leave
/// the user in raw mode on the alternate screen. `WatchTerminal`'s `Drop` also restores during
/// unwind, but the default hook prints the panic message first — this runs before that so the
/// message lands on the normal screen.
fn install_watch_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, Show);
        original(info);
    }));
}

/// Runs fetchers and persists the results. Called by the detached `fetch-only` subcommand so the
/// main invocation can exit without waiting for I/O. Errors from individual fetchers are logged
/// nowhere (daemon has no stdio) and simply leave stale cache in place. The daemon re-checks
/// trust itself because it loads config via its own command-line argument, not from shared
/// state.
pub async fn fetch_and_persist(
    config: &Config,
    config_ident: Option<(&Path, &str)>,
    cache: Option<&dyn CacheBackend>,
) {
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, _gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    // Unknown fetcher names are surfaced by the main process as placeholders; the daemon has
    // nothing to fetch for them.
    let (fetchable, _unknown) = partition_by_known_fetcher(&fetchable, &registry);
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    // Shape-invalid widgets are rendered as placeholders by the main process; there's no value
    // in the daemon hitting their fetchers just to have the result discarded.
    let (fetchable, _shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);
    // Daemon only persists cached-fetcher output. Realtime widgets have no disk representation.
    let (cached_widgets, _realtime) = split_by_fetcher_kind(&fetchable, &registry);
    let entries = load_entries(cache, &registry, &cached_widgets, &config.general, &shapes);
    let _ = fetch_all(
        &registry,
        cache,
        &cached_widgets,
        &entries,
        &config.general,
        &shapes,
        DAEMON_DEADLINE,
    )
    .await;
}

/// Widget partitions the runtime carries across refreshes. Bundling keeps
/// [`refresh_payloads`] under clippy's arg-count limit and makes the "what goes where"
/// categories explicit at call sites.
struct WidgetBuckets<'a> {
    cached: &'a [WidgetConfig],
    gated: &'a [WidgetConfig],
    unknown: &'a [WidgetConfig],
    shape_invalid: &'a [(WidgetConfig, ShapeMismatch)],
}

/// Merges fresh daemon-written cache entries into the payload map between frames. Realtime
/// payloads stay exactly as they were computed once before the loop — the animation window is
/// short (2s) and recomputing per frame would make expensive fetchers like `system_processes`
/// scale badly. Trust-gate and shape-mismatch placeholders are re-applied defensively so a
/// daemon-written entry can't leak into a gated slot.
fn refresh_payloads(
    cache: Option<&dyn CacheBackend>,
    registry: &Registry,
    buckets: &WidgetBuckets<'_>,
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
    payloads: &mut HashMap<WidgetId, Payload>,
) -> HashMap<WidgetId, CacheEntry> {
    let entries = load_entries(cache, registry, buckets.cached, general, shapes);
    for (id, payload) in entries_to_payloads(&entries) {
        payloads.insert(id, payload);
    }
    for w in buckets.gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for w in buckets.unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
    }
    for (w, err) in buckets.shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }
    entries
}

fn fetch_context(
    w: &WidgetConfig,
    general: &General,
    shape: Option<Shape>,
    deadline: Duration,
) -> FetchContext {
    FetchContext {
        widget_id: w.id.clone(),
        format: w.format.clone(),
        timeout: deadline,
        file_format: w.file_format.clone(),
        shape,
        options: w.options.clone(),
        timezone: general.timezone.clone(),
        locale: general.locale.clone(),
    }
}

/// Coerces the owned `Option<Cache>` callers carry into the trait-object form the cache
/// plumbing accepts. One liner, but kept named so call sites don't repeat the cast.
fn as_backend(cache: &Option<Cache>) -> Option<&dyn CacheBackend> {
    cache.as_ref().map(|c| c as &dyn CacheBackend)
}

/// Derives the on-disk cache key for every cached widget. Used by `watch` to know which disk
/// entries to copy into its `MemCache` at start-up.
fn cache_keys_for(
    widgets: &[WidgetConfig],
    registry: &Registry,
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
) -> Vec<String> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_cached(&w.fetcher)?;
            let ctx = fetch_context(
                w,
                general,
                shapes.get(&w.id).copied(),
                Duration::from_secs(0),
            );
            Some(fetcher.cache_key(&ctx))
        })
        .collect()
}

fn load_entries(
    cache: Option<&dyn CacheBackend>,
    registry: &Registry,
    widgets: &[WidgetConfig],
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
) -> HashMap<WidgetId, CacheEntry> {
    let Some(c) = cache else {
        return HashMap::new();
    };
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_cached(&w.fetcher)?;
            let shape = shapes.get(&w.id).copied();
            let key = fetcher.cache_key(&fetch_context(w, general, shape, Duration::from_secs(0)));
            c.load(&key).map(|e| (w.id.clone(), e))
        })
        .collect()
}

fn entries_to_payloads(entries: &HashMap<WidgetId, CacheEntry>) -> HashMap<WidgetId, Payload> {
    entries
        .iter()
        .map(|(k, v)| (k.clone(), v.payload.clone()))
        .collect()
}

/// Short TTL for error/timeout cache entries so a transient blip doesn't pin the widget on the
/// warning placeholder for long. Set well below typical success-path refresh intervals so the
/// next tick retries; long enough that back-to-back renders don't re-run a fetch that just failed.
const ERROR_CACHE_TTL_SECS: u64 = 30;

/// TTL for a cached widget's successful payload: the per-widget config override wins, otherwise
/// the fetcher's declared interval. There is no global fallback — every cached fetcher declares
/// its own [`fetcher::Fetcher::refresh_interval`].
fn resolve_refresh_interval(w: &WidgetConfig, fetcher: &dyn fetcher::Fetcher) -> u64 {
    w.refresh_interval
        .unwrap_or_else(|| fetcher.refresh_interval())
}

async fn fetch_all(
    registry: &Registry,
    cache: Option<&dyn CacheBackend>,
    widgets: &[WidgetConfig],
    cached: &HashMap<WidgetId, CacheEntry>,
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
    deadline: Duration,
) -> HashMap<WidgetId, Payload> {
    let mut set = JoinSet::new();
    for w in widgets {
        if !should_refresh(cached, w) {
            continue;
        }
        let Some(fetcher) = registry.get_cached(&w.fetcher) else {
            continue;
        };
        let ctx = fetch_context(w, general, shapes.get(&w.id).copied(), deadline);
        let cache_key = fetcher.cache_key(&ctx);
        let lock = match cache {
            Some(c) => match c.try_lock(&cache_key) {
                Some(l) => Some(l),
                None => continue,
            },
            None => None,
        };
        let id = w.id.clone();
        let ttl = resolve_refresh_interval(w, fetcher.as_ref());
        let fetcher_name = fetcher.name().to_string();
        set.spawn(async move {
            let _lock = lock;
            match tokio::time::timeout(deadline, fetcher.fetch(&ctx)).await {
                Ok(Ok(payload)) => (id, cache_key, ttl, CacheEntryKind::Ok, payload),
                Ok(Err(e)) => {
                    tracing::error!(
                        widget = %id,
                        fetcher = %fetcher_name,
                        cache_key = %cache_key,
                        error = %e,
                        "fetch failed"
                    );
                    (
                        id,
                        cache_key,
                        ERROR_CACHE_TTL_SECS,
                        CacheEntryKind::Err,
                        fetcher::error_placeholder(&e.to_string()),
                    )
                }
                Err(_) => {
                    tracing::warn!(
                        widget = %id,
                        fetcher = %fetcher_name,
                        cache_key = %cache_key,
                        timeout_ms = deadline.as_millis() as u64,
                        "fetch timed out"
                    );
                    (
                        id,
                        cache_key,
                        ERROR_CACHE_TTL_SECS,
                        CacheEntryKind::Timeout,
                        fetcher::timeout_placeholder(),
                    )
                }
            }
        });
    }

    let mut out = HashMap::new();
    while let Some(joined) = set.join_next().await {
        let Ok((id, key, ttl, kind, payload)) = joined else {
            continue;
        };
        if let Some(c) = cache {
            let _ = c.store(&key, &CacheEntry::new_with_kind(payload.clone(), ttl, kind));
        }
        out.insert(id, payload);
    }
    out
}

fn should_refresh(cached: &HashMap<WidgetId, CacheEntry>, w: &WidgetConfig) -> bool {
    match cached.get(&w.id) {
        Some(e) => !e.is_fresh(),
        None => true,
    }
}

fn make_terminal<B: Backend>(backend: B, viewport_lines: u16) -> Result<Terminal<B>, B::Error> {
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_lines),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    terminal.draw(|frame| {
        draw_frame(
            frame,
            frame.area(),
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            padding,
            hidden_rows,
            loading,
        )
    })?;
    Ok(())
}

/// Draw used by `watch`. The alternate screen is fullscreen, but the dashboard is drawn at its
/// natural `requested_height`, top-anchored — `draw_frame` distributes the layout's rows across
/// whatever area it's handed, so a fullscreen area would stretch the dashboard down to fill the
/// terminal. The whole screen is painted with the theme bg first so the band below the
/// dashboard isn't a bare terminal-coloured strip, and the bottom row is reserved for the
/// key-binding footer.
#[allow(clippy::too_many_arguments)]
fn draw_watch<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    requested_height: u16,
    loading: &HashMap<WidgetId, Shape>,
    status: &WatchStatus,
) -> Result<(), B::Error> {
    terminal.draw(|frame| {
        let full = frame.area();
        paint_viewport_bg(frame, full, theme);
        let footer_height = u16::from(full.height >= 2);
        let body = Rect {
            height: requested_height.min(full.height - footer_height),
            ..full
        };
        draw_frame(
            frame, body, root, payloads, specs, registry, theme, general, padding, 0, loading,
        );
        if footer_height > 0 {
            let footer = Rect {
                y: full.y + full.height - 1,
                height: 1,
                ..full
            };
            render_watch_footer(frame, footer, status, theme);
        }
    })?;
    Ok(())
}

/// Footer status shown by `watch`. The loop tracks the last-drawn copy of this so the
/// countdown still ticks when widget data is otherwise idle. `Copy` so change detection is a
/// trivial value comparison, no clone needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WatchStatus {
    /// Seconds until the soonest cached widget goes stale — when `watch` next has new data.
    /// `None` for a realtime-only dashboard (no cached widgets).
    next_refresh_secs: Option<u64>,
    /// Cached widgets still waiting on their first payload.
    loading: usize,
}

/// Seconds until the soonest cached widget goes stale. `None` when there are no cached widgets.
fn next_refresh_secs(entries: &HashMap<WidgetId, CacheEntry>) -> Option<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    entries
        .values()
        .map(|e| {
            e.refreshed_at
                .saturating_add(e.ttl_seconds)
                .saturating_sub(now)
        })
        .min()
}

/// The left-hand meta segment of the watch footer: what the dashboard is doing right now.
fn watch_footer_meta(status: &WatchStatus) -> String {
    if status.loading > 0 {
        let plural = if status.loading == 1 { "" } else { "s" };
        return format!("loading {} widget{plural}…", status.loading);
    }
    match status.next_refresh_secs {
        None => "realtime".to_string(),
        Some(0) => "refreshing…".to_string(),
        Some(secs) => format!("next refresh in {}", fmt_duration_compact(secs)),
    }
}

/// Compact duration for the footer: `45s`, `3m`, `2h`. Status-bar granularity, not precise.
fn fmt_duration_compact(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

/// One-line status footer pinned to the bottom of the `watch` alternate screen: live meta info
/// on the left, key bindings on the right, on the theme's subtle surface so it reads as chrome
/// separate from the dashboard band.
fn render_watch_footer(frame: &mut Frame, area: Rect, status: &WatchStatus, theme: &Theme) {
    let bg = theme.bg_subtle;
    let label = Style::default().bg(bg).fg(theme.text_secondary);
    let key = Style::default()
        .bg(bg)
        .fg(theme.text)
        .add_modifier(Modifier::BOLD);

    frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" ⟳ {}", watch_footer_meta(status)),
            label,
        ))),
        area,
    );
    let controls = Line::from(vec![
        Span::styled("q", key),
        Span::styled(" / ", label),
        Span::styled("Ctrl-C", key),
        Span::styled(" quit ", label),
    ]);
    frame.render_widget(Paragraph::new(controls).alignment(Alignment::Right), area);
}

/// Same as [`draw`] but parks the cursor at the bottom-left of the inline area afterwards, so
/// the caller's `println!()` lands one row below the splash instead of inside it. Moves via
/// the backend rather than `Frame::set_cursor_position` — the latter re-shows the cursor and
/// we want it to stay hidden (matches ratatui's default for every intermediate draw). The
/// caller is responsible for re-showing the cursor just before returning to the shell.
#[allow(clippy::too_many_arguments)]
fn draw_final<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let mut captured: Option<Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        draw_frame(
            frame,
            area,
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            padding,
            hidden_rows,
            loading,
        );
    })?;
    if let Some(area) = captured
        && area.height > 0
    {
        terminal.set_cursor_position(ratatui::layout::Position {
            x: area.x,
            y: area.y + area.height - 1,
        })?;
    }
    Ok(())
}

/// End-of-run draw. By default, when the configured height doesn't fit the terminal, the
/// overflow rows are dropped and the bottom of the viewport carries a `… +N rows` clip hint
/// — same behaviour as the animation-loop frames. With `general.flush_to_scrollback = true`,
/// the top rows are pushed into shell scrollback via [`Terminal::insert_before`] and the
/// viewport repaints with the bottom slice (full splash visible by scrolling up).
///
/// Flush is off by default because ratatui's `insert_before` write loop doesn't honour
/// `Cell::skip`, which `ratatui-image`'s stateful protocols (kitty / iTerm2 / sixel) rely on
/// to keep cells around the image untouched. The flush path overdraws those cells with
/// spaces and visibly chews up the image.
#[allow(clippy::too_many_arguments)]
fn finalize_draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    requested_height: u16,
    viewport_lines: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let scrollback_rows = requested_height.saturating_sub(viewport_lines);
    if scrollback_rows == 0 {
        return draw_final(
            terminal, root, payloads, specs, registry, theme, general, padding, 0, loading,
        );
    }
    if !general.flush_to_scrollback {
        return draw_final(
            terminal,
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            padding,
            scrollback_rows + 1,
            loading,
        );
    }
    flush_full_to_scrollback(
        terminal,
        root,
        payloads,
        specs,
        registry,
        theme,
        general,
        padding,
        requested_height,
        viewport_lines,
        loading,
    )
}

#[allow(clippy::too_many_arguments)]
fn flush_full_to_scrollback<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    requested_height: u16,
    viewport_lines: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let width = terminal.size()?.width;
    let full_buf = render_full_into_buffer(
        width,
        requested_height,
        root,
        payloads,
        specs,
        registry,
        theme,
        general,
        padding,
        loading,
    );
    let scrollback_rows = requested_height - viewport_lines;
    terminal.insert_before(scrollback_rows, |buf| {
        copy_rows(&full_buf, 0..scrollback_rows, buf);
    })?;
    let mut captured: Option<Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        copy_rows(
            &full_buf,
            scrollback_rows..requested_height,
            frame.buffer_mut(),
        );
    })?;
    if let Some(area) = captured
        && area.height > 0
    {
        terminal.set_cursor_position(Position {
            x: area.x,
            y: area.y + area.height - 1,
        })?;
    }
    Ok(())
}

/// Renders the layout into an off-screen buffer at full configured height. Uses `TestBackend`
/// only because it's ratatui's one publicly exposed in-memory `Backend` — the name is
/// historical, nothing about it is test-only.
#[allow(clippy::too_many_arguments)]
fn render_full_into_buffer(
    width: u16,
    height: u16,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    loading: &HashMap<WidgetId, Shape>,
) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut inner = Terminal::new(backend).expect("TestBackend::new is infallible");
    inner
        .draw(|frame| {
            // `TestBackend` stores cell symbols verbatim and never interprets escape
            // sequences, so the OSC 8 wrap that `list_links` adds for clickable hyperlinks
            // would only pollute this off-screen buffer (the diff drops the wrapped row's
            // skip cells, leaving them empty in the captured scrollback). Suppress the wrap
            // for this draw — the user-facing on-screen frame still goes through
            // `CrosstermBackend` without suppression and keeps the clickable links.
            render::list_links::without_osc8(|| {
                let area = frame.area();
                paint_viewport_bg(frame, area, theme);
                let content = shrink(area, padding);
                layout::draw(
                    frame, content, root, payloads, specs, registry, theme, general, loading,
                );
            });
        })
        .expect("TestBackend draw is infallible");
    inner.backend().buffer().clone()
}

/// Copies a vertical slice from `src` (rows `src_y_range`, taken relative to `src.area.top()`)
/// into `dst`, laid out from `dst.area.top()` onward. Widths are clamped to the narrower of the
/// two buffers. Each destination row uses the same column origin as `dst.area.left()` so it
/// works for both an `insert_before` buffer (origin `(viewport_x, 0)`) and a frame buffer
/// (origin `(viewport_x, viewport_y)`).
fn copy_rows(src: &Buffer, src_y_range: std::ops::Range<u16>, dst: &mut Buffer) {
    let src_area = src.area;
    let dst_area = dst.area;
    let copy_width = src_area.width.min(dst_area.width);
    for (i, offset) in src_y_range.enumerate() {
        let src_y = src_area.top() + offset;
        let dst_y = dst_area.top() + i as u16;
        if dst_y >= dst_area.bottom() {
            break;
        }
        for x in 0..copy_width {
            let src_pos = Position::new(src_area.left() + x, src_y);
            let dst_pos = Position::new(dst_area.left() + x, dst_y);
            if let (Some(src_cell), Some(dst_cell)) = (src.cell(src_pos), dst.cell_mut(dst_pos)) {
                *dst_cell = src_cell.clone();
            }
        }
    }
}

/// Renders the layout into the given area, sacrificing the bottom row for a `… +N rows` hint
/// when the configured height doesn't fit and the viewport had to be clamped to the terminal.
#[allow(clippy::too_many_arguments)]
fn draw_frame(
    frame: &mut Frame,
    area: Rect,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) {
    // Paint the full area first so the padded band takes the theme bg too — otherwise the
    // terminal bg would show through the padding ring and fight the theme surface.
    paint_viewport_bg(frame, area, theme);
    let content = shrink(area, padding);
    if hidden_rows > 0 && content.height > 1 {
        let layout_area = Rect {
            height: content.height - 1,
            ..content
        };
        let hint_area = Rect {
            y: content.y + content.height - 1,
            height: 1,
            ..content
        };
        layout::draw(
            frame,
            layout_area,
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            loading,
        );
        render_clip_hint(frame, hint_area, hidden_rows, theme);
    } else {
        layout::draw(
            frame, content, root, payloads, specs, registry, theme, general, loading,
        );
    }
}

/// Shrinks `area` by `padding` on each axis, clamping so padding larger than the half-extent
/// doesn't produce a negative-sized rect.
fn shrink(area: Rect, (x, y): (u16, u16)) -> Rect {
    let dx = x.min(area.width / 2);
    let dy = y.min(area.height / 2);
    Rect {
        x: area.x + dx,
        y: area.y + dy,
        width: area.width.saturating_sub(dx.saturating_mul(2)),
        height: area.height.saturating_sub(dy.saturating_mul(2)),
    }
}

/// Paints `theme.bg` across the whole splash area before any widget draws, so themes that
/// ship a bg (Tokyo Night, Dracula, etc.) get a solid painted surface instead of inheriting
/// the terminal's own background. `Color::Reset` is a no-op — cells stay terminal-default.
fn paint_viewport_bg(frame: &mut Frame, area: Rect, theme: &Theme) {
    if theme.bg == Color::Reset {
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);
}

fn render_clip_hint(frame: &mut Frame, area: Rect, hidden_rows: u16, theme: &Theme) {
    let text = format!("… +{hidden_rows} rows (terminal too short)");
    let style = Style::default()
        .fg(theme.text_dim)
        .add_modifier(Modifier::ITALIC);
    let p = Paragraph::new(Line::from(text).style(style)).alignment(Alignment::Right);
    frame.render_widget(p, area);
}

/// Hand back to the shell: show the cursor (ratatui hid it during drawing) and emit the
/// trailing newline that moves the cursor past the inline viewport.
fn finalize_splash<B: Backend>(terminal: &mut Terminal<B>) {
    let _ = terminal.show_cursor();
    println!();
}

fn render_specs(widgets: &[WidgetConfig]) -> HashMap<WidgetId, RenderSpec> {
    widgets
        .iter()
        .filter_map(|w| w.render.clone().map(|s| (w.id.clone(), s)))
        .collect()
}


pub fn dump(
    config: &Config,
    source: &crate::config::DashboardSource,
    config_ident: Option<(&Path, &str)>,
    width: u16,
) -> io::Result<String> {
    let layout = config.to_layout();
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let specs = render_specs(&config.widgets);
    let theme = Theme::from_style();
    let padding =
        config.general.padding.map(|p| p.xy()).unwrap_or((0, 0));
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, gated) = trust::partition_by_trust(
        &config.widgets,
        &registry,
        decision,
    );
    let (fetchable, unknown) =
        partition_by_known_fetcher(&fetchable, &registry);
    let shapes = derive_shapes(
        &config.widgets,
        &registry,
        &render_registry,
    );
    let (fetchable, shape_invalid) =
        partition_by_shape_support(&fetchable, &shapes, &registry);
    let (cached, realtime) =
        split_by_fetcher_kind(&fetchable, &registry);
    let entries = load_entries(
        as_backend(&cache),
        &registry,
        &cached,
        &config.general,
        &shapes,
    );
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(
        &registry,
        &realtime,
        &config.general,
        &shapes,
    ));
    for w in &gated {
        payloads.insert(
            w.id.clone(),
            trust::requires_trust_placeholder(),
        );
    }
    for w in &unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
    }
    for (w, err) in &shape_invalid {
        payloads.insert(
            w.id.clone(),
            fetcher::shape_mismatch_placeholder(err),
        );
    }
    let loading =
        compute_loading(&cached, &payloads, &entries, &shapes, false);
    let height = config.general.height.unwrap_or_else(|| {
        config.computed_height().max(DEFAULT_VIEWPORT_LINES)
    });
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal =
        ratatui::Terminal::new(backend).map_err(io::Error::other)?;
    terminal
        .draw(|frame| {
            draw_frame(
                frame,
                frame.area(),
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                0,
                &loading,
            )
        })
        .map_err(io::Error::other)?;
    let buf = terminal.backend().buffer().clone();
    let mut rows: Vec<String> = Vec::new();
    for y in 0..height {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buf[(x, y)].symbol());
        }
        rows.push(row.trim_end().to_string());
    }
    let _ = daemon::spawn_fetch_daemon(source);
    Ok(rows.join("\n"))
}
