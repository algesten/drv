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
3. [Atoms: two kinds of ground truth](#atoms-two-kinds-of-ground-truth)
4. [Drivers: the sync/async split](#drivers-the-syncasync-split)
5. [Queries: desired state, not transitions](#queries-desired-state-not-transitions)
6. [Actions as diffs](#actions-as-diffs)
7. [The main loop](#the-main-loop)
8. [Bridging to a reactive UI framework (SwiftUI)](#bridging-to-a-reactive-ui-framework-swiftui)
9. [Threading model](#threading-model)
10. [Worked example: microphone lifecycle](#worked-example-microphone-lifecycle)
11. [Worked example: remote participants](#worked-example-remote-participants)
12. [Testing](#testing)
13. [Guidelines](#guidelines)

---

## Core idea

The application is structured as a **database of ground truth** (atoms) and
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

## Atoms: two kinds of ground truth

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
They should live in **separate atoms**.

```rust
// External fact — managed by the device enumeration driver
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MicInventory {
    pub permission: MicPermission,     // NotAsked, Pending, Granted, Denied
    pub mics: imbl::Vector<MicDevice>, // empty until Granted
}

// User decision — managed by UI event handlers
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MicPreference {
    pub selected: Option<MicId>,
}
```

At runtime each atom is held inside a `drv::Atom<T>` wrapper (via
`Atom::new(MyAtom { ... })`); the wrapper owns the per-instance memoization
cache. Drivers and the main loop pass `&mut Atom<T>` around, and field
access (`atom.field`) goes through `Deref`/`DerefMut` to the inner data.

Why separate? The device driver should not need to know about user
preferences. The UI handler should not need to know about OS enumeration.
Memos bridge them:

```rust
#[drv::memo]
fn mic_picker(inv: &MicListLens, pref: &MicSelectLens) -> MicPickerModel {
    // lenses from two atoms — only the fields this memo reads
}
```

### Organizing atoms by domain

Group atoms into domains. Each domain covers one area of the application:

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

Drivers manage external-fact atoms. UI handlers manage user-decision atoms.
Memos reach across domains freely via multi-lens parameters.

---

## Drivers: the sync/async split

A driver manages one external-fact atom. Every driver has two sides:

### The synchronous side: the atom

The atom is the queryable truth. It lives on the main thread (or whichever
thread owns the atoms). It is always available for reading via lenses.
Queries never block.

### The asynchronous side: platform I/O

Hardware access, network calls, OS dialogs — these run on background threads
or async tasks. When they complete, they post results back to the main thread,
which writes them into the atom.

```
Background threads                        Main thread
(expensive I/O)                           (cheap: atoms + queries)

mic_driver ──────────┐
camera_driver ───────┤  atom updates
signaling_driver ────┼──────────────────→  process_updates(&mut atoms)
webrtc_driver ───────┤
hotplug_listener ────┘
```

### Driver structure

```rust
/// Manages the MicInventory atom.
pub struct MicEnumerationDriver {
    /// Receives updates from the async enumeration task.
    rx: mpsc::Receiver<MicEvent>,
    /// Handle to request permission, start/stop enumeration, etc.
    platform: PlatformMicEnumerator,
}

impl MicEnumerationDriver {
    /// Called on the main thread each tick. Drains pending events
    /// into the atom. Cheap — just field assignments.
    pub fn process(&self, inv: &mut drv::Atom<MicInventory>) {
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
    pub fn request_permission(&self, inv: &mut drv::Atom<MicInventory>) {
        inv.permission = MicPermission::Pending;
        self.platform.request_permission_async();
    }
}
```

### The execute pattern

Some drivers also need to act on query results. For example, the mic stream
driver opens/closes streams based on the diff between desired and actual
state. The execute method does two things atomically:

1. Writes the intent into the atom (sync — prevents re-triggering)
2. Starts the async work (async — completes later)

```rust
pub struct MicStreamDriver {
    platform: PlatformAudioCapture,
}

impl MicStreamDriver {
    pub fn execute(&self, action: MicAction, cap: &mut drv::Atom<MicCapture>) {
        match action {
            MicAction::Open(id) => {
                // 1. Sync: update atom immediately
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

The synchronous write into the atom is critical. Without it, the next query
would return the same action again (because the atom hasn't changed yet),
causing a double-open. The atom write closes the loop: execute → atom
updates → query returns Noop → no re-execution.

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
/// Multi-lens memo: lenses from two atoms, only the fields that matter.
#[drv::memo]
fn desired_mic(inv: &MicListLens, pref: &MicSelectLens) -> Option<MicId> {
    match (*inv.permission, pref.selected) {
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

A query describes what should be true. The driver's atom describes what is
actually true. The diff is the action.

```rust
/// "What should we do about the mic stream?"
#[drv::memo]
fn mic_action(
    inv: &MicListLens,
    pref: &MicSelectLens,
    cap: &MicCapStateLens,
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
    // Drain async driver results into atoms.
    // Drain UI events into user-decision atoms.
    mic_enum_driver.process(&mut mic_inventory);
    mic_stream_driver.process(&mut mic_capture);
    camera_driver.process(&mut camera_inventory);
    signaling_driver.process(&mut call_session);
    webrtc_driver.process(&mut remote_participants, &mut remote_streams);
    ui_events.process(&mut mic_preference, &mut participant_settings, ...);

    // ── Phase 2: Query ───────────────────────────────────────
    // Ask questions. All reads, no writes. Memoized — mostly cache hits.
    let mic_act = mic_action(&mic_inventory, &mic_preference, &mic_capture);
    let cam_act = camera_action(&camera_inventory, &camera_preference, &camera_capture);

    // ── Phase 3: Execute ─────────────────────────────────────
    // Act on query results. Writes intent into atoms + starts async work.
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
- **Ingest**: writes to atoms from external sources. No reads.
- **Query**: reads from atoms. No writes. All memoized.
- **Execute**: writes intent to atoms + starts async work.
- **Render**: reads from atoms. Pushes to UI. All memoized.

---

## Bridging to a reactive UI framework (SwiftUI)

SwiftUI is reactive — it re-renders views when `@Observable` properties
change. `drv` is query-driven — it returns cached values when you ask.
The bridge converts pull into push.

### One memo per view

Each view has a corresponding memo that produces a view-specific model:

```rust
#[drv::memo]
fn mic_picker_model(inv: &MicListLens, pref: &MicSelectLens) -> MicPickerModel {
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

// Lenses for participant grid — only the fields this view needs.
// Standalone lenses borrow non-Copy fields as `&'a T` (only built-in
// Copy primitives may be stored by value), so the struct needs a lifetime.
#[drv::lens(RemoteParticipants)]
struct GridParticipantsLens<'a> {
    pub participants: &'a imbl::Vector<Participant>,
}

#[drv::lens(ParticipantSettings)]
struct GridSettingsLens<'a> {
    pub muted: &'a imbl::HashSet<ParticipantId>,
    pub pinned: &'a Option<ParticipantId>,
}

#[drv::memo]
fn participant_grid_model(
    p: &GridParticipantsLens,
    s: &GridSettingsLens,
    m: &RemoteStreams,  // identity lens ok if all fields are relevant
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
| `drv` lens | Field-by-field `PartialEq` on projected fields | Recomputing memo when irrelevant atom fields changed |
| `PartialEq` bridge | `new_model != current_model` | Pushing to Swift when the model is identical |
| SwiftUI `@Observable` | Per-property tracking | Re-rendering views that didn't read the changed property |

### User events flow back

SwiftUI user actions cross the FFI boundary in the opposite direction and
mutate user-decision atoms:

```
SwiftUI onTapGesture
  → RustBridge.selectMic(id)      // FFI call
    → mic_preference.selected = Some(id)  // atom mutation
      → next tick: queries recompute, bridge pushes if changed
```

---

## Threading model

```
Background threads                        UI / Main thread
(async I/O, platform calls)              (atoms, queries, render)

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

### Why atoms stay on the UI thread

- The cache inside each `drv::Atom<T>` wrapper uses `RefCell` (not `Mutex`)
  — fast, no contention, but makes `Atom<T>` `!Sync` (it is still `Send`
  whenever `T: Send`).
- Query/memo computation is cheap (cache hits ~100ns). Hundreds of queries
  per frame is < 1ms.
- The expensive work (I/O, network, audio processing) is already on
  background threads.
- Keeping atoms on the UI thread means views can lens directly — no
  thread-boundary serialization for read access.

### Communication pattern

Background threads communicate with the main thread via channels
(`mpsc::Receiver`, `crossbeam::Receiver`, etc.) or callback dispatch
(e.g., `DispatchQueue.main.async` on iOS).

The main loop's **ingest phase** drains these channels into atoms. This is
the only point where external data enters the query system.

---

## Worked example: microphone lifecycle

A complete walkthrough of the mic selection flow, showing atoms, mutations,
queries, and actions at each step.

### Atoms

A lens projects from a single atom. To combine fields from multiple atoms,
use multi-lens memos — one parameter per atom or per lens. Always use
lenses that project only the fields the memo actually reads. This way, changes
to unrelated fields (e.g., `last_enumerated` timestamp updating frequently)
don't cause unnecessary recomputation.

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MicInventory {
    #[drv::lens(MicListLens)]
    pub permission: MicPermission,

    #[drv::lens(MicListLens)]
    pub mics: imbl::Vector<MicDevice>,

    pub last_enumerated: Option<Instant>,   // updated frequently, most memos don't care
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MicPreference {
    #[drv::lens(MicSelectLens)]
    pub selected: Option<MicId>,

    pub last_changed: Option<Instant>,      // for analytics, not for queries
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MicCapture {
    #[drv::lens(MicCapStateLens)]
    pub state: CaptureState,

    #[drv::lens(MicCapStateLens)]
    pub device_id: Option<MicId>,

    pub volume_level: f32,                  // updated every audio frame, mic_action doesn't need it
}
```

### Queries

Each memo takes lens references that project only the fields it needs.
Changes to fields outside the lens (e.g., `volume_level` updating 60 times
per second) don't trigger recomputation of unrelated memos.

```rust
/// "What mic should be open right now?"
/// Needs permission + available mics + user selection. Does NOT need
/// last_enumerated, last_changed, or volume_level.
#[drv::memo]
fn desired_mic(inv: &MicListLens, pref: &MicSelectLens) -> Option<MicId> {
    match (*inv.permission, pref.selected) {
        (MicPermission::Granted, Some(id))
            if inv.mics.iter().any(|m| m.id == *id) => Some(*id),
        _ => None,
    }
}

/// "What should we do about the mic stream?"
/// Needs desired state + capture state. Does NOT need volume_level.
#[drv::memo]
fn mic_action(
    inv: &MicListLens,
    pref: &MicSelectLens,
    cap: &MicCapStateLens,
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
#[drv::memo]
fn mic_picker_model(inv: &MicListLens, pref: &MicSelectLens) -> MicPickerModel {
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
#[drv::lens(MicCapture)]
struct MicVolumeLens {
    pub volume_level: f32,
}

#[drv::memo]
fn mic_volume_display(vol: &MicVolumeLens) -> VolumeLevel {
    VolumeLevel::from_linear(*vol.volume_level)
}
```

### Step-by-step flow

```
Step  What happens                 Atom mutations                   Query results
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

### Atoms

```rust
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct RemoteParticipants {
    pub participants: imbl::Vector<Participant>,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct ParticipantSettings {
    pub muted: imbl::HashSet<ParticipantId>,
    pub pinned: Option<ParticipantId>,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct RemoteStreams {
    pub video_tracks: imbl::HashMap<ParticipantId, VideoTrackState>,
    pub audio_tracks: imbl::HashMap<ParticipantId, AudioTrackState>,
}
```

### Queries

```rust
#[drv::memo]
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
because it's cleaning up a user-decision atom in response to an external
fact changing.

---

## Testing

Query-driven architecture makes testing straightforward because queries are
pure functions.

### Unit testing a query

```rust
use drv::Atom;

#[test]
fn desired_mic_requires_permission_and_availability() {
    let mut inv = Atom::new(MicInventory::default());
    let mut pref = Atom::new(MicPreference::default());

    // No permission → no desired mic
    pref.selected = Some(MicId(1));
    assert_eq!(desired_mic(&inv, &pref), None);

    // Permission but mic not in list → no desired mic
    inv.permission = MicPermission::Granted;
    assert_eq!(desired_mic(&inv, &pref), None);

    // Permission and mic available → desired
    inv.mics.push_back(MicDevice { id: MicId(1), name: "USB Mic".into() });
    assert_eq!(desired_mic(&inv, &pref), Some(MicId(1)));

    // Mic removed → no desired mic (reconnect scenario)
    inv.mics.retain(|m| m.id != MicId(1));
    assert_eq!(desired_mic(&inv, &pref), None);

    // Mic re-added → desired again (selection survived)
    inv.mics.push_back(MicDevice { id: MicId(1), name: "USB Mic".into() });
    assert_eq!(desired_mic(&inv, &pref), Some(MicId(1)));
}
```

### Testing the full action cycle

```rust
use drv::Atom;

#[test]
fn mic_action_produces_correct_diffs() {
    let mut inv = Atom::new(MicInventory {
        permission: MicPermission::Granted,
        ..Default::default()
    });
    let mut pref = Atom::new(MicPreference::default());
    let mut cap = Atom::new(MicCapture::default());

    inv.mics.push_back(mic(1));
    inv.mics.push_back(mic(2));

    // Select mic1 → should open
    pref.selected = Some(MicId(1));
    assert_eq!(mic_action(&inv, &pref, &cap), MicAction::Open(MicId(1)));

    // Simulate execute: cap reflects Opening
    cap.state = CaptureState::Opening;
    cap.device_id = Some(MicId(1));
    assert_eq!(mic_action(&inv, &pref, &cap), MicAction::Noop);

    // Stream live
    cap.state = CaptureState::Live;
    assert_eq!(mic_action(&inv, &pref, &cap), MicAction::Noop);

    // Switch to mic2
    pref.selected = Some(MicId(2));
    assert_eq!(mic_action(&inv, &pref, &cap), MicAction::Switch(MicId(2)));

    // Deselect
    pref.selected = None;
    assert_eq!(mic_action(&inv, &pref, &cap), MicAction::Close);
}
```

No mocking. No async runtime. No event loop setup. Just pure functions with
known inputs and expected outputs.

---

## Guidelines

### 1. External facts and user decisions go in separate atoms

Don't mix "what the OS told me" with "what the user chose" in the same atom.
Drivers manage external facts. UI handlers manage user decisions. Memos
combine them.

### 2. Queries describe desired state, not transitions

Write "what mic should be open?" not "when the user selects a mic, open it."
The diff between desired and actual is the action.

### 3. Execute writes intent synchronously

When acting on a query result, update the driver's atom to reflect the
in-progress operation *before* starting the async work. This prevents the
query from returning the same action on the next tick.

### 4. The main loop is ingest → query → execute → render

Keep the phases separate. Ingest writes to atoms from external sources.
Query reads atoms (memoized). Execute acts on query results and writes
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
selected mic removed), clean up the user-decision atom during ingest. This
is application logic, not a query.

### 8. Keep drivers ignorant of each other

The mic enumeration driver should not know about the stream driver. The
stream driver should not know about user preferences. Memos express the
relationships between domains. Drivers just keep their atom in sync with
reality.
