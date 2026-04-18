use imbl::HashMap as ImHashMap;
use imbl::Vector as ImVector;

// ══════════════════════════════════════════════════════════════════════
// 1. INLINE LENSES — basic atom with field annotations
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Editor {
    #[drv::lens(VisibleLines)]
    pub scroll_row: u32,

    #[drv::lens(VisibleLines)]
    pub viewport_rows: u32,

    pub cursor_row: u32,
    pub cursor_col: u32,

    #[drv::lens(VisibleLines, TabList)]
    pub content: ImVector<String>,

    #[drv::lens(TabList)]
    pub tabs: ImVector<String>,
}

#[drv::memo(single)]
fn visible_lines<'a>(lens: impl Into<VisibleLines<'a>>) -> Vec<String> {
    lens.content
        .iter()
        .skip(lens.scroll_row as usize)
        .take(lens.viewport_rows as usize)
        .cloned()
        .collect()
}

#[drv::memo(single)]
fn tab_list<'a>(lens: impl Into<TabList<'a>>) -> Vec<String> {
    let mut out: Vec<String> = lens.tabs.iter().cloned().collect();
    out.push(format!("({} lines)", lens.content.len()));
    out
}

// ══════════════════════════════════════════════════════════════════════
// 2. STANDALONE LENS — separate struct with #[drv::lens] + // ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Dashboard {
    pub user_name: String,
    pub notification_count: u32,
    pub theme: String,
    pub items: ImVector<String>,
}

#[drv::lens]
struct NotificationLens<'a> {
    pub user_name: &'a String,
    pub notification_count: u32,
}

impl<'a> From<&'a Dashboard> for NotificationLens<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self {
            user_name: &d.user_name,
            notification_count: d.notification_count,
        }
    }
}

#[drv::memo(single)]
fn notification_badge<'a>(lens: impl Into<NotificationLens<'a>>) -> String {
    if lens.notification_count == 0 {
        format!("{}: no notifications", lens.user_name)
    } else {
        format!("{}: {} new", lens.user_name, lens.notification_count)
    }
}

#[drv::lens]
struct ItemsLens<'a> {
    pub items: &'a ImVector<String>,
}

impl<'a> From<&'a Dashboard> for ItemsLens<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self { items: &d.items }
    }
}

#[drv::memo(single)]
fn item_count<'a>(lens: impl Into<ItemsLens<'a>>) -> usize {
    lens.items.len()
}

// ══════════════════════════════════════════════════════════════════════
// 3. CHAINING — memo output as atom, feeding into another memo
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Summary {
    #[drv::lens(CountLens)]
    pub total: usize,

    pub label: String,
}

#[drv::memo(single)]
fn doubled_total<'a>(lens: impl Into<CountLens<'a>>) -> usize {
    lens.total * 2
}

// ══════════════════════════════════════════════════════════════════════
// 4. MULTIPLE LENSES ON SAME ATOM — different projections
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct GameState {
    #[drv::lens(PlayerLens)]
    pub player_x: f32,

    #[drv::lens(PlayerLens)]
    pub player_y: f32,

    #[drv::lens(ScoreLens)]
    pub score: u64,

    #[drv::lens(ScoreLens)]
    pub high_score: u64,

    pub frame_count: u64,
}

#[drv::memo(single)]
fn player_distance<'a>(lens: impl Into<PlayerLens<'a>>) -> f32 {
    (lens.player_x * lens.player_x + lens.player_y * lens.player_y).sqrt()
}

#[drv::memo(single)]
fn score_display<'a>(lens: impl Into<ScoreLens<'a>>) -> String {
    format!("{} / {}", lens.score, lens.high_score)
}

// ══════════════════════════════════════════════════════════════════════
// 5. IMBL HASHMAP — structural sharing with maps
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub struct BufferData {
    pub content: String,
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct BufferStore {
    #[drv::lens(AllBuffersLens)]
    pub buffers: ImHashMap<String, BufferData>,

    #[drv::lens(AllBuffersLens)]
    pub active: Option<String>,

    pub last_save: u64,
}

#[drv::memo(single)]
fn active_content<'a>(lens: impl Into<AllBuffersLens<'a>>) -> Option<String> {
    lens.active
        .as_ref()
        .and_then(|name| lens.buffers.get(name))
        .map(|b| b.content.clone())
}

