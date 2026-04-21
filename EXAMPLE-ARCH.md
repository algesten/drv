# Architecture: Query-Driven Applications with `drv`

This document describes how to structure an interactive application using
query-driven architecture powered by `drv`. The running example is a mobile
teleconferencing app, but the patterns apply to any stateful application
with derived views and asynchronous I/O.

The goal: an assistant (human or AI) should be able to read this document
together with the `drv` crate documentation and structure a new application
from scratch.

## Table of contents

1. [Core idea](#core-idea)
2. [Query-driven vs reactive](#query-driven-vs-reactive)
3. [Sources: two kinds of ground truth](#sources-two-kinds-of-ground-truth)
4. [Drivers: the sync/async split](#drivers-the-syncasync-split)
5. [Queries: desired state, not transitions](#queries-desired-state-not-transitions)
6. [Actions as diffs](#actions-as-diffs)
7. [The main loop](#the-main-loop)
8. [Organizing the code: crate layout](#organizing-the-code-crate-layout)
9. [Crossing the ABI: foreign-owned buffers](#crossing-the-abi-foreign-owned-buffers)
10. [Bridging to a reactive UI framework (SwiftUI)](#bridging-to-a-reactive-ui-framework-swiftui)
11. [Threading model](#threading-model)
12. [Worked example: microphone lifecycle](#worked-example-microphone-lifecycle)
13. [Worked example: remote participants](#worked-example-remote-participants)
14. [Testing](#testing)
15. [Guidelines](#guidelines)

---

## Core idea

The application is structured as a **database of ground truth** (sources) and
**cached queries** over that database (memos). Nothing watches anything.
Nothing subscribes to anything. The main loop sets inputs, asks questions,
and acts on answers. Memoization makes the asking cheap.

```
set inputs → ask questions → act on answers → repeat
```

This is the same model used by rust-analyzer (via Salsa) and incremental
build systems. `drv` provides the memoization layer; this document describes
the application architecture built on top of it.

---

## Query-driven vs reactive

In a **reactive** architecture, state changes push notifications to observers.
You wire up handlers: "when X changes, do Y." The full behavior of the system
is the sum of all handlers — distributed across the codebase, implicitly
ordered, and easy to get wrong.

In a **query-driven** architecture, consumers pull query results. You write
functions: "given the current state of everything, what should be true?" The
system caches results and only recomputes when inputs change. The full
behavior of any query is in one function — centralized, explicit, testable.

### Concrete differences

| Concern | Reactive | Query-driven |
|---------|----------|-------------|
| "What happens when the user selects a mic?" | Find all observers of `selected_mic`, trace their effects | Read `desired_mic()` — it's a pure function |
| Mic disconnects while selected | Write an on-disconnect handler that checks intent and re-opens | `desired_mic()` still returns `Some(mic1)` — reconnection is emergent |
| Two inputs change on the same tick | Observer ordering matters; need batching to avoid glitches | Set both inputs, then query. Always consistent. |
| View removed | Must unsubscribe observers or leak memory | Nobody calls the query. No cost. |
| Testing | Set up reactive runtime, trigger change, observe effects | Call a pure function with known inputs, assert output |

### The key insight

Reactive: "when X changes, do Y." Correct when you've written handlers for
every possible transition. N states = O(N^2) possible transitions.

Query-driven: "given everything, what should be true?" Correct by
construction. N states = O(N) rules.

---

## Sources: two kinds of ground truth

Not all ground truth is the same. There are two distinct categories:

### External facts

Discovered, not chosen. Changes come from outside the application.

- Available microphones (OS hotplug events)
- Remote participants (network/WebRTC)
- Network connectivity state
- Permission grants from the OS

### User decisions

Chosen, not discovered. Changes come from user interaction.

- Which microphone is selected
- Which participant is muted/pinned
- Whether the camera is on
- UI preferences (layout, theme)

These have different lifecycles, different update paths, and different owners.
They should live in **separate sources**.

```rust
// External fact — managed by the device enumeration driver
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicInventory {
    pub permission: MicPermission,     // NotAsked, Pending, Granted, Denied
    pub mics: imbl::Vector<MicDevice>, // empty until Granted
}

// User decision — managed by UI event handlers
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicPreference {
    pub selected: Option<MicId>,
}
```

Sources are plain Rust structs. `drv` doesn't wrap them. Memoization lives
entirely in per-memo thread-local caches keyed by input value, so drivers
and the main loop simply pass `&mut MySource` around and mutate fields
directly.

Why separate? The device driver should not need to know about user
preferences. The UI handler should not need to know about OS enumeration.
Memos bridge them — each memo takes one or more **inputs** (projections
declared with `#[drv::input]`):

```rust
#[drv::memo(single)]
fn mic_picker<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> MicPickerModel {
    // inputs from two sources — only the fields this memo reads
}
```

### Organizing sources by domain

Group sources into domains. Each domain covers one area of the application:

```
Domain: Devices
  MicInventory      (external: what mics/cameras exist)
  MicPreference     (user: which mic is selected)
  CameraInventory   (external: what cameras exist)
  CameraPreference  (user: which camera is selected)

Domain: Call
  CallSession       (external: room state from server)
  LocalMedia        (user: camera on/off, mic on/off)

Domain: Participants
  RemoteParticipants  (external: who is in the call)
  ParticipantSettings (user: per-participant mute, pin)

Domain: Media
  MicCapture        (external: local mic stream state)
  CameraCapture     (external: local camera stream state)
  RemoteStreams      (external: incoming audio/video streams)
```

Drivers manage external-fact sources. UI handlers manage user-decision sources.
Memos reach across domains freely via multi-input parameters.

### User-decision sources keyed by external facts

The temptation to merge user decisions and external facts is strongest when
the user decision is a map or set keyed by identifiers the driver discovered:
muted participants keyed by ids the signaling driver reported, expanded
directories keyed by paths the FS driver listed, pinned items referencing
entries the inventory contains.

Resist. The user decision is the **key set**; the external fact is the **map
values**. They have different lifecycles (user choices persist across
reconnects; enumeration rebuilds) and different owners (dispatch mutates
decisions; the driver mutates facts). Keep them in separate sources; memos
that read both produce the combined view.

```rust
// External fact — driver-owned
pub struct Inventory {
    pub mics: imbl::Vector<MicDevice>,
}

// User decision — dispatch-owned, keyed by MicId the driver discovered
pub struct MicPreference {
    pub favorites: imbl::HashSet<MicId>,
    pub do_not_use: imbl::HashSet<MicId>,
}
```

The key set and the map values share an identifier type. That's not coupling
— it's reference. Neither source needs to know about the other; a memo reads
both and returns the combined shape.

### Shadow sources

Sometimes a user-decision source holds a *mutation* of an external-fact
source rather than an independent choice: an editor's edited buffer shadows
the pristine file on disk; an in-progress form shadows a server-side record;
a local draft shadows the synced document.

Two sources, shared key (path, record id, document id), one seeded from the
other. Document the seeding protocol explicitly, because "seeded" is where
the bugs hide:

- **Seed on first completion.** When the external source first becomes
  `Ready` for a key, ingest copies it into the user source.
- **Subsequent external updates don't auto-apply.** If the disk file changes
  again after the user has started editing, the new version is queued as a
  reload candidate, not silently dropped into the user source.
- **Writes are explicit.** The user source flows *out* to the external
  source only via a deliberate save action, with explicit conflict handling.

When both fields share the same underlying type, field names won't stop
a confused assignment during a refactor. Newtype the roles so the
compiler carries the invariant:

```rust
pub struct Synced(Arc<String>);
pub struct Local(Arc<String>);

pub struct Draft {
    pub synced: Synced,
    pub local:  Local,
}
```

---

## Drivers: the sync/async split

A driver manages one external-fact source. Every driver has two sides:

### The synchronous side: the source

The source is the queryable truth. It lives on the main thread (or whichever
thread owns the sources). It is always available for reading via inputs.
Queries never block.

### The asynchronous side: platform I/O

Hardware access, network calls, OS dialogs — these run on background threads
or async tasks. When they complete, they post results back to the main thread,
which writes them into the source.

```
Background threads                        Main thread
(expensive I/O)                           (cheap: sources + queries)

mic_driver ──────────┐
camera_driver ───────┤  source updates
signaling_driver ────┼──────────────────→  process_updates(&mut sources)
webrtc_driver ───────┤
hotplug_listener ────┘
```

### Driver structure

```rust
/// Manages the MicInventory source.
pub struct MicEnumerationDriver {
    /// Receives updates from the async enumeration task.
    rx: mpsc::Receiver<MicEvent>,
    /// Handle to request permission, start/stop enumeration, etc.
    platform: PlatformMicEnumerator,
}

impl MicEnumerationDriver {
    /// Called on the main thread each tick. Drains pending events
    /// into the source. Cheap — just field assignments.
    pub fn process(&self, inv: &mut MicInventory) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                MicEvent::PermissionGranted => {
                    inv.permission = MicPermission::Granted;
                }
                MicEvent::PermissionDenied => {
                    inv.permission = MicPermission::Denied;
                }
                MicEvent::DeviceAdded(mic) => {
                    inv.mics.push_back(mic);
                }
                MicEvent::DeviceRemoved(id) => {
                    inv.mics.retain(|m| m.id != id);
                }
            }
        }
    }

    /// Initiate a permission request. The result arrives via rx.
    /// Must write Pending synchronously — without it, the next tick's
    /// query still sees NotAsked and would request permission again.
    pub fn request_permission(&self, inv: &mut MicInventory) {
        inv.permission = MicPermission::Pending;
        self.platform.request_permission_async();
    }
}
```

### The execute pattern

Some drivers also need to act on query results. For example, the mic stream
driver opens/closes streams based on the diff between desired and actual
state. The execute method does two things atomically:

1. Writes the intent into the source (sync — prevents re-triggering)
2. Starts the async work (async — completes later)

```rust
pub struct MicStreamDriver {
    platform: PlatformAudioCapture,
}

impl MicStreamDriver {
    pub fn execute(&self, action: MicAction, cap: &mut MicCapture) {
        match action {
            MicAction::Open(id) => {
                // 1. Sync: update source immediately
                cap.state = CaptureState::Opening;
                cap.device_id = Some(id);
                // 2. Async: start the work
                self.platform.open_mic_async(id);
            }
            MicAction::Close => {
                cap.state = CaptureState::Closing;
                cap.device_id = None;
                self.platform.close_mic_async();
            }
            MicAction::Noop => {}
        }
    }
}
```

The synchronous write into the source is critical. Without it, the next query
would return the same action again (because the source hasn't changed yet),
causing a double-open. The source write closes the loop: execute → source
updates → query returns Noop → no re-execution.

### Pure output drivers

Some drivers have no external-fact source to maintain — they exist only to
push computed state out to a platform: paint a terminal frame, render a
graphics surface, play a sound buffer, flash an LED. No hotplug, no
incoming events; the "what should be true" is a memo in the runtime.

These still follow the execute pattern. The driver's source is the
**in-flight artifact** plus acknowledgement state:

```rust
pub struct PaintState {
    pub in_flight: Option<FrameId>,
    pub last_acked: Option<FrameId>,
}

impl TerminalPaintDriver {
    pub fn execute(&self, frame: Frame, state: &mut PaintState) {
        if state.in_flight == Some(frame.id) { return; }
        state.in_flight = Some(frame.id);      // sync intent write
        self.platform.paint_async(frame);      // async work
    }

    pub fn process(&self, state: &mut PaintState) {
        while let Ok(ack) = self.rx.try_recv() {
            state.last_acked = Some(ack.id);
            if state.in_flight == Some(ack.id) { state.in_flight = None; }
        }
    }
}
```

The point isn't tracking frames for their own sake — it's that the same
"write intent sync, fire async, diff is Noop until Done" pattern every other
driver uses applies here too. Without a source, paint becomes a free
function in the runtime that can't be mocked, can't be throttled at its
natural seat, and can't be traced with the same mechanism as every other
driver.

### Stateless drivers still need an in-flight source

A driver that's purely a wrapper around a platform API — OS clipboard,
one-shot shell command, single DNS lookup — has no persistent external-fact
source. But it still has state while an operation is in flight ("is a read
currently outstanding?", "is a write queued?").

That state belongs on a **driver-owned source**, never piggybacked onto the
user-decision source that triggered the operation:

```rust
// NO: flag lives on a user-decision source
pub struct KillRing {
    pub entries: imbl::Vector<Arc<str>>,
    pub clipboard_read_in_flight: bool,  // ← driver state on a user source
}

// YES: minimal driver-owned source
pub struct ClipboardState {
    pub read: ReadState,    // Idle | InFlight
    pub write: WriteState,  // Idle | Pending(Arc<str>)
}
```

Without this split, the user-decision source accumulates async bookkeeping
over time, its `PartialEq` picks up churn from driver completions, and the
rule that says "user decisions are chosen, not discovered" quietly rots.

### One driver, two crates

"The driver" as described here is a unit, but it's best organised as two
crates: a portable sync **core** (the source + the sync `process`/`execute`
API + the `Cmd`/`Event` types that cross to the async side) and a
platform-specific **native** (the worker implementation — thread on
desktop, GCD on iOS, coroutines on Android). The mpsc between them *is*
the ABI boundary, and it's also the mock point for tests. See
[Organizing the code](#organizing-the-code-crate-layout).

---

## Queries: desired state, not transitions

The fundamental unit of logic in a query-driven app is a **query** (a `drv`
memo) that describes what should be true given the current inputs.

### Anti-pattern: transition handlers

```
// DON'T: one handler per transition
on_mic_selected(id)     → open_stream(id)
on_mic_disconnected(id) → if selected, set_reconnect_flag(id)
on_mic_reconnected(id)  → if reconnect_flag, reopen_stream(id)
on_permission_revoked() → close_stream(), clear_selection()
on_switch_mic(old, new) → close_stream(old), open_stream(new)
```

Each handler encodes one transition. Miss a handler and you have a bug.
The "close old stream when switching mics" case is easy to forget.

### Pattern: desired-state query

```rust
/// "What mic should be open right now?"
/// Multi-input memo: inputs from two sources, only the fields that matter.
#[drv::memo(single)]
fn desired_mic<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> Option<MicId> {
    match (inv.permission, pref.selected) {
        (MicPermission::Granted, Some(id))
            if inv.mics.iter().any(|m| m.id == *id) => Some(*id),
        _ => None,
    }
}
```

This single function handles every transition:

| Scenario | Inputs | Result | Effect |
|----------|--------|--------|--------|
| No mic selected | selected=None | None | Stream stays closed |
| Mic selected, permission pending | permission=Pending | None | Nothing yet |
| Permission granted, mic available | permission=Granted, mic in list | Some(mic1) | Open stream |
| Mic disconnected (USB yanked) | mic not in list | None | Close stream |
| Mic reconnected (USB reinserted) | mic back in list | Some(mic1) | Reopen stream |
| Permission revoked | permission=Denied | None | Close stream |
| Switch to mic2 | selected=mic2 | Some(mic2) | Close mic1, open mic2 |

No transition handler needed for any of these. The query describes the
desired end state. The diff against actual state determines the action.

---

## Actions as diffs

A query describes what should be true. The driver's source describes what is
actually true. The diff is the action.

```rust
/// "What should we do about the mic stream?"
#[drv::memo(single)]
fn mic_action<'a, 'b, 'c>(
    inv: MicListInput<'a>,
    pref: MicSelectInput<'b>,
    cap: MicCapStateInput<'c>,
) -> MicAction {
    let desired = desired_mic(inv, pref);
    let actual = match cap.state {
        CaptureState::Live => *cap.device_id,
        CaptureState::Opening => *cap.device_id,  // already in progress
        _ => None,
    };
    match (desired, actual) {
        (Some(d), None)              => MicAction::Open(d),
        (Some(d), Some(a)) if d != a => MicAction::Switch(d),
        (None, Some(_))              => MicAction::Close,
        _                            => MicAction::Noop,
    }
}
```

Note how `Opening` is treated the same as `Live` for the "actual" side —
the stream is either open or in the process of being opened. This prevents
re-triggering an open that's already in flight.

---

## The main loop

The main loop has three phases, every tick:

```rust
loop {
    // ── Phase 1: Ingest ──────────────────────────────────────
    // Drain async driver results into sources.
    // Drain UI events into user-decision sources.
    mic_enum_driver.process(&mut mic_inventory);
    mic_stream_driver.process(&mut mic_capture);
    camera_driver.process(&mut camera_inventory);
    signaling_driver.process(&mut call_session);
    webrtc_driver.process(&mut remote_participants, &mut remote_streams);
    ui_events.process(&mut mic_preference, &mut participant_settings, ...);

    // ── Phase 2: Query ───────────────────────────────────────
    // Ask questions. All reads, no writes. Memoized — mostly cache hits.
    let mic_act = mic_action(
        MicListInput::new(&mic_inventory),
        MicSelectInput::new(&mic_preference),
        MicCapStateInput::new(&mic_capture),
    );
    let cam_act = camera_action(
        CamListInput::new(&camera_inventory),
        CamSelectInput::new(&camera_preference),
        CamCapStateInput::new(&camera_capture),
    );

    // ── Phase 3: Execute ─────────────────────────────────────
    // Act on query results. Writes intent into sources + starts async work.
    mic_stream_driver.execute(mic_act, &mut mic_capture);
    camera_stream_driver.execute(cam_act, &mut camera_capture);

    // ── Phase 4: Render ──────────────────────────────────────
    // Compute view models (also memoized queries).
    // Push to UI framework if changed.
    update_views(&mic_inventory, &mic_preference, &mic_capture,
                 &remote_participants, &participant_settings, ...);
}
```

Every phase is clearly separated:
- **Ingest**: writes to sources from outside the app. No reads.
- **Query**: reads from sources. No writes. All memoized.
- **Execute**: writes intent to sources + starts async work.
- **Render**: reads from sources. Pushes to UI. All memoized.

### Wake on event, don't spin

Block the main thread until something actually happens — a driver
completion, a user keystroke, a timer deadline. **Never sleep a fixed
duration "just in case."** A 100 Hz fixed tick means 100 cache reads per
second burning CPU for no reason on every idle moment, and adds up to one
full tick of latency when an event arrives right after a sleep starts.
Treat fixed-tick loops as an anti-pattern — they're easier to write for
five minutes and a drag on the app forever after.

The specific primitive depends on how the drivers are wired:

**Sync drivers (std threads + `std::mpsc`).** The main thread owns a
`Receiver<()>`; every driver, the input source, and timer machinery hold
clones of the matching `Sender`. Drivers signal after each completion;
input signals on each key. The loop blocks on
`recv_timeout(nearest_deadline)`; the timeout is only there so wall-clock
timers (toast expiry, animation frames) don't oversleep.

```rust
let wake = Wake::new();  // Sender<()> + Receiver<()>
let drivers = spawn_drivers(trace, &wake)?;  // each gets a wake.tx clone

loop {
    // ingest → query → execute → render

    // Drain stale signals — already accounted for by the ingest above.
    while wake.rx.try_recv().is_ok() {}

    let timeout = nearest_deadline(&atoms)
        .and_then(|d| d.checked_duration_since(Instant::now()))
        .unwrap_or(Duration::from_secs(60));
    match wake.rx.recv_timeout(timeout) { /* ... */ }
}
```

**Async drivers (tokio tasks).** The main task `select!`s over driver
completion channels plus `sleep_until(nearest_deadline)`:

```rust
loop {
    // ingest → query → execute → render

    tokio::select! {
        _ = driver_completions.recv() => {}
        _ = input_rx.recv() => {}
        _ = tokio::time::sleep_until(deadline) => {}
    }
}
```

Same shape, different primitives. Pick whichever matches the rest of the
app; mixing is fine (a std thread can forward into a tokio channel via
`spawn_blocking`).

**Readiness signals from the OS (inotify, kqueue, epoll, socket ready).**
Never poll these. The kernel already knows when something happened — use
`notify`, the raw OS API, or tokio's `AsyncFd`, and have the native
worker forward into the wake channel the instant the event fires. A
polling layer between the kernel and your main loop is pure waste.

### Centralize deadline management

Any non-trivial app has multiple pending timers: a toast auto-dismiss, a
debounced recomputation, an LSP response timeout, an animation frame.
Hardcoding one into the loop's timeout works for the first feature; the
second feature has to find-and-replace.

Fold all pending deadlines into one query:

```rust
#[drv::memo(single)]
fn nearest_deadline(
    alerts: AlertsInput<'_>,
    lsp: LspInput<'_>,
    anim: AnimationsInput<'_>,
) -> Option<Instant> {
    [alerts.info_expires_at, lsp.request_deadline, anim.next_frame_at]
        .into_iter()
        .flatten()
        .min()
}
```

The loop reads one value; adding a timer means adding a field to the fold.
This scales; ad-hoc `.unwrap_or(60s)` in the loop does not.

---

## Organizing the code: crate layout

Everything above describes the shape of the code at runtime. A second
decision is *where* the code lives. The patterns below keep the model
scalable across domains and portable across platforms.

### One driver per crate pair

For any app where the async side could plausibly be replaced — desktop
thread, iOS GCD, Android coroutines, or a fake for tests — split each
driver into **two crates**:

```
driver-<name>/
  core/    → led-driver-<name>-core       portable: source + sync API
                                          + `Cmd`/`Event` ABI types
                                          + trace trait. Pure Rust, no
                                          platform deps.
  native/  → led-driver-<name>-native     platform-specific: the async
                                          worker (thread/GCD/coroutine)
                                          connected via mpsc.
```

The **core** crate knows nothing about any platform. It holds:

- The source(s) the driver owns.
- The sync driver struct with `process` / `execute` (or analogous)
  methods the main loop calls.
- The `Cmd` / `Done` / `Event` types that cross to the async worker.
- A `Trace` trait the runtime implements to hook observable events.

The **native** crate implements the async worker against those types,
typically exposing a `spawn(trace) -> (Driver, Native)` convenience.
Swapping native implementations per platform is the whole point of the
split; the core crate is shared.

### Multiple platforms: one native per platform, not cfg

When the same driver concept exists on multiple targets (desktop / iOS /
Android), prefer **separate `*-native-<platform>` crates** over one
crate with `#[cfg(target_os = ...)]` inside:

```
driver-buffers/
  core/
  native-desktop/    Rust thread + std::fs
  native-ios/        cdylib bridge: extern "C" shims;
                     Swift + GCD lives in a SwiftPM package
                     that links this
  native-android/    cdylib bridge via jni; Kotlin + coroutines
                     live in a Gradle module that loads it
```

Reasons:

- **Deps diverge.** Desktop wants `std::fs`. iOS wants Foundation
  bindings (e.g. `objc2` or hand-rolled FFI). Android wants `jni` and
  whatever native libraries you bind. Each crate's `Cargo.toml` only
  carries what it actually needs.
- **No dead code compiled.** With cfg inside one crate, iOS still
  parses the desktop `mod` tree even if gated out; build time and
  error-surface both bloat. Separate crates don't pay that.
- **Cargo.toml is the enforcement.** Separate crates let the compiler
  stop you from accidentally calling a desktop API on iOS.

Cfg *is* appropriate for small leaf differences inside an otherwise-
shared native (e.g., a desktop native covering macOS and Linux with one
`cfg` flip for a syscall). Top-level platform selection is the wrong
place for it.

The runtime crate picks one of the natives via **target-specific
dependencies** in its `Cargo.toml`:

```toml
[target.'cfg(all(not(target_os = "ios"), not(target_os = "android")))'.dependencies]
my-app-driver-buffers-native-desktop = { workspace = true }

[target.'cfg(target_os = "ios")'.dependencies]
my-app-driver-buffers-native-ios = { workspace = true }

[target.'cfg(target_os = "android")'.dependencies]
my-app-driver-buffers-native-android = { workspace = true }
```

That works when the shape of `spawn_drivers` is identical across
platforms. When the runtime itself differs substantially — notably
because the UI driver is entirely different (terminal vs SwiftUI vs
Jetpack Compose) — prefer **parallel runtime crates** instead:

```
crates/
  runtime-desktop/   wires terminal UI driver + desktop workers
  runtime-ios/       wires SwiftUI bridge + iOS workers
  runtime-android/   wires Jetpack Compose bridge + Android workers
```

Each runtime crate depends on the `*-core` crates (shared) plus the
right `*-native-<platform>` crate. This scales cleanly when the
platforms have genuinely different wiring needs, which they usually do.

### User-decision sources have no driver

A user-decision source is chosen, not discovered — no async side, no
worker, no driver struct. Let it sit in a plain `state-*` crate and be
mutated directly by dispatch in the runtime. Don't force it into the
driver shape for consistency; the driver shape is for units that have
an async peer.

### Cross-driver composition lives in a runtime crate

**Drivers must not know about each other.** `driver-foo/core/` must not
import `driver-bar/core/` or `state-baz/`. Each driver is independently
testable, independently swappable, independently mockable.

Every memo that combines inputs from multiple drivers' sources lives in a
separate **runtime** crate that depends on all of them. That runtime
crate also owns:

- `Event` + `dispatch` (the user-event handler that mutates driver sources).
- `Trace` / unified trace sink that driver traces forward to.
- The main loop `run` itself.
- The platform-specific wiring that spawns native workers and builds
  the `Drivers` bundle.

Cross-platform payoff: a mobile target replaces the runtime crate
around the same `*-core` crates; the drivers stay intact.

```
crates/
  core/                              shared primitives (paths, id newtypes, ...)
  state-<name>/                      user-decision source + its own memos (self-contained)
  platform-<name>/                   shared platform primitive used by >1 driver
    core/                            portable sync driver + source + ABI types
    native/                          platform-specific async worker
  driver-<name>/                     domain driver (room, mic, camera, ...)
    core/                            portable sync driver + source + ABI types
    native/                          platform-specific async worker
  abi-<name>/                        stable-ABI types crossing Rust ↔ native (Swift/Kotlin)
  runtime/                           cross-source inputs + memos + dispatch + main loop
<bin>/                               thin main: CLI parse, raw-mode guard, run
```

### Platform drivers: shared primitives as a third tier

Some drivers wrap a **platform-provided primitive** that multiple domain
drivers need. A UDP socket driver used by a WebRTC stack *and* by an
opportunistic peer-discovery component. An HTTP driver used by auth,
signaling, and push. A timer driver used by everything that needs
`wake_me_at(t)`. These are infrastructure, not domain state.

They sit **below** domain drivers in the dependency graph and
deliberately break one rule from the previous section: *cross-driver
imports into them are expected*. A domain driver importing
`platform-http-core`'s `Cmd` / `Event` ABI is how it speaks HTTP at
all. The isolation rule is about stopping domain drivers from knowing
about each other's domains — it is not about stopping them from using
shared transport.

Give them a distinct crate prefix — `platform-*` — so the role is
visible at a glance and the exception doesn't read as drift. The
three-tier dependency rule the `Cargo.toml` graph then enforces:

- `state-*` depends on nothing (or only on shared primitive crates).
- `platform-*` may depend on `state-*` and shared primitives; not on
  `driver-*`.
- `driver-*` may depend on `platform-*` and on `state-*`; must not
  depend on other `driver-*`.
- `runtime` depends on all of them.

**Not every cross-cutting concern is a platform driver.** The criterion
is *multiple domain drivers actively call into it* — i.e. they import
its `Cmd` / `Event` ABI and issue requests. It is **not** *multiple
parts of the app observe it*. A lifecycle / foreground-background
driver has a singleton source that runtime-level memos read and feed
into dispatch; no domain driver imports `driver-lifecycle-core`. That
is a regular `driver-*` with a cross-cutting observer pattern, not a
platform driver. The `platform-*` prefix is for transport and
infrastructure that domain drivers actively issue requests to.

**Shape-wise:** platform drivers are usually stateless in the domain
sense but **multi-request**. Their source is typically a
`HashMap<RequestId, InFlight>` keyed by caller id, not a singleton
struct. Callers allocate request ids and correlate via events. This is
the usual "stateless drivers need in-flight sources" pattern — the
in-flight state is just plural.

### Cross-crate inputs: the consumer declares its own

Inputs live in the crate that uses them. A source crate (e.g.
`driver-mic-enum/core`) exports its plain struct and doesn't depend on
`drv`. The consumer crate declares the input, the projection, and the
memo:

```rust
// In the runtime crate, projecting MicInventory from the sibling
// driver-mic-enum/core crate.

#[drv::input]
struct MicsInput<'a> {
    pub mics: &'a imbl::Vector<MicDevice>,
}

impl<'a> MicsInput<'a> {
    pub fn new(m: &'a MicInventory) -> Self {
        Self { mics: &m.mics }
    }
}

#[drv::memo(single)]
fn mic_count<'a>(inv: MicsInput<'a>) -> usize {
    inv.mics.len()
}

// Call site:
// mic_count(MicsInput::new(&mic_inventory))
```

No registry, no cross-crate coordination. The consumer crate is the only
one that needs a `drv` dependency.

### Drop-order: don't join in `Drop`

The sync driver holds the `Sender<Cmd>`; its native handle typically
wraps a `JoinHandle<()>`. If you construct both via `let (driver, native)
= spawn(...)`, Rust drops `native` before `driver` (reverse
declaration order). If `native`'s `Drop` calls `join()`, it deadlocks —
the worker is still blocked on `recv()` waiting for `driver`'s
`Sender` to close.

Safer: don't `join` in the native's `Drop`. Let the worker self-exit on
channel hangup when the sync driver's `Sender` drops, and let process
exit reap any straggler. If a specific test needs deterministic
shutdown, drop the driver explicitly first.

---

## Crossing the ABI: foreign-owned buffers

Mobile targets compile Rust as a static lib (iOS) or cdylib (Android)
and hand data across a C ABI to Swift / Kotlin. For small one-shot
values — a bool, an integer — copy; the cost is nothing. For **byte
buffers** (media frames, encoded packets, screen captures, channel
payloads) copying on every hop defeats the point of a native client.

This is not a driver concern. It is a thin layer of **types with
stable ABI and explicit lifetime rules**, kept in its own
`abi-<name>/` crate (see the crate layout above) so any driver or
runtime can import it without pulling driver dependencies along.

Two directions to model:

- **Rust-owned, foreign-consumed.** A `#[repr(C)]` handle carries a
  stable `(ptr, len)` view plus a raw pointer into whatever Rust
  container owns the allocation, paired with an `extern "C"` free
  function that reconstitutes and drops it. Ownership invariant: the
  foreign side calls the free exactly once. Which Rust container the
  handle wraps is an application choice.

- **Foreign-owned, Rust-consumed.** Usually call-scoped: the foreign
  side lends a pointer + length for the duration of one extern call,
  Rust copies what it needs within the call, the foreign side retains
  ownership. When a buffer must outlive the call, mirror the shape
  above in reverse — a foreign-owned handle with a Rust-invoked free
  callback.

Platform-typed GPU buffers (`CVPixelBuffer`, `HardwareBuffer`,
`AImage`) are out of scope for this layer — they are opaque handles
with their own lifecycle and retain semantics, often backed by GPU
memory. Model them as platform-specific types in the relevant
driver's core crate rather than forcing them into a contiguous-`[u8]`
shape.

## Bridging to a reactive UI framework (SwiftUI)

SwiftUI is reactive — it re-renders views when `@Observable` properties
change. `drv` is query-driven — it returns cached values when you ask.
The bridge converts pull into push.

### One memo per view

Each view has a corresponding memo that produces a view-specific model:

```rust
#[drv::memo(single)]
fn mic_picker_model<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> MicPickerModel {
    match inv.permission {
        MicPermission::NotAsked => MicPickerModel::NeedPermission,
        MicPermission::Pending  => MicPickerModel::Waiting,
        MicPermission::Denied   => MicPickerModel::Denied,
        MicPermission::Granted  => MicPickerModel::Ready {
            mics: inv.mics.iter().cloned().collect(),
            selected: *pref.selected,
        },
    }
}

// Inputs for participant grid — only the fields this view needs.
#[drv::input]
struct GridParticipantsInput<'a> {
    pub participants: &'a imbl::Vector<Participant>,
}

impl<'a> GridParticipantsInput<'a> {
    pub fn new(p: &'a RemoteParticipants) -> Self {
        Self { participants: &p.participants }
    }
}

#[drv::input]
struct GridSettingsInput<'a> {
    pub muted: &'a imbl::HashSet<ParticipantId>,
    pub pinned: &'a Option<ParticipantId>,
}

impl<'a> GridSettingsInput<'a> {
    pub fn new(s: &'a ParticipantSettings) -> Self {
        Self { muted: &s.muted, pinned: &s.pinned }
    }
}

#[drv::memo(single)]
fn participant_grid_model<'a, 'b>(
    p: GridParticipantsInput<'a>,
    s: GridSettingsInput<'b>,
    m: &RemoteStreams,   // whole source; cached as `Clone` + `PartialEq`
) -> ParticipantGridModel {
    p.participants.iter().map(|participant| {
        GridTile {
            name: participant.name.clone(),
            muted: s.muted.contains(&participant.id),
            pinned: *s.pinned == Some(participant.id),
            has_video: m.has_video_for(participant.id),
        }
    }).collect()
}
```

### The bridge layer

The bridge holds the previous model and pushes to the Swift side only when
the model changes:

```rust
struct ViewBridge<T: PartialEq + Clone> {
    current: Option<T>,
    push_fn: Box<dyn Fn(&T)>,  // FFI call to update Swift @Observable
}

impl<T: PartialEq + Clone> ViewBridge<T> {
    fn update(&mut self, new_value: T) {
        if self.current.as_ref() != Some(&new_value) {
            (self.push_fn)(&new_value);
            self.current = Some(new_value);
        }
    }
}
```

### The Swift side

```swift
@Observable
class MicPickerVM {
    var permission: MicPermission = .notAsked
    var mics: [MicDevice] = []
    var selected: MicId? = nil

    // Called from Rust via FFI when the model changes
    func update(from model: MicPickerModel) {
        permission = model.permission
        mics = model.mics
        selected = model.selected
    }
}

struct MicPickerView: View {
    @State var vm: MicPickerVM

    var body: some View {
        switch vm.permission {
        case .needPermission:
            Button("Grant Mic Access") { RustBridge.requestMicPermission() }
        case .waiting:
            ProgressView("Requesting access...")
        case .denied:
            Text("Microphone access denied")
        case .ready:
            List(vm.mics) { mic in
                MicRow(mic: mic, selected: mic.id == vm.selected)
                    .onTapGesture { RustBridge.selectMic(mic.id) }
            }
        }
    }
}
```

### Granularity chain

Three layers, each avoiding redundant work at its own level:

| Layer | Mechanism | What it avoids |
|-------|-----------|----------------|
| `drv` input | Field-by-field `PartialEq` on projected fields | Recomputing memo when irrelevant source fields changed |
| `PartialEq` bridge | `new_model != current_model` | Pushing to Swift when the model is identical |
| SwiftUI `@Observable` | Per-property tracking | Re-rendering views that didn't read the changed property |

### User events flow back

SwiftUI user actions cross the FFI boundary in the opposite direction and
mutate user-decision sources:

```
SwiftUI onTapGesture
  → RustBridge.selectMic(id)      // FFI call
    → mic_preference.selected = Some(id)  // source mutation
      → next tick: queries recompute, bridge pushes if changed
```

---

## Threading model

```
Background threads                        UI / Main thread
(async I/O, platform calls)              (sources, queries, render)

┌──────────────────────┐
│  mic_enum_driver     │──┐
│  (OS hotplug events) │  │
└──────────────────────┘  │
┌──────────────────────┐  │  mpsc / callback
│  mic_stream_driver   │──┼──────────────────→  main loop {
│  (audio capture)     │  │                       ingest
└──────────────────────┘  │                       query (memoized)
┌──────────────────────┐  │                       execute
│  signaling_driver    │──┤                       render
│  (WebSocket)         │  │                     }
└──────────────────────┘  │
┌──────────────────────┐  │
│  webrtc_driver       │──┘
│  (peer connections)  │
└──────────────────────┘
```

### Why sources stay on the UI thread

- Sources are plain structs; they're `Send` (or `Sync`) iff `T` is. The
  memoization caches live in per-memo `thread_local!` slots, so each
  thread builds its own cache independently — no shared state, no
  contention.
- Query/memo computation is cheap (cache hits ~10ns). Hundreds of queries
  per frame is < 1ms.
- The expensive work (I/O, network, audio processing) is already on
  background threads.
- Keeping sources on the UI thread means views can read them directly
  through inputs — no thread-boundary serialization for read access.
  A source *can* move between threads, but the cache doesn't follow;
  the new thread builds its own on first miss.

### Communication pattern

Background threads communicate with the main thread via channels
(`mpsc::Receiver`, `crossbeam::Receiver`, etc.) or callback dispatch
(e.g., `DispatchQueue.main.async` on iOS).

The main loop's **ingest phase** drains these channels into sources. This is
the only point where external data enters the query system.

---

## Worked example: microphone lifecycle

A complete walkthrough of the mic selection flow, showing sources, mutations,
queries, and actions at each step.

### Sources

Sources are plain Rust structs. Inputs — the projections memos read — are
declared separately, each with a `#[drv::input]` struct and a hand-written
projection (a `::new` method, a `From` impl, whatever reads naturally).
Always project only the fields the memo actually needs. Changes to
unrelated fields (e.g., `last_enumerated` timestamp updating frequently)
then don't trigger recomputation.

```rust
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicInventory {
    pub permission: MicPermission,
    pub mics: imbl::Vector<MicDevice>,
    pub last_enumerated: Option<Instant>,   // updated frequently, most memos don't care
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicPreference {
    pub selected: Option<MicId>,
    pub last_changed: Option<Instant>,      // for analytics, not for queries
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MicCapture {
    pub state: CaptureState,
    pub device_id: Option<MicId>,
    pub volume_level: f32,                  // updated every audio frame, mic_action doesn't need it
}
```

Inputs for the queries below project only what each one reads:

```rust
#[drv::input]
struct MicListInput<'a> {
    pub permission: &'a MicPermission,
    pub mics: &'a imbl::Vector<MicDevice>,
}

impl<'a> MicListInput<'a> {
    pub fn new(inv: &'a MicInventory) -> Self {
        Self { permission: &inv.permission, mics: &inv.mics }
    }
}

#[drv::input]
struct MicSelectInput<'a> {
    pub selected: &'a Option<MicId>,
}

impl<'a> MicSelectInput<'a> {
    pub fn new(pref: &'a MicPreference) -> Self {
        Self { selected: &pref.selected }
    }
}

#[drv::input]
struct MicCapStateInput<'a> {
    pub state: &'a CaptureState,
    pub device_id: &'a Option<MicId>,
}

impl<'a> MicCapStateInput<'a> {
    pub fn new(cap: &'a MicCapture) -> Self {
        Self { state: &cap.state, device_id: &cap.device_id }
    }
}
```

### Queries

Each memo takes inputs that project only the fields it needs. Changes to
fields outside the input (e.g., `volume_level` updating 60 times per
second) don't trigger recomputation of unrelated memos.

```rust
/// "What mic should be open right now?"
/// Needs permission + available mics + user selection. Does NOT need
/// last_enumerated, last_changed, or volume_level.
#[drv::memo(single)]
fn desired_mic<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> Option<MicId> {
    match (inv.permission, pref.selected) {
        (MicPermission::Granted, Some(id))
            if inv.mics.iter().any(|m| m.id == *id) => Some(*id),
        _ => None,
    }
}

/// "What should we do about the mic stream?"
/// Needs desired state + capture state. Does NOT need volume_level.
#[drv::memo(single)]
fn mic_action<'a, 'b, 'c>(
    inv: MicListInput<'a>,
    pref: MicSelectInput<'b>,
    cap: MicCapStateInput<'c>,
) -> MicAction {
    let want = desired_mic(inv, pref);
    let have = match cap.state {
        CaptureState::Live | CaptureState::Opening => *cap.device_id,
        _ => None,
    };
    match (want, have) {
        (Some(d), None)              => MicAction::Open(d),
        (Some(d), Some(a)) if d != a => MicAction::Switch(d),
        (None, Some(_))              => MicAction::Close,
        _                            => MicAction::Noop,
    }
}

/// "What should the mic picker view show?"
/// Needs permission + mics + selection. Same fields as desired_mic, different output.
#[drv::memo(single)]
fn mic_picker_model<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> MicPickerModel {
    match inv.permission {
        MicPermission::NotAsked => MicPickerModel::NeedPermission,
        MicPermission::Pending  => MicPickerModel::Waiting,
        MicPermission::Denied   => MicPickerModel::Denied,
        MicPermission::Granted  => MicPickerModel::Ready {
            mics: inv.mics.iter().cloned().collect(),
            selected: *pref.selected,
        },
    }
}

/// "What is the current mic volume?" — depends ONLY on volume_level.
/// Audio frame updates don't trigger mic_action or mic_picker recomputation.
#[drv::input]
struct MicVolumeInput<'a> {
    pub volume_level: &'a f32,
}

impl<'a> MicVolumeInput<'a> {
    pub fn new(c: &'a MicCapture) -> Self {
        Self { volume_level: &c.volume_level }
    }
}

#[drv::memo(single)]
fn mic_volume_display<'a>(vol: MicVolumeInput<'a>) -> VolumeLevel {
    VolumeLevel::from_linear(*vol.volume_level)
}
```

Call sites build inputs explicitly: `mic_action(MicListInput::new(&inv),
MicSelectInput::new(&pref), MicCapStateInput::new(&cap))`.

### Step-by-step flow

```
Step  What happens                 Source mutations                   Query results
────  ──────────────────────────── ──────────────────────────────── ──────────────────────
1     User opens mic picker        (none)                           mic_picker_model → NeedPermission
2     User taps "grant access"     inv.permission = Pending         mic_picker_model → Waiting
3     OS grants permission         inv.permission = Granted         mic_picker_model → Ready {
      Driver enumerates            inv.mics = [mic1, mic2]           mics: [mic1, mic2],
                                                                     selected: None }
4     User taps mic1               pref.selected = Some(mic1)       desired_mic → Some(mic1)
                                                                    mic_action → Open(mic1)
      Execute writes intent        cap.state = Opening              mic_action → Noop (next tick)
                                   cap.device_id = Some(mic1)
5     Driver reports stream open   cap.state = Live                 mic_action → Noop
6     User yanks USB               inv.mics removes mic1            desired_mic → None
      Driver reports disconnect    cap.state = Disconnected         mic_action → Close
      Execute writes intent        cap.state = Closing              mic_action → Noop (next tick)
7     User re-inserts USB          inv.mics adds mic1               desired_mic → Some(mic1)
                                                                    mic_action → Open(mic1)
      Execute writes intent        cap.state = Opening              mic_action → Noop (next tick)
8     Driver reports stream open   cap.state = Live                 mic_action → Noop
```

Reconnection (steps 6→7→8) requires zero special-case code. It falls out
from the query returning `Some(mic1)` (because `selected` was never cleared)
and the diff against `Disconnected` producing `Open(mic1)`.

---

## Worked example: remote participants

### Sources

```rust
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteParticipants {
    pub participants: imbl::Vector<Participant>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParticipantSettings {
    pub muted: imbl::HashSet<ParticipantId>,
    pub pinned: Option<ParticipantId>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteStreams {
    pub video_tracks: imbl::HashMap<ParticipantId, VideoTrackState>,
    pub audio_tracks: imbl::HashMap<ParticipantId, AudioTrackState>,
}
```

### Queries

```rust
#[drv::memo(single)]
fn participant_grid(
    p: &RemoteParticipants,
    s: &ParticipantSettings,
    streams: &RemoteStreams,
) -> Vec<GridTile> {
    let mut tiles: Vec<GridTile> = p.participants.iter().map(|participant| {
        GridTile {
            id: participant.id,
            name: participant.name.clone(),
            is_muted: s.muted.contains(&participant.id),
            is_pinned: s.pinned == Some(participant.id),
            has_video: streams.video_tracks.get(&participant.id)
                .is_some_and(|t| t.is_active()),
        }
    }).collect();

    // Pinned participant goes first
    if let Some(pin_id) = s.pinned {
        if let Some(pos) = tiles.iter().position(|t| t.id == pin_id) {
            let pinned = tiles.remove(pos);
            tiles.insert(0, pinned);
        }
    }

    tiles
}
```

When a participant leaves, the WebRTC driver removes them from
`RemoteParticipants`. The query naturally excludes them from the grid.
No "on participant left" handler needed. When the user mutes someone,
`ParticipantSettings.muted` is updated. The query includes the mute state.
No "on mute toggled, re-render tile" handler needed.

### Invariant enforcement

When a pinned participant leaves, the pinned state becomes stale. The query
handles this gracefully (the `position()` call returns `None`, so no tile is
moved to front). But the stale state should be cleaned up. This belongs in
the ingest phase:

```rust
// In the main loop's ingest phase, after processing webrtc events:
if let Some(pinned_id) = participant_settings.pinned {
    if !remote_participants.participants.iter().any(|p| p.id == pinned_id) {
        participant_settings.pinned = None;
    }
}
```

This is application logic — not a query, not a driver. It runs during ingest
because it's cleaning up a user-decision source in response to an external
fact changing.

---

## Testing

Query-driven architecture makes testing straightforward because queries are
pure functions.

### Unit testing a query

```rust
#[test]
fn desired_mic_requires_permission_and_availability() {
    let mut inv = MicInventory::default();
    let mut pref = MicPreference::default();

    // No permission → no desired mic
    pref.selected = Some(MicId(1));
    assert_eq!(desired_mic(MicListInput::new(&inv), MicSelectInput::new(&pref)), None);

    // Permission but mic not in list → no desired mic
    inv.permission = MicPermission::Granted;
    assert_eq!(desired_mic(MicListInput::new(&inv), MicSelectInput::new(&pref)), None);

    // Permission and mic available → desired
    inv.mics.push_back(MicDevice { id: MicId(1), name: "USB Mic".into() });
    assert_eq!(desired_mic(MicListInput::new(&inv), MicSelectInput::new(&pref)), Some(MicId(1)));

    // Mic removed → no desired mic (reconnect scenario)
    inv.mics.retain(|m| m.id != MicId(1));
    assert_eq!(desired_mic(MicListInput::new(&inv), MicSelectInput::new(&pref)), None);

    // Mic re-added → desired again (selection survived)
    inv.mics.push_back(MicDevice { id: MicId(1), name: "USB Mic".into() });
    assert_eq!(desired_mic(MicListInput::new(&inv), MicSelectInput::new(&pref)), Some(MicId(1)));
}
```

### Testing the full action cycle

```rust
#[test]
fn mic_action_produces_correct_diffs() {
    let mut inv = MicInventory {
        permission: MicPermission::Granted,
        ..Default::default()
    };
    let mut pref = MicPreference::default();
    let mut cap = MicCapture::default();

    inv.mics.push_back(mic(1));
    inv.mics.push_back(mic(2));

    let act = |inv: &MicInventory, pref: &MicPreference, cap: &MicCapture| {
        mic_action(
            MicListInput::new(inv),
            MicSelectInput::new(pref),
            MicCapStateInput::new(cap),
        )
    };

    // Select mic1 → should open
    pref.selected = Some(MicId(1));
    assert_eq!(act(&inv, &pref, &cap), MicAction::Open(MicId(1)));

    // Simulate execute: cap reflects Opening
    cap.state = CaptureState::Opening;
    cap.device_id = Some(MicId(1));
    assert_eq!(act(&inv, &pref, &cap), MicAction::Noop);

    // Stream live
    cap.state = CaptureState::Live;
    assert_eq!(act(&inv, &pref, &cap), MicAction::Noop);

    // Switch to mic2
    pref.selected = Some(MicId(2));
    assert_eq!(act(&inv, &pref, &cap), MicAction::Switch(MicId(2)));

    // Deselect
    pref.selected = None;
    assert_eq!(act(&inv, &pref, &cap), MicAction::Close);
}
```

No mocking. No async runtime. No event loop setup. Just pure functions with
known inputs and expected outputs.

### Testing a driver at the mpsc boundary

The sync driver is just `new(channels, ...) + process/execute`. In the
driver's `*-core` crate you can test it directly against synthetic
channel peers — no thread, no platform I/O, no other drivers:

```rust
#[test]
fn execute_writes_pending_sync_and_emits_cmd() {
    let (tx_cmd, rx_cmd) = mpsc::channel::<ReadCmd>();
    let (_tx_done, rx_done) = mpsc::channel::<ReadDone>();
    let driver = FileReadDriver::new(tx_cmd, rx_done, Arc::new(NoopTrace));

    let mut store = BufferStore::default();
    let acts = [LoadAction::Load(path.clone())];
    driver.execute(acts.iter(), &mut store);

    // (1) Sync state was written immediately.
    assert!(matches!(store.loaded.get(&path), Some(LoadState::Pending)));
    // (2) The command landed on the ABI boundary.
    matches!(rx_cmd.try_recv(), Ok(ReadCmd::Read(_)));
}

#[test]
fn process_applies_worker_completion() {
    let (tx_cmd, _rx_cmd) = mpsc::channel::<ReadCmd>();
    let (tx_done, rx_done) = mpsc::channel::<ReadDone>();
    let driver = FileReadDriver::new(tx_cmd, rx_done, Arc::new(NoopTrace));

    // Play the role of the native worker directly.
    tx_done.send(ReadDone { path: path.clone(), result: Ok(rope) }).unwrap();

    let mut store = BufferStore::default();
    driver.process(&mut store);
    assert!(matches!(store.loaded.get(&path), Some(LoadState::Ready(_))));
}
```

The mpsc pair is the **mock point**. Tests at this layer replace the
native worker with a directly-driven peer, which scales from unit tests
(send one `Done`) to integration tests (script a whole session of
`Cmd`/`Done` exchanges). Integration tests against the real native
worker live in the `*-native` crate and can be sparser, because the
logic above the ABI is already covered.

### Mid-layer integration tests

Unit tests cover queries (pure functions) and driver cores (at the mpsc).
End-to-end tests (real binary, real I/O) cover everything together but are
slow and hard to diagnose. The gap between them — "does dispatch actually
produce the right driver commands for this keystroke?" — is where plumbing
bugs hide.

Cover it with mid-layer tests: real dispatch, real runtime composition,
but driver workers replaced by capturing mocks at the mpsc. Assert on the
sequence of `Cmd`s emitted and on post-mutation source state:

```rust
#[test]
fn save_keybinding_emits_save_cmd_for_dirty_active_buffer() {
    let mut world = World::fixture_with_dirty_buffer("foo.rs");
    let (drivers, captures) = capturing_drivers();

    dispatch(Event::Key(ctrl('s')), &mut world);
    execute(&mut world, &drivers);

    assert_eq!(captures.file_write.drain(), vec![save_cmd("foo.rs")]);
    assert!(!world.edits.pending_saves.contains(Path::new("foo.rs")));
}
```

These catch wiring mistakes (wrong source mutated, wrong driver called,
execute-pattern discipline broken) before they surface as mysterious
end-to-end diffs.

---

## Guidelines

### 1. External facts and user decisions go in separate sources

Don't mix "what the OS told me" with "what the user chose" in the same source.
Drivers manage external facts. UI handlers manage user decisions. Memos
combine them.

### 2. Queries describe desired state, not transitions

Write "what mic should be open?" not "when the user selects a mic, open it."
The diff between desired and actual is the action.

### 3. Execute writes intent synchronously

When acting on a query result, update the driver's source to reflect the
in-progress operation *before* starting the async work. This prevents the
query from returning the same action on the next tick.

### 4. The main loop is ingest → query → execute → render

Keep the phases separate. Ingest writes to sources from outside the app.
Query reads sources (memoized). Execute acts on query results and writes
intent. Render computes view models (also memoized) and pushes to UI.

### 5. Use `imbl` collections for large or frequently-cloned data

Enable the `imbl` feature in `drv`. Persistent collections give O(1) clone
(cheap snapshots on cache miss) and O(1) pointer-equality comparison on
cache hit.

### 6. One memo per view for the UI bridge

Each UI component gets a memo producing its specific view model. The bridge
pushes to the reactive framework only when the model changes (`PartialEq`).
The reactive framework handles per-property granularity from there.

### 7. Clean up stale user decisions in the ingest phase

When an external fact invalidates a user decision (pinned participant left,
selected mic removed), clean up the user-decision source during ingest. This
is application logic, not a query.

### 8. Keep drivers ignorant of each other

The mic enumeration driver should not know about the stream driver. The
stream driver should not know about user preferences. Memos express the
relationships between domains. Drivers just keep their source in sync with
reality.

Exception: `platform-*` drivers (guideline 13) are shared
infrastructure and are *meant* to be imported by multiple domain
drivers. The ignorance rule applies between *domain* drivers.

### 9. Enforce driver ignorance with crate boundaries

Don't rely on discipline — put each driver in its own crate (or crate
pair) that has no dependency on other drivers or on sibling
user-decision state. The Cargo.toml is the constraint; the compiler
rejects accidental coupling. Cross-source composition lives in a separate
runtime crate that depends on all of them.

### 10. Split each driver into a portable core and a platform-specific native

The sync API + source + ABI types are the same on every platform. The
async worker is not. Putting them in separate crates means the core
crate compiles on every target and stays thin; the native crate is
swapped (Rust thread / Swift + GCD / Kotlin + coroutines / mock for
tests) without touching anything upstream.

### 10a. One native crate per platform, not `cfg` inside one crate

When the same driver exists on multiple platforms, ship one
`*-native-<platform>` crate per target instead of a single native with
`#[cfg(target_os = ...)]` gates. Separate crates keep each platform's
dependency set narrow, avoid compiling dead code, and let the compiler
(via `Cargo.toml`) reject accidental cross-platform calls. The runtime
picks one via target-specific dependencies — or, when the UI differs
substantially per platform, ships as parallel `runtime-<platform>`
crates depending on the shared `*-core` crates plus the right native.

### 11. Declare inputs in the crate that uses them

When a memo in crate B projects a source defined in crate A, declare the
`#[drv::input]` struct in crate B alongside a hand-written projection
(inherent `::new` method, `From` impl, etc.). The source crate stays
plain and doesn't need `drv` as a dependency.

### 12. Don't `join()` native workers in `Drop`

The mpsc hangup is the shutdown signal: when the sync driver's `Sender`
drops, the worker's `recv` returns `Err` and it exits. A `join()` in
the native handle's `Drop` deadlocks whenever the native handle drops
before the sync driver (e.g. reverse-order drops in a tuple binding).

### 13. Platform primitives go in a `platform-*` tier, not `driver-*`

When a driver wraps a platform-provided primitive (UDP, HTTP, timers,
keychain) that *multiple* domain drivers actively call into, put it in
a `platform-<name>/` crate pair. Cross-driver imports into it are
expected — a domain driver importing `platform-http-core`'s `Cmd`
types is how it speaks HTTP at all. This is the one exception to
guideline 8.

The promotion criterion is *multiple domain drivers issue requests to
it*, not *wraps a platform API* and not *observed by multiple parts of
the app*. One-consumer wrappers stay in `driver-*`; event-observed
singletons (lifecycle, locale) stay in `driver-*` too. Promote to
`platform-*` only when the second *caller* appears.

### 14. ABI-crossing types live in `abi-*` crates, separate from drivers

Cross-language byte buffers and other stable-ABI wrappers (the kind
that move through `extern "C"` and need matching Swift / Kotlin
handling) don't belong in driver cores — they are type and lifetime
contracts, not behaviours. Keep them in small `abi-*` crates that any
driver or runtime can import. See *Crossing the ABI: foreign-owned
buffers*.
Let the worker self-exit; process exit reaps any straggler.

### 13. Keep ABI types in the driver's `*-core` crate

When a `Cmd` or `Done` carries a compound type (`DirEntry`, `FrameDelta`,
`LspResponse`), that type lives in the driver's `*-core` crate, not in
whichever state crate happens to consume it. Consumer crates depend on
driver-core for ABI types, never the reverse — even when the consumer is
the type's only reader today.

Getting this backward (driver-core imports a state crate because "the
state crate defined the type first") re-couples every driver to every
consumer that touches its ABI, and the compiler stops catching isolation
violations. The type always belongs on the driver side of the ABI.

### 14. Zero allocation on idle ticks

An idle tick should be free: every memo cache-hits, every `execute`
iterates an empty collection, paint is skipped. Allocation on the idle
path is a bug even when it's "only" a `Vec::new()` — it's noise in
profiles, pressure on the allocator, and a leading indicator that a memo
has escaped its cache.

Memo outputs wrapping large data are `Arc`-wrapped (`Arc<Vec<T>>`,
`Arc<str>`) so cache-hit clones are refcount bumps. Collections used in
memos are `imbl` so structural sharing survives the clone. Action memos
filter before cloning (return `Noop` rather than `Open(...)` that the
driver will no-op on). Render walks views (`RopeSlice`, `&str`) rather
than materializing intermediate owned collections.

This discipline regresses on its own as features land. Audit the hot
paths periodically.
