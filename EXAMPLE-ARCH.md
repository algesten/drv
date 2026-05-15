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
9. [Testing](#testing)

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
| Two inputs change in the same iteration | Observer ordering matters; need batching to avoid glitches | Set both inputs, then query. Always consistent. |
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
declared with `#[derive(drv::Input)]`):

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
pub struct Persisted(Arc<String>);
pub struct Draft(Arc<String>);

pub struct Document {
    pub persisted: Persisted,
    pub draft:     Draft,
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
    /// Called on the main thread each iteration. Drains pending events
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
    /// Must write Pending synchronously — without it, the next iteration's
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

### Anti-pattern: derived UI state

The query-driven discipline doesn't stop at the runtime boundary. The
UI layer (Kotlin / Swift / SwiftUI / Compose / a TUI render loop) is a
driver too: its job is to *render* the queries the runtime exposes,
not to *re-derive them* on the platform side.

Concrete smell: any boolean or enum in your UI code that combines two
or more fields read from the runtime (over FFI, over a channel, over
whatever surface). It is a query wearing the wrong language.

```kotlin
// ANTI-PATTERN: query in Kotlin
val connected = handle != 0L && (exit == null || exit == Running)
//              └── UI fact ──┘   └─── runtime-observed facts ───┘
```

```swift
// SAME ANTI-PATTERN: query in Swift
let connected = runtime.lastExit == nil && runtime.isAlive
//              └────── two runtime-observed facts combined here ─────┘
```

The runtime crate has all the source state; the UI is reading
snapshots over a narrow surface. The combination rule belongs *with*
the sources, not on the consumer side. Two implementations of the
same rule in two languages are also two opportunities to drift —
nothing forces them to agree.

The corrective shape: write the rule as a memo over the runtime's
sources, expose its output as a single value across the boundary, and
let the UI read it.

```rust
// In the runtime crate:

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConnectionPhase { Disconnected, Connecting, Connected, Reconnecting, Failed }

impl ConnectionPhase {
    pub fn is_active(self) -> bool {
        matches!(self, ConnectionPhase::Connecting | ConnectionPhase::Connected
                     | ConnectionPhase::Reconnecting)
    }
}

#[drv::memo(single)]
pub fn connection_phase<'a, 'b>(
    desired: DesiredInput<'a>,
    rtc:     RtcInput<'b>,
) -> ConnectionPhase { /* … */ }
```

```kotlin
// On the UI side:
val connected = isPhaseActive(Native.connectionPhase(handle))
```

```swift
// On the UI side:
let connected = runtime.connectionPhase.isActive
```

One rule, in one place, derived from sources the cascade already
tracks, surfaced over the FFI as a single value the UI renders. Adding
inputs (a permission state, a network gate, a pre-flight check) is one
change in the memo plus zero changes in the UIs.

#### How to spot it on review

- If a diff adds `&&` or `||` in UI code over fields you can name as
  "things I read from the runtime," reject in favour of a memo.
- If two platform UIs end up with structurally similar `if`/`when`
  ladders, the ladder belongs in a memo.
- If you find yourself writing `onChange` / `LaunchedEffect` /
  `Observer` whose job is "sync this derived state when an input
  changed," it's a memo with extra steps.

UI driver lifecycle (handle allocation, async resource freeing) is
a legitimate UI-side concern — that's the "execute pattern" from
earlier, just with the UI as the driver. The line is: lifecycle of
the UI's own artifacts vs. derivation from runtime-observed facts.

---

## The main loop

"Main loop" here means the runtime's own dedicated loop thread — the
one that owns the sources and runs ingest/query/execute/render. It is
*not* the platform UI thread (SwiftUI's main, Android's UI thread,
the terminal's render thread). Those are separate; the UI should be
considered another driver.

The main loop has three phases, every iteration:

```rust
loop {
    // ── Phase 1: Ingest ──────────────────────────────────────
    // Drain each driver's pending events into its source. Drivers'
    // workers signaled wake; this just reads what they already
    // queued. No platform calls happen here.
    mic_enum_driver.process(&mut mic_inventory);
    mic_stream_driver.process(&mut mic_capture);
    camera_driver.process(&mut camera_inventory);
    signaling_driver.process(&mut call_session);
    webrtc_driver.process(&mut remote_participants, &mut remote_streams);
    ui_driver.process(&mut mic_preference, &mut participant_settings, ...);

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
completion, a user keystroke, or a deadline the runner needs to wake
for. **Never sleep a fixed duration "just in case."** A 100 Hz fixed
tick means 100 cache reads per second burning CPU on every idle
moment, and adds one full tick of latency when an event arrives right
after a sleep starts.

The specific primitive depends on how the drivers are wired:

**Sync drivers (std threads + `std::mpsc`).** The main thread owns a
`Receiver<()>`; every driver and input source holds a clone of the
matching `Sender`. The loop blocks on `recv_timeout(timeout)`; the
timeout comes from a deadline query (see *Deadlines are queries*
below) so the loop wakes at the nearest pending deadline.

```rust
let wake = Wake::new();
let drivers = spawn_drivers(trace, &wake)?;

loop {
    // ingest → query → execute → render

    let timeout = nearest_deadline(...)
        .and_then(|d| d.checked_duration_since(Instant::now()))
        .unwrap_or(Duration::from_secs(60));
    match wake.rx.recv_timeout(timeout) { /* ... */ }

    // Drain AFTER recv_timeout, never before — execute may have
    // kicked off work that signals wake before the sleep, and a
    // pre-sleep drain would swallow it.
    while wake.rx.try_recv().is_ok() {}
}
```

**Async drivers (tokio tasks).** The main task `select!`s over driver
completion channels plus `sleep_until(deadline)`:

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

### Time is a source field

"Now" is data, not a global. Put it on a source, set it during ingest
each iteration from whatever oracle you've chosen — `Instant::now()`
in production, a controlled value in tests — and treat it as a
memoizable input like every other source.

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Clock {
    pub now: Instant,
}

#[derive(drv::Input)]
struct ClockInput<'a> {
    pub now: &'a Instant,
}
```

Time-dependent queries take `ClockInput` alongside their other
inputs:

```rust
#[drv::memo(single)]
fn toast_visible<'a, 'b>(
    alerts: AlertsInput<'a>,
    clock: ClockInput<'b>,
) -> Option<&'a str> {
    match alerts.info_expires_at {
        Some(t) if *clock.now < t => Some(alerts.info_message.as_str()),
        _ => None,
    }
}
```

Two payoffs:

1. **Tests drive faster than realtime.** A test that exercises a
   30-second timeout takes microseconds: write `clock.now = t0 + 30s`
   into the source, rerun the query, assert. No `tokio::time::pause`
   ceremony, no wall-clock sleeps, no flakiness — and the same
   pattern handles simulated years without taking any longer.

2. **Cache cost is proportional to time-dependent work.** Memos that
   read clock recompute every iteration — their input changed by
   definition. Memos that don't read clock are unaffected. The
   recompute itself is cheap; downstream consumers only re-fire when
   the output value actually changes (e.g., when the toast crosses
   its expiry).

### Deadlines are queries the runner consults

A wall-clock deadline (toast expiry, animation frame, debounce flush,
LSP timeout) is just a field on a source. The runner doesn't need a
separate timer system — it reads a memo that folds all current
deadlines and uses the nearest one to size its sleep:

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

This is the same kind of memo as everything else. The runner consults
it to decide *how long to sleep*; it carries no semantic load. The
question "is the toast still visible?" is answered by a clock-aware
query (`toast_visible` above), not by the deadline — the fold's only
job is to keep the loop from oversleeping past a state transition.

Adding a feature with a time-bound is two steps: add the deadline
field to its source, add it to the fold.

### Fire-and-forget timers via a driver

Some timers don't fit on a source as state — "auto-save 5s after the
last keystroke," "retry the connection in 30s." For those, a small
`driver-timer` works well: it accepts `Set { name, duration,
schedule }` commands and posts back `Fired { name }` events. Ingest
writes the fire into the appropriate source field (e.g., flipping
`auto_save.pending = true`); queries observe the field.

This composes with clock-on-source rather than replacing it. The
driver is the *trigger*; the source field it mutates is what queries
read. Reach for it when the timer is genuinely a scheduled
side-effect rather than an attribute of state.

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

#[derive(drv::Input)]
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