// ══════════════════════════════════════════════════════════════════════
// 6. SINGLE-FIELD LENS — minimal case
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Counter {
    #[drv::lens(ValueLens)]
    pub value: i64,

    pub name: String,
}

#[drv::memo(single)]
fn is_positive<'a>(lens: impl Into<ValueLens<'a>>) -> bool {
    lens.value > 0
}

// ══════════════════════════════════════════════════════════════════════
// 7. MULTI-LENS — memo taking lenses from different atoms
// ══════════════════════════════════════════════════════════════════════

#[drv::lens]
struct EditorTabsLens<'a> {
    pub tabs: &'a ImVector<String>,
}

impl<'a> From<&'a Editor> for EditorTabsLens<'a> {
    fn from(e: &'a Editor) -> Self {
        Self { tabs: &e.tabs }
    }
}

#[drv::lens]
struct DashboardUserLens<'a> {
    pub user_name: &'a String,
}

impl<'a> From<&'a Dashboard> for DashboardUserLens<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self {
            user_name: &d.user_name,
        }
    }
}

#[drv::memo(single)]
fn combined_header<'a, 'b>(
    tabs: impl Into<EditorTabsLens<'a>>,
    user: impl Into<DashboardUserLens<'b>>,
) -> String {
    format!("{}: {} tabs open", user.user_name, tabs.tabs.len())
}

// ══════════════════════════════════════════════════════════════════════
// 8. TWO-LEVEL CHAINING — memo output is an atom feeding another memo
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct AppState {
    #[drv::lens(AppItemsLens)]
    pub items: ImVector<String>,

    #[drv::lens(AppItemsLens)]
    pub selected: Option<usize>,

    pub theme: String,
}

#[drv::memo(single)]
fn items_summary<'a>(lens: impl Into<AppItemsLens<'a>>) -> ItemsSummary {
    ItemsSummary {
        count: lens.items.len(),
        current: lens
            .selected
            .and_then(|i| lens.items.get(i).cloned())
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct ItemsSummary {
    #[drv::lens(ItemsSummaryLens)]
    pub count: usize,

    #[drv::lens(ItemsSummaryLens)]
    pub current: String,
}

#[drv::memo(single)]
fn summary_label<'a>(lens: impl Into<ItemsSummaryLens<'a>>) -> String {
    if lens.current.is_empty() {
        format!("{} items, none selected", lens.count)
    } else {
        format!("{} items, viewing: {}", lens.count, lens.current)
    }
}

// Atom used directly as memo input (identity lens — all fields).
#[drv::memo(single)]
fn summary_label_full(s: &ItemsSummary) -> String {
    format!("{}: {}", s.count, s.current)
}

// ══════════════════════════════════════════════════════════════════════
// 9. REENTRANCY — a memo body may invoke another memo on the same atom.
//    Each memo owns its own thread-local cache, so there's no shared
//    RefCell to double-borrow; the inner call simply runs (or hits the
//    inner memo's own cache).
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Reentrant {
    pub value: u32,
}

#[drv::memo(single)]
fn inner_memo(r: &Reentrant) -> u32 {
    r.value * 2
}

#[drv::memo(single)]
fn outer_memo(r: &Reentrant) -> u32 {
    inner_memo(r) + 1
}

// ══════════════════════════════════════════════════════════════════════
// 10. MIXED PARAMS — memos taking lens and value parameters together.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct MixedAtom {
    #[drv::lens(BaseLens)]
    pub base: u32,
}

// Value param AFTER a lens param.
#[drv::memo(single)]
fn with_value_after<'a>(lens: impl Into<BaseLens<'a>>, multiplier: u32) -> u32 {
    lens.base * multiplier
}

// Value param BEFORE a lens param — order must be preserved.
#[drv::memo(single)]
fn with_value_before<'a>(multiplier: u32, lens: impl Into<BaseLens<'a>>) -> u32 {
    multiplier * lens.base
}

// Multiple value params mixed with a lens.
#[drv::memo(single)]
fn multi_value<'a>(offset: u32, lens: impl Into<BaseLens<'a>>, scale: u32) -> u32 {
    offset + (lens.base * scale)
}

// Reference value params — `&str` and `&[u8]` stored via ToOwned.
#[drv::memo(single)]
fn with_str<'a>(lens: impl Into<BaseLens<'a>>, prefix: &str) -> String {
    format!("{}={}", prefix, lens.base)
}

