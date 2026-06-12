# Architecture: Query-Driven Applications with `drv`

This document describes query-driven application architecture with `drv`.
The running example is a mobile teleconferencing app, but the patterns apply
to any stateful app with derived views and asynchronous I/O.

The goal: an assistant (human or AI) should be able to read this document
together with the `drv` crate documentation and structure a new application
from scratch.

## Core idea

The application is a **database of ground truth** (sources) plus **cached
queries** over that database (memos). Nothing watches or subscribes. The main
loop sets inputs, asks questions, and acts on answers. Memoization makes the
asking cheap.

```
set inputs → ask questions → act on answers → repeat
```

This is the same model used by rust-analyzer (via Salsa) and incremental
build systems. `drv` provides the memoization layer; this document describes
the application architecture built on top of it.

---

## Query-driven vs reactive

Reactive architectures push notifications to observers: "when X changes, do
Y." Behavior is the sum of distributed, implicitly ordered handlers.

Query-driven architectures pull query results: "given the current state, what
should be true?" Results are cached and recomputed only when inputs change.
Each query's behavior is centralized, explicit, and testable.

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

External facts and user decisions have different lifecycles, update paths, and
owners. Keep them in **separate sources**.

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

Sources are plain Rust structs. `drv` doesn't wrap them; memoization lives in
per-memo thread-local caches keyed by input value. Drivers and the main loop
pass `&mut MySource` around and mutate fields directly.

Drivers should not know about user preferences, and UI handlers should not
know about OS enumeration. Memos bridge sources via **inputs**: projections
declared with `#[derive(drv::Input)]`.

```rust
#[drv::memo(single)]
fn mic_picker<'a, 'b>(inv: MicListInput<'a>, pref: MicSelectInput<'b>) -> MicPickerModel {
    // inputs from two sources — only the fields this memo reads
}
```

### Organizing sources by domain

Group sources by domain:

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

Drivers manage external facts. UI handlers manage user decisions. Memos reach
across domains via multi-input parameters.

### User-decision sources keyed by external facts

Do not merge decisions with external facts just because a decision is keyed by
ids the driver discovered: muted participants by signaling id, expanded
directories by filesystem path, pinned inventory entries by item id.

The user decision is the **key set**; the external fact is the **map values**.
User choices can persist across reconnects while enumeration rebuilds. Dispatch
mutates decisions; drivers mutate facts. Keep them separate and combine them in
memos.

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

Sharing an identifier type is reference, not coupling. Neither source needs to
know about the other; a memo reads both and returns the combined shape.

### Shadow sources

Sometimes a user-decision source holds a *mutation* of an external-fact source:
an edited buffer shadows the file on disk, a form shadows a server record, or a
draft shadows a synced document.

Use two sources with a shared key (path, record id, document id), one seeded
from the other. Document the seeding protocol:

- **Seed on first completion.** When the external source first becomes
  `Ready` for a key, ingest copies it into the user source.
- **Subsequent external updates don't auto-apply.** If the disk file changes
  after editing starts, queue the new version as a reload candidate.
- **Writes are explicit.** The user source flows *out* to the external
  source only via a deliberate save action, with explicit conflict handling.

When both roles share an underlying type, newtype them so the compiler carries
the invariant:

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

The sync write is critical: without it, the next query sees unchanged source
state and returns the same action again. Execute writes intent, the query
returns `Noop`, and work is not re-triggered.

### Pure output drivers

Some drivers only push computed state out to a platform: paint a terminal
frame, render a graphics surface, play a sound buffer, flash an LED. They
still follow the execute pattern. Their source is the **in-flight artifact**
plus acknowledgement state:

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

This is the same "write intent sync, fire async, diff is `Noop` until done"
pattern. Without a source, paint becomes an unmockable free function that
cannot be throttled or traced like other drivers.

### Stateless drivers still need an in-flight source

A wrapper around a platform API — clipboard, shell command, DNS lookup — may
have no persistent external fact, but it still has in-flight state. Keep that
state on a **driver-owned source**, not on the user-decision source that
triggered the operation:

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

Otherwise user-decision sources accumulate async bookkeeping, `PartialEq` sees
driver-completion churn, and "chosen, not discovered" stops being true.

---

## Queries: desired state, not transitions

The fundamental unit of logic is a `drv` memo that describes what should be
true given the current inputs.

### Anti-pattern: transition handlers

```
// DON'T: one handler per transition
on_mic_selected(id)     → open_stream(id)
on_mic_disconnected(id) → if selected, set_reconnect_flag(id)
on_mic_reconnected(id)  → if reconnect_flag, reopen_stream(id)
on_permission_revoked() → close_stream(), clear_selection()
on_switch_mic(old, new) → close_stream(old), open_stream(new)
```

Each handler encodes one transition. Miss one and you have a bug.

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

The query describes the desired end state; the diff against actual state
determines the action.

---

## Actions as diffs

A query describes desired state. A driver source describes actual state. The
diff is the action.

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

`Opening` counts as actual because an open is already in flight; this prevents
duplicate opens.

### Anti-pattern: derived UI state

The UI layer is a driver: it should *render* runtime queries, not
*re-derive* them. A boolean or enum that combines runtime fields over FFI, a
channel, or another narrow surface is a query in the wrong language.

```kotlin
// ANTI-PATTERN: query in Kotlin
val connected = handle != 0L && (exit == null || exit == Running)
```

```swift
// SAME ANTI-PATTERN: query in Swift
let connected = runtime.lastExit == nil && runtime.isAlive
```

The runtime crate has the sources, so the combination rule belongs there.
Write the rule as a memo, expose one value across the boundary, and let the UI
read it.

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

Adding inputs later is one memo change and no UI rewrites.

#### How to spot it on review

- `&&` or `||` over runtime-observed fields belongs in a memo.
- Similar `if` / `when` ladders across platform UIs belong in a memo.
- `onChange` / `LaunchedEffect` / `Observer` used to sync derived state is a
  memo with extra steps.

UI lifecycle (handle allocation, async resource freeing) is UI-side driver
work. Derivation from runtime-observed facts belongs in runtime memos.

### Anti-pattern: the view model assembled across the FFI

The previous anti-pattern is about *deriving* a value UI-side. This one is
about *transport*: even when every field is a faithful memo read, **pulling
the view model across the boundary one field at a time, and re-declaring its
shape once per platform, is the same mistake one level up.**

The render layer should read *one parked view model per change*, not call N
narrow accessors per frame. The shape lives in the runtime, defined once; each
UI decodes that one shape. Two tells that it has gone wrong:

```swift
// ANTI-PATTERN: the view assembles the model by calling N getters at render
var remoteTiles: [RemoteTile] {
    let count = runtime_remote_tile_count(rt)
    return (0..<count).map { i in
        RemoteTile(
            id:        readString { runtime_remote_tile_id_at(rt, i, $0, $1) },
            hue:       Int(runtime_remote_tile_hue_at(rt, i)),
            micMuted:  runtime_remote_tile_mic_muted_at(rt, i),
            videoOff:  runtime_remote_tile_video_off_at(rt, i),
            // …8 more per-field calls per tile, every render…
        )
    }
}
```

```kotlin
// SAME ANTI-PATTERN, re-declared a second time in the other UI:
data class RemoteTile(val id: String, val hue: Int, val micMuted: Boolean, …)
fun snapshotRemoteTiles(h: Long) = (0 until Native.remoteTileCount(h)).map { i ->
    RemoteTile(Native.remoteTileIdAt(h, i), Native.remoteTileHueAt(h, i), …)
}
```

Three costs compound:

- **The shape is duplicated per platform.** `RemoteTile` exists in Swift *and*
  in Kotlin, hand-kept in sync, drifting field-by-field (one platform grows a
  20th getter the other lacks). The "single source of truth" stops at the memo
  output and forks at the boundary.
- **The transport is O(fields × items) FFI calls per render.** A 50-tile grid
  is hundreds of boundary crossings every frame, each a separate
  length-query-then-copy dance for strings.
- **It invites a fixed-cadence poll.** Because no single signal says "the model
  changed," the UI re-pulls everything on a timer — which is the *spin* the
  "wake on event" rule forbids, now mass-multiplied: every poll re-allocates
  the whole graph whether or not anything moved.

Corrective shape: **the runtime owns one view-model type, a memo composes it
from the per-field memos, and exactly one value crosses the boundary per
change.** Serialize it once (any self-describing format both sides already
decode — JSON, MessagePack), park it, signal "new model," let each UI decode
the one shape into its render structs.

```rust
// One type, defined once, in the runtime crate. The memos above feed it.
#[derive(Serialize, Clone, PartialEq)]
pub struct InCallViewModel {
    pub connection_phase: ConnectionPhase,
    pub self_tile:        SelfTile,
    pub remote_tiles:     Vec<RemoteTile>,
    pub screen_share:     Option<ScreenShare>,
    // …the whole drawable model, in one place…
}

#[drv::memo(single)]
pub fn in_call_view_model(/* the source inputs */) -> InCallViewModel {
    InCallViewModel {
        connection_phase: connection_phase(/* … */),
        self_tile:        self_participant(/* … */),
        remote_tiles:     remote_participants(/* … */),
        screen_share:     active_screen_share(/* … */),
    }
}
```