#[drv::memo(single)]
fn with_bytes<'a>(lens: impl Into<BaseLens<'a>>, bytes: &[u8]) -> usize {
    (lens.base as usize) + bytes.len()
}

// ══════════════════════════════════════════════════════════════════════
// 11. FastEq ptr_eq — Arc<T> field should short-circuit equality via
//     Arc::ptr_eq, so cloning the atom and calling the memo again hits
//     the fast path even when T: !PartialEq would otherwise be O(n).
// ══════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct ArcAtom {
    #[drv::lens(ArcLens)]
    pub data: Arc<Vec<u32>>,
}

static ARC_MEMO_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn arc_sum<'a>(lens: impl Into<ArcLens<'a>>) -> u32 {
    ARC_MEMO_COMPUTES.fetch_add(1, Ordering::SeqCst);
    lens.data.iter().sum()
}

// ══════════════════════════════════════════════════════════════════════
// 12. FACTORY LENSES — user-defined From impl with arbitrary field
//     names/types and nested struct access.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inner {
    pub value: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct Outer {
    pub inner: Inner,
    pub name: String,
    pub count: u32,
}

// Exercises owned and reference fields side-by-side in a single lens.
#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct CopyTest {
    pub x: u32,
    pub y: u32,
    pub label: String,
}

// x is stored by value (u32 copy), y is borrowed as &'a u32.
#[drv::lens]
struct CopyMixLens<'a> {
    pub x: u32,
    pub y: &'a u32,
}

impl<'a> From<&'a CopyTest> for CopyMixLens<'a> {
    fn from(c: &'a CopyTest) -> Self {
        Self { x: c.x, y: &c.y }
    }
}

#[drv::memo(single)]
fn copy_mix_sum<'a>(lens: impl Into<CopyMixLens<'a>>) -> u32 {
    // x is u32 (no deref needed), y is &u32 (deref needed)
    lens.x + *lens.y
}

// Projection lens: field names differ, types differ, reaches into nested struct.
#[drv::lens]
struct ProjLens<'a> {
    pub inner_value: u32,  // owned, from inner.value
    pub name_ref: &'a str, // borrow &str from String
}

impl<'a> From<&'a Outer> for ProjLens<'a> {
    fn from(v: &'a Outer) -> Self {
        Self {
            inner_value: v.inner.value,
            name_ref: &v.name,
        }
    }
}

#[drv::memo(single)]
fn proj_derived<'a>(lens: impl Into<ProjLens<'a>>) -> String {
    format!("{}={}", lens.name_ref, lens.inner_value)
}

static PROJ_HIT_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn proj_derived_for_hit<'a>(lens: impl Into<ProjLens<'a>>) -> String {
    PROJ_HIT_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!("{}={}", lens.name_ref, lens.inner_value)
}

static PROJ_MISS_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn proj_derived_for_miss<'a>(lens: impl Into<ProjLens<'a>>) -> String {
    PROJ_MISS_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!("{}={}", lens.name_ref, lens.inner_value)
}

// Projection lens used together with a regular lens in a multi-lens memo.
#[drv::memo(single)]
fn proj_plus_regular<'a, 'b>(fl: impl Into<ProjLens<'a>>, bl: impl Into<BaseLens<'b>>) -> String {
    format!("{}-{}", fl.name_ref, bl.base)
}

// ══════════════════════════════════════════════════════════════════════
// 13. PROJ DOES ARBITRARY LOGIC — the From impl isn't bound to "copy
//     matching fields"; it can transform values, double things, etc.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct OverrideAtom {
    pub n: u32,
    pub tag: String,
}

// Standalone lens with the same field names as the atom, but the
// user-written `From` impl deliberately doubles `n` to verify the
// projection can transform values (not just copy them).
#[drv::lens]
struct OverrideLens<'a> {
    pub n: u32,
    pub tag: &'a String,
}

impl<'a> From<&'a OverrideAtom> for OverrideLens<'a> {
    fn from(v: &'a OverrideAtom) -> Self {
        // Deliberately diverges from the default: double `n`, keep `tag` as-is.
        Self {
            n: v.n * 2,
            tag: &v.tag,
        }
    }
}

#[drv::memo(single)]
fn override_display<'a>(lens: impl Into<OverrideLens<'a>>) -> String {
    format!("{}:{}", lens.tag, lens.n)
}

// ══════════════════════════════════════════════════════════════════════
// 14. VALUE-KEYED CACHE — ping-pong hits (different atom instances with
//     the same field values share the cache entry) and LRU eviction.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
#[drv::atom]
pub struct CacheBehaviorAtom {
    #[drv::lens(CbLens)]
    pub value: u32,
}

static CACHE_BEHAVIOR_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(lru = 4)]
fn cache_behavior<'a>(lens: impl Into<CbLens<'a>>) -> u32 {
    CACHE_BEHAVIOR_COMPUTES.fetch_add(1, Ordering::SeqCst);
    lens.value * 2
}

// Tiny cache to make LRU eviction trivial to trigger.
static LRU_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(lru = 2)]
fn lru_memo<'a>(lens: impl Into<CbLens<'a>>) -> u32 {
    LRU_COMPUTES.fetch_add(1, Ordering::SeqCst);
    lens.value + 1000
}

// Single-slot cache: only the most recent (input, output) is remembered.
static SINGLE_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn single_memo<'a>(lens: impl Into<CbLens<'a>>) -> u32 {
    SINGLE_COMPUTES.fetch_add(1, Ordering::SeqCst);
    lens.value + 7
}

// ══════════════════════════════════════════════════════════════════════
// 15. LITERAL `&Lens` SIGNATURE — users who want an honest, non-sugared
//     signature can write `&Lens` directly. The macro preserves it
//     verbatim; callers project explicitly at the call site via `.into()`.
// ══════════════════════════════════════════════════════════════════════

#[drv::memo(single)]
fn literal_ref_memo(lens: &CbLens) -> u32 {
    lens.value * 5
}

// ══════════════════════════════════════════════════════════════════════
// ASSEMBLE — must come after all declarations
// ══════════════════════════════════════════════════════════════════════

drv::assemble!();

// ══════════════════════════════════════════════════════════════════════
// TESTS
// ══════════════════════════════════════════════════════════════════════