```swift
// One boundary call per change; decode the one shared shape.
struct InCallViewModel: Decodable { /* mirrors the Rust struct */ }
let vm = try JSONDecoder().decode(InCallViewModel.self, from: parkedBytes)
```

The memo is the "view compiler": when a driver pushes a fact, only the
sub-memos downstream of it recompute, so composing the whole model per change
is still cheap. The model is parked in shared memory; the main loop signals the
UI once; the UI renders the parked value. That is the same **set inputs → ask
questions → act on answers** loop, with "render" reading a single answer
instead of interrogating the database field by field.

#### How to spot it on review

- A `*_count()` accessor paired with `*_at(i)` per-field getters — the model is
  being walked across the boundary by hand.
- The same `data class` / `struct` view-model shape declared in both the Swift
  and Kotlin (or other per-platform) source trees.
- The UI re-reads the model on a timer rather than on a change signal.

---

## The main loop

"Main loop" means the runtime thread that owns sources and runs
ingest/query/execute/render. It is not SwiftUI's main thread, Android's UI
thread, or a terminal render thread; those are UI drivers.

Every iteration:

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

Phase boundaries stay strict:
- **Ingest** writes outside events into sources.
- **Query** reads sources only.
- **Execute** writes intent into sources and starts async work.
- **Render** reads memoized view models and pushes UI updates.

### Wake on event, don't spin

Block until a driver completion, user input, or deadline. **Never sleep a
fixed duration "just in case."** Fixed ticks burn idle CPU and add latency
when an event arrives just after sleep starts.

The specific primitive depends on how the drivers are wired:

**Sync drivers (`std::mpsc`).** The main thread owns a `Receiver<()>`; drivers
and input sources hold cloned `Sender`s. The loop blocks on
`recv_timeout(timeout)`, where `timeout` comes from the nearest-deadline query.

```rust
let wake = Wake::new();
let drivers = spawn_drivers(trace, &wake)?;

loop {
    // ingest → query → execute → render

    let timeout = nearest_deadline(...)
        .and_then(|d| d.checked_duration_since(Instant::now()))
        .unwrap_or(Duration::from_secs(60));
    match wake.rx.recv_timeout(timeout) { /* ... */ }

    // Drain after recv_timeout; a pre-sleep drain can swallow a wake
    // signaled by work started during execute.
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
(`Instant::now()` in production, controlled value in tests), and pass it as a
memoizable input.

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

Time-dependent queries take `ClockInput` with their other inputs:

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

Payoffs:

1. **Tests drive faster than realtime.** Set `clock.now = t0 + 30s`, rerun the
   query, assert. No wall-clock sleeps or timer-runtime ceremony.
2. **Cache cost is local.** Only memos that read clock recompute every
   iteration; consumers only re-fire when outputs change.

### Deadlines are queries the runner consults

A wall-clock deadline (toast expiry, animation frame, debounce flush, LSP
timeout) is just a source field. The runner reads a memo that folds current
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

The deadline fold carries no semantic load; `toast_visible` still answers
whether the toast is visible. The fold only keeps the loop from oversleeping
past a state transition. Adding a time-bound feature means adding a deadline
field to its source and to this fold.

### Fire-and-forget timers via a driver

For timers like "auto-save 5s after the last keystroke" or "retry in 30s",
use a small `driver-timer`: it accepts `Set { name, duration, schedule }` and
posts `Fired { name }`. Ingest writes that fire into a source field, and memos
observe the field. The driver is the trigger; the source remains the truth.

---

## Organizing the code: crate layout

The runtime model needs a crate layout that scales across domains and
platforms.

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

The runtime can pick natives with target-specific Cargo dependencies when
`spawn_drivers` has the same shape on every platform:

```toml
[target.'cfg(all(not(target_os = "ios"), not(target_os = "android")))'.dependencies]
my-app-driver-buffers-native-desktop = { workspace = true }

[target.'cfg(target_os = "ios")'.dependencies]
my-app-driver-buffers-native-ios = { workspace = true }

[target.'cfg(target_os = "android")'.dependencies]
my-app-driver-buffers-native-android = { workspace = true }
```

If runtime wiring differs substantially, especially because the UI driver
differs, use **parallel runtime crates** instead:

```
crates/
  runtime-desktop/   wires terminal UI driver + desktop workers
  runtime-ios/       wires SwiftUI bridge + iOS workers
  runtime-android/   wires Jetpack Compose bridge + Android workers