#[test]
fn inline_lens_memoizes_on_irrelevant_change() {
    let mut state = Editor {
        scroll_row: 0,
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into(), "ccc".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let result = visible_lines(&state);
    assert_eq!(result, vec!["aaa".to_string(), "bbb".to_string()]);

    // Change cursor (not in VisibleLines lens) — cache hit.
    state.cursor_row = 99;
    state.cursor_col = 42;
    let result2 = visible_lines(&state);
    assert_eq!(result2, vec!["aaa".to_string(), "bbb".to_string()]);
}

#[test]
fn inline_lens_recomputes_on_relevant_change() {
    let mut state = Editor {
        scroll_row: 0,
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into(), "ccc".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let _ = visible_lines(&state);

    state.scroll_row = 1;
    let result = visible_lines(&state);
    assert_eq!(result, vec!["bbb".to_string(), "ccc".to_string()]);
}

#[test]
fn field_in_multiple_lenses() {
    let state = Editor {
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let lines = visible_lines(&state);
    assert_eq!(lines, vec!["aaa".to_string(), "bbb".to_string()]);

    let tabs = tab_list(&state);
    assert_eq!(tabs, vec!["main.rs".to_string(), "(2 lines)".to_string()]);
}

#[test]
fn standalone_lens_basic() {
    let mut state = Dashboard {
        user_name: "alice".into(),
        notification_count: 3,
        theme: "dark".into(),
        ..Default::default()
    };

    let badge = notification_badge(&state);
    assert_eq!(badge, "alice: 3 new");

    // Change theme (not in NotificationLens) — cache hit.
    state.theme = "light".into();
    let badge2 = notification_badge(&state);
    assert_eq!(badge2, "alice: 3 new");

    // Change notification_count — recomputes.
    state.notification_count = 0;
    let badge3 = notification_badge(&state);
    assert_eq!(badge3, "alice: no notifications");
}

#[test]
fn standalone_lens_imbl_vector() {
    let mut state = Dashboard {
        items: ImVector::from(vec!["a".into(), "b".into(), "c".into()]),
        ..Default::default()
    };

    assert_eq!(item_count(&state), 3);

    // Change user_name — item_count lens doesn't include it.
    state.user_name = "carol".into();
    assert_eq!(item_count(&state), 3);

    // Add an item — recomputes.
    state.items.push_back("d".into());
    assert_eq!(item_count(&state), 4);
}

#[test]
fn chaining_memo_output_as_atom() {
    let mut summary = Summary {
        total: 5,
        label: "test".into(),
    };

    let doubled = doubled_total(&summary);
    assert_eq!(doubled, 10);

    // Change label (not in CountLens) — cache hit.
    summary.label = "changed".into();
    assert_eq!(doubled_total(&summary), 10);

    // Change total — recomputes.
    summary.total = 7;
    assert_eq!(doubled_total(&summary), 14);
}

#[test]
fn multiple_lenses_same_atom_independent() {
    let mut state = GameState {
        player_x: 3.0,
        player_y: 4.0,
        score: 100,
        high_score: 200,
        ..Default::default()
    };

    let dist = player_distance(&state);
    assert!((dist - 5.0).abs() < 0.001);

    let score = score_display(&state);
    assert_eq!(score, "100 / 200");

    // Change score — player_distance still cached, score_display recomputes.
    state.score = 150;
    let dist2 = player_distance(&state);
    assert!((dist2 - 5.0).abs() < 0.001); // cache hit

    let score2 = score_display(&state);
    assert_eq!(score2, "150 / 200"); // recomputed

    // Change player — score_display still cached, player_distance recomputes.
    state.player_x = 0.0;
    let dist3 = player_distance(&state);
    assert!((dist3 - 4.0).abs() < 0.001); // recomputed

    let score3 = score_display(&state);
    assert_eq!(score3, "150 / 200"); // cache hit
}

#[test]
fn imbl_hashmap_structural_sharing() {
    let mut state = BufferStore {
        buffers: ImHashMap::unit(
            "main.rs".into(),
            BufferData {
                content: "hello".into(),
                modified: false,
            },
        ),
        active: Some("main.rs".into()),
        ..Default::default()
    };

    let content = active_content(&state);
    assert_eq!(content, Some("hello".to_string()));

    // Change last_save (not in lens) — cache hit.
    state.last_save = 42;
    let content2 = active_content(&state);
    assert_eq!(content2, Some("hello".to_string()));

    // Update a buffer — recomputes.
    state.buffers = state.buffers.update(
        "main.rs".into(),
        BufferData {
            content: "world".into(),
            modified: true,
        },
    );
    let content3 = active_content(&state);
    assert_eq!(content3, Some("world".to_string()));
}

#[test]
fn imbl_hashmap_no_active_buffer() {
    let state = BufferStore::default();
    let content = active_content(&state);
    assert_eq!(content, None);
}

#[test]
fn single_field_lens() {
    let mut state = Counter {
        value: 5,
        ..Default::default()
    };

    assert!(is_positive(&state));

    // Change name — cache hit.
    state.name = "changed".into();
    assert!(is_positive(&state));

    // Change value to negative — recomputes.
    state.value = -1;
    assert!(!is_positive(&state));
}

#[test]
fn empty_content_visible_lines() {
    let state = Editor::default();
    let result = visible_lines(&state);
    assert!(result.is_empty());
}

#[test]
fn scroll_past_end() {
    let state = Editor {
        scroll_row: 100,
        viewport_rows: 10,
        content: ImVector::from(vec!["only line".into()]),
        ..Default::default()
    };

    let result = visible_lines(&state);
    assert!(result.is_empty());
}

#[test]
fn repeated_eval_same_state_no_recompute() {
    let state = Counter {
        value: 42,
        ..Default::default()
    };

    for _ in 0..100 {
        assert!(is_positive(&state));
    }
}

#[test]
fn multi_lens_basic() {
    let editor = Editor {
        tabs: ImVector::from(vec!["a.rs".into(), "b.rs".into()]),
        ..Default::default()
    };

    let dashboard = Dashboard {
        user_name: "alice".into(),
        ..Default::default()
    };

    let header = combined_header(&editor, &dashboard);
    assert_eq!(header, "alice: 2 tabs open");
}

#[test]
fn multi_lens_memoizes_on_irrelevant_change() {
    let mut editor = Editor {
        tabs: ImVector::from(vec!["a.rs".into()]),
        ..Default::default()
    };

    let dashboard = Dashboard {
        user_name: "bob".into(),
        notification_count: 5,
        theme: "dark".into(),
        ..Default::default()
    };

    let _ = combined_header(&editor, &dashboard);

    editor.cursor_col = 42;
    let mut dashboard2 = dashboard.clone();
    dashboard2.theme = "light".into();

    let header = combined_header(&editor, &dashboard2);
    assert_eq!(header, "bob: 1 tabs open"); // cached
}

#[test]
fn multi_lens_recomputes_on_relevant_change() {
    let mut editor = Editor {
        tabs: ImVector::from(vec!["a.rs".into()]),
        ..Default::default()
    };

    let dashboard = Dashboard {
        user_name: "carol".into(),
        ..Default::default()
    };

    let _ = combined_header(&editor, &dashboard);

    editor.tabs = ImVector::from(vec!["a.rs".into(), "b.rs".into(), "c.rs".into()]);
    let header = combined_header(&editor, &dashboard);
    assert_eq!(header, "carol: 3 tabs open");

    let dashboard2 = Dashboard {
        user_name: "dave".into(),
        ..Default::default()
    };
    let header2 = combined_header(&editor, &dashboard2);
    assert_eq!(header2, "dave: 3 tabs open");
}

#[test]
fn two_level_chaining() {
    let mut app = AppState {
        items: ImVector::from(vec!["foo".into(), "bar".into(), "baz".into()]),
        selected: Some(1),
        ..Default::default()
    };

    let summary = items_summary(&app);
    assert_eq!(summary.count, 3);
    assert_eq!(summary.current, "bar");

    let label = summary_label(&summary);
    assert_eq!(label, "3 items, viewing: bar");

    // Change theme (not in AppItemsLens) — level 1 cache hit.
    app.theme = "dark".into();
    let summary2 = items_summary(&app);
    assert_eq!(summary2.count, 3);

    // Change selection — level 1 recomputes.
    app.selected = None;
    let summary3 = items_summary(&app);
    assert_eq!(summary3.count, 3);
    assert_eq!(summary3.current, "");

    let label3 = summary_label(&summary3);
    assert_eq!(label3, "3 items, none selected");
}

#[test]
fn atom_as_memo_input() {
    let mut s = ItemsSummary {
        count: 5,
        current: "hello".into(),
    };

    assert_eq!(summary_label_full(&s), "5: hello");

    s.count = 10;
    assert_eq!(summary_label_full(&s), "10: hello");

    s.current = "world".into();
    assert_eq!(summary_label_full(&s), "10: world");
}

#[test]
fn outer_memo_caches() {
    // The generated wrapper releases the cache borrow before running the
    // user's compute function, keeping the RefCell invariants intact even
    // if the compute itself reborrows (via some other code path).
    let r = Reentrant { value: 5 };
    assert_eq!(outer_memo(&r), 11);
    assert_eq!(outer_memo(&r), 11); // cache hit
}

// ── Mixed lens + value parameter tests ──

#[test]
fn mixed_value_after_lens() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_value_after(&a, 3), 30);
    // Same multiplier: cache hit.
    assert_eq!(with_value_after(&a, 3), 30);
    // Different multiplier: recomputes.
    assert_eq!(with_value_after(&a, 4), 40);
}

#[test]
fn mixed_value_before_lens() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_value_before(5, &a), 50);
    // Same call: cache hit.
    assert_eq!(with_value_before(5, &a), 50);
    // Different value: recomputes.
    assert_eq!(with_value_before(7, &a), 70);
}

#[test]
fn mixed_multi_value() {
    let a = MixedAtom { base: 10 };
    // offset=1, scale=3: 1 + (10 * 3) = 31
    assert_eq!(multi_value(1, &a, 3), 31);
    assert_eq!(multi_value(1, &a, 3), 31); // hit
    assert_eq!(multi_value(2, &a, 3), 32); // different offset
    assert_eq!(multi_value(2, &a, 5), 52); // different scale
}

#[test]
fn mixed_lens_change_invalidates() {
    let mut a = MixedAtom { base: 10 };
    assert_eq!(with_value_after(&a, 2), 20);
    // Change the lens's field — cache should invalidate.
    a.base = 100;
    assert_eq!(with_value_after(&a, 2), 200);
}

#[test]
fn value_ref_str() {
    let a = MixedAtom { base: 42 };
    // Pass &str — stored internally as String.
    assert_eq!(with_str(&a, "val"), "val=42");
    // Same string: cache hit.
    assert_eq!(with_str(&a, "val"), "val=42");
    // Different string: recomputes.
    assert_eq!(with_str(&a, "new"), "new=42");
    // Back to the first string: recomputes (single slot, old was overwritten).
    assert_eq!(with_str(&a, "val"), "val=42");
}

#[test]
fn value_ref_bytes() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_bytes(&a, &[1, 2, 3]), 13);
    assert_eq!(with_bytes(&a, &[1, 2, 3]), 13); // hit
    assert_eq!(with_bytes(&a, &[1, 2, 3, 4]), 14); // different bytes
}