```

Each runtime crate depends on shared `*-core` crates plus the right
`*-native-<platform>` crates.

### User-decision sources have no driver

A user-decision source is chosen, not discovered: no async side, worker, or
driver struct. Put it in a `state-*` crate and mutate it from runtime dispatch.
The driver shape is for units with an async peer.

### Cross-driver composition lives in a runtime crate

**Drivers must not know about each other.** `driver-foo/core/` must not import
`driver-bar/core/` or `state-baz/`. Cross-source memos live in a **runtime**
crate that depends on all sources. The runtime also owns:

- `Event` + `dispatch` (the user-event handler that mutates driver sources).
- `Trace` / unified trace sink that driver traces forward to.
- The main loop `run` itself.
- The platform-specific wiring that spawns native workers and builds
  the `Drivers` bundle.

Cross-platform payoff: targets can replace runtime crates around the same
`*-core` drivers.

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

Some drivers wrap **platform primitives** used by multiple domain drivers: UDP,
HTTP, timers. These are infrastructure, not domain state.

They sit **below** domain drivers, and imports into them are expected. A domain
driver importing `platform-http-core`'s `Cmd` / `Event` ABI is how it speaks
HTTP. Use the `platform-*` prefix so the dependency exception is explicit:

- `state-*` depends on nothing (or only on shared primitive crates).
- `platform-*` may depend on `state-*` and shared primitives; not on
  `driver-*`.
- `driver-*` may depend on `platform-*` and on `state-*`; must not
  depend on other `driver-*`.
- `runtime` depends on all of them.

**Not every cross-cutting concern is a platform driver.** The criterion is
*multiple domain drivers actively call into it*, not "many app parts observe
it." A lifecycle driver read by runtime memos is a regular `driver-*`.

Platform drivers are usually domain-stateless but **multi-request**: their
source is typically `HashMap<RequestId, InFlight>` keyed by caller id.

### Cross-crate inputs: the consumer declares its own

Inputs live in the crate that uses them. A source crate exports a plain struct
and usually has no `drv` dependency. The consumer declares the input,
projection, and memo:

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

No registry, no cross-crate coordination.

`#[derive(drv::Input)]` alone is cache-key / `ToStatic` plumbing, not a memo.
A source crate may derive it on plain value types when needed for
consumer-owned snapshots (for example `imbl::Vector<T>` where `T` needs
`ToStatic`). That does **not** move query ownership into the source crate:
avoid consumer-specific constructors, `#[drv::memo]`, and cross-source logic
there.

```rust
// In state-room: plain value type; derive is cache-key plumbing only.
#[derive(Debug, Clone, PartialEq, Eq, drv::Input)]
pub struct Peer {
    pub peer_id: String,
    pub name: String,
}

// In runtime-common: consumer-owned projection + memo.
#[derive(drv::Input)]
pub struct RoomPeersInput<'a> {
    pub peers: &'a imbl::Vector<Peer>,
}

impl<'a> RoomPeersInput<'a> {
    pub fn new(room: &'a Room) -> Self {
        Self { peers: &room.peers }
    }
}

#[drv::memo(single)]
pub fn remote_participants(input: RoomPeersInput<'_>) -> Vec<RemoteTile> {
    todo!()
}
```

### Input construction does no work

`Input::new` is a **projection**, not a computation. It selects fields and
stops there: no filtering, summing, mapping, or other derivation.

```rust
// ANTI-PATTERN: aggregation inside Input::new
impl<'a> DesiredBitrateInput<'a> {
    pub fn new(encoders: &'a Encoders) -> Self {
        let sum_bps = encoders.by_id.values()
            .filter(|s| matches!(s.status, EncoderStatus::Opening | EncoderStatus::Active))
            .map(|s| s.params.bitrate.bps())
            .sum();
        Self { encoders_sum_bps: sum_bps, _p: PhantomData }
    }
}
```

```rust
// CORRECTIVE SHAPE: project the field, let a memo do the work
#[derive(drv::Input)]
struct EncodersInput<'a> {
    pub by_id: &'a imbl::HashMap<EncoderId, EncoderState>,
}

#[drv::memo(single)]
fn desired_bitrate_bps<'a>(enc: EncodersInput<'a>) -> u64 {
    enc.by_id.values()
        .filter(|s| matches!(s.status, EncoderStatus::Opening | EncoderStatus::Active))
        .map(|s| s.params.bitrate.bps())
        .sum()
}
```

Work inside `Input::new` runs **every iteration**, uncached. Moving derivation
upstream of the cache also makes broad derived values like `sum_bps` churn and
recompute downstream memos that needed narrower dependencies.

Rule: **Inputs project; memos compute.** A `filter` / `map` / `sum` / `fold` /
`match` inside `Input::new` is a memo trying to escape.

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