#[test]
fn value_ref_with_owned_string() {
    // Users can also pass owned types that coerce via AutoRef.
    let a = MixedAtom { base: 1 };
    let s: String = "hello".into();
    assert_eq!(with_str(&a, &s), "hello=1");
}

// ══════════════════════════════════════════════════════════════════════
// Send assertion — atoms are plain structs; Send is inherited from T.
// ══════════════════════════════════════════════════════════════════════

#[test]
fn atoms_are_send() {
    fn assert_send<T: Send>() {}

    assert_send::<Editor>();
    assert_send::<Dashboard>();
    assert_send::<Summary>();
    assert_send::<GameState>();
    assert_send::<BufferStore>();
    assert_send::<Counter>();
    assert_send::<AppState>();
    assert_send::<ItemsSummary>();
}

// ══════════════════════════════════════════════════════════════════════
// FastEq ptr_eq fast path (Arc)
// ══════════════════════════════════════════════════════════════════════

#[test]
fn arc_ptr_eq_fast_path() {
    ARC_MEMO_COMPUTES.store(0, Ordering::SeqCst);

    let data = Arc::new(vec![1u32, 2, 3, 4, 5]);
    let mut a = ArcAtom { data: data.clone() };
    assert_eq!(arc_sum(&a), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    // Identical Arc — should hit the cache (trivially).
    assert_eq!(arc_sum(&a), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    // Replace with a *different* Arc that has *identical* contents.
    // The generic PartialEq would still see equality (O(n)), and the
    // ptr_eq short-circuit is an optimisation — both paths must skip recompute.
    a.data = Arc::new(vec![1u32, 2, 3, 4, 5]);
    assert_eq!(arc_sum(&a), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    // Contents differ — must recompute.
    a.data = Arc::new(vec![1u32, 2, 3, 4, 5, 6]);
    assert_eq!(arc_sum(&a), 21);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 2);

    // Same Arc pointer as the most recent snapshot — ptr_eq wins, no recompute.
    let p = a.data.clone();
    a.data = p;
    assert_eq!(arc_sum(&a), 21);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 2);
}

// ══════════════════════════════════════════════════════════════════════
// Projection lens tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn proj_lens_basic() {
    let a = Outer {
        inner: Inner {
            value: 42,
            label: "hello".into(),
        },
        name: "test".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived(&a), "test=42");
}

#[test]
fn proj_lens_cache_hit() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived_for_hit(&a), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);

    // Change a field the lens doesn't project → cache hit.
    a.count = 999;
    assert_eq!(proj_derived_for_hit(&a), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);

    // Change inner.label — also not projected → cache hit.
    a.inner.label = "changed".into();
    assert_eq!(proj_derived_for_hit(&a), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);
}

#[test]
fn proj_lens_cache_miss() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived_for_miss(&a), "foo=10");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 1);

    // Change inner.value — projected as inner_value → cache miss.
    a.inner.value = 20;
    assert_eq!(proj_derived_for_miss(&a), "foo=20");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 2);

    // Change name — projected as name_ref → cache miss.
    a.name = "bar".into();
    assert_eq!(proj_derived_for_miss(&a), "bar=20");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 3);
}

#[test]
fn proj_lens_mixed_with_regular() {
    let outer = Outer {
        inner: Inner {
            value: 5,
            label: "x".into(),
        },
        name: "hello".into(),
        ..Default::default()
    };
    let mixed = MixedAtom { base: 7 };
    assert_eq!(proj_plus_regular(&outer, &mixed), "hello-7");
}

#[test]
fn proj_lens_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Outer>();
}

// ══════════════════════════════════════════════════════════════════════
// Copy-by-value + explicit &T in standalone lens
// ══════════════════════════════════════════════════════════════════════

#[test]
fn proj_can_transform_values() {
    // The `From<&OverrideAtom>` impl doubles `n` — verifies the
    // projection can compute derived values, not just copy fields.
    let a = OverrideAtom {
        n: 7,
        tag: "hi".into(),
    };
    assert_eq!(override_display(&a), "hi:14");
}

#[test]
fn copy_by_value_in_lens() {
    // Standalone lens with x: u32 (owned, no deref) and y: &u32 (reference, deref).
    // Both styles work in the same lens.
    let a = CopyTest {
        x: 10,
        y: 20,
        ..Default::default()
    };
    assert_eq!(copy_mix_sum(&a), 30);
}

// ══════════════════════════════════════════════════════════════════════
// VALUE-KEYED CACHE tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn ping_pong_cache_hit_across_values() {
    CACHE_BEHAVIOR_COMPUTES.store(0, Ordering::SeqCst);

    let a1 = CacheBehaviorAtom { value: 10 };
    let a2 = CacheBehaviorAtom { value: 20 };

    // First touches each compute once.
    assert_eq!(cache_behavior(&a1), 20);
    assert_eq!(cache_behavior(&a2), 40);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);

    // Ping-pong: value 10 → 20 → 10. With `lru = 4`, both states stay
    // cached and the repeat call hits.
    assert_eq!(cache_behavior(&a1), 20);
    assert_eq!(cache_behavior(&a2), 40);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);

    // Different instance with same field value → still a hit.
    let a1_clone = CacheBehaviorAtom { value: 10 };
    assert_eq!(cache_behavior(&a1_clone), 20);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);
}

#[test]
fn single_strategy_last_call_only() {
    SINGLE_COMPUTES.store(0, Ordering::SeqCst);

    let a = CacheBehaviorAtom { value: 1 };
    let b = CacheBehaviorAtom { value: 2 };

    // Compute A: miss.
    assert_eq!(single_memo(&a), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 1);

    // Same inputs → hit (slot still holds A).
    assert_eq!(single_memo(&a), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 1);

    // Different input B: miss, and the slot now holds B.
    assert_eq!(single_memo(&b), 9);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 2);

    // Back to A: miss — single-slot cache evicted A when B was installed.
    // (With lru>=2 this would have hit; with `single` it recomputes.)
    assert_eq!(single_memo(&a), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 3);

    // Different instance with the same value as the current slot: hit.
    let a_clone = CacheBehaviorAtom { value: 1 };
    assert_eq!(single_memo(&a_clone), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 3);
}

#[test]
fn lru_evicts_least_recently_used() {
    LRU_COMPUTES.store(0, Ordering::SeqCst);

    let a = CacheBehaviorAtom { value: 1 };
    let b = CacheBehaviorAtom { value: 2 };
    let c = CacheBehaviorAtom { value: 3 };

    // Fill both slots. Cache is [1, 2] with 1 older than 2.
    assert_eq!(lru_memo(&a), 1001);
    assert_eq!(lru_memo(&b), 1002);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 2);

    // Access `a` again — now `b` is LRU.
    assert_eq!(lru_memo(&a), 1001);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 2);

    // Bring in `c` → should evict `b` (LRU), keep `a`.
    assert_eq!(lru_memo(&c), 1003);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 3);

    // `a` still in cache.
    assert_eq!(lru_memo(&a), 1001);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 3);

    // `b` was evicted → recompute.
    assert_eq!(lru_memo(&b), 1002);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 4);
}

#[test]
fn literal_ref_signature_with_explicit_projection() {
    // Literal `&Lens` signature — the macro doesn't rewrite to
    // `impl Into<...>`, so callers must project to a `&Lens` themselves.
    // Here we use the auto-generated `From<&CacheBehaviorAtom> for CbLens<'_>`
    // and the explicit `.into()` on the borrow.
    let atom = CacheBehaviorAtom { value: 4 };
    let lens: CbLens<'_> = (&atom).into();
    assert_eq!(literal_ref_memo(&lens), 20);

    // Cache hit on the same projected lens value.
    let lens2: CbLens<'_> = (&atom).into();
    assert_eq!(literal_ref_memo(&lens2), 20);

    // Different underlying atom with the same projected value — still a hit
    // (value-keyed cache, not instance-keyed).
    let other = CacheBehaviorAtom { value: 4 };
    let lens3: CbLens<'_> = (&other).into();
    assert_eq!(literal_ref_memo(&lens3), 20);
}
