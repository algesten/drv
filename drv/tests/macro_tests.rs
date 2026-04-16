use imbl::HashMap as ImHashMap;
use imbl::Vector as ImVector;

// ══════════════════════════════════════════════════════════════════════
// 1. INLINE LENSES — basic atom with field annotations
// ══════════════════════════════════════════════════════════════════════

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

#[drv::memo]
fn visible_lines(lens: &VisibleLines) -> Vec<String> {
    lens.content
        .iter()
        .skip(*lens.scroll_row as usize)
        .take(*lens.viewport_rows as usize)
        .cloned()
        .collect()
}

#[drv::memo]
fn tab_list(lens: &TabList) -> Vec<String> {
    let mut out: Vec<String> = lens.tabs.iter().cloned().collect();
    out.push(format!("({} lines)", lens.content.len()));
    out
}

// ══════════════════════════════════════════════════════════════════════
// 2. STANDALONE LENS — separate struct with #[drv::lens(Atom)]
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct Dashboard {
    pub user_name: String,
    pub notification_count: u32,
    pub theme: String,
    pub items: ImVector<String>,
}

#[drv::lens(Dashboard)]
struct NotificationLens {
    pub user_name: String,
    pub notification_count: u32,
}

#[drv::memo]
fn notification_badge(lens: &NotificationLens) -> String {
    if *lens.notification_count == 0 {
        format!("{}: no notifications", lens.user_name)
    } else {
        format!("{}: {} new", lens.user_name, lens.notification_count)
    }
}

#[drv::lens(Dashboard)]
struct ItemsLens {
    pub items: ImVector<String>,
}

#[drv::memo]
fn item_count(lens: &ItemsLens) -> usize {
    lens.items.len()
}

// ══════════════════════════════════════════════════════════════════════
// 3. CHAINING — memo output as atom, feeding into another memo
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct Summary {
    #[drv::lens(CountLens)]
    pub total: usize,

    pub label: String,
}

#[drv::memo]
fn doubled_total(lens: &CountLens) -> usize {
    *lens.total * 2
}

// ══════════════════════════════════════════════════════════════════════
// 4. MULTIPLE LENSES ON SAME ATOM — different projections
// ══════════════════════════════════════════════════════════════════════

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

#[drv::memo]
fn player_distance(lens: &PlayerLens) -> f32 {
    (*lens.player_x * *lens.player_x + *lens.player_y * *lens.player_y).sqrt()
}

#[drv::memo]
fn score_display(lens: &ScoreLens) -> String {
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

#[drv::atom]
pub struct BufferStore {
    #[drv::lens(AllBuffersLens)]
    pub buffers: ImHashMap<String, BufferData>,

    #[drv::lens(AllBuffersLens)]
    pub active: Option<String>,

    pub last_save: u64,
}

#[drv::memo]
fn active_content(lens: &AllBuffersLens) -> Option<String> {
    lens.active
        .as_ref()
        .and_then(|name| lens.buffers.get(name))
        .map(|b| b.content.clone())
}

// ══════════════════════════════════════════════════════════════════════
// 6. SINGLE-FIELD LENS — minimal case
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct Counter {
    #[drv::lens(ValueLens)]
    pub value: i64,

    pub name: String,
}

#[drv::memo]
fn is_positive(lens: &ValueLens) -> bool {
    *lens.value > 0
}

// ══════════════════════════════════════════════════════════════════════
// 7. MULTI-LENS — memo taking lenses from different atoms
// ══════════════════════════════════════════════════════════════════════

#[drv::lens(Editor)]
struct EditorTabsLens {
    pub tabs: ImVector<String>,
}

#[drv::lens(Dashboard)]
struct DashboardUserLens {
    pub user_name: String,
}

#[drv::memo]
fn combined_header(tabs: &EditorTabsLens, user: &DashboardUserLens) -> String {
    format!("{}: {} tabs open", user.user_name, tabs.tabs.len())
}

// ══════════════════════════════════════════════════════════════════════
// 8. TWO-LEVEL CHAINING — memo output is an atom feeding another memo
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct AppState {
    #[drv::lens(AppItemsLens)]
    pub items: ImVector<String>,

    #[drv::lens(AppItemsLens)]
    pub selected: Option<usize>,

    pub theme: String,
}

#[drv::memo]
fn items_summary(lens: &AppItemsLens) -> ItemsSummary {
    ItemsSummary {
        count: lens.items.len(),
        current: lens
            .selected
            .and_then(|i| lens.items.get(i).cloned())
            .unwrap_or_default(),
        ..Default::default()
    }
}

#[drv::atom]
pub struct ItemsSummary {
    #[drv::lens(ItemsSummaryLens)]
    pub count: usize,

    #[drv::lens(ItemsSummaryLens)]
    pub current: String,
}

#[drv::memo]
fn summary_label(lens: &ItemsSummaryLens) -> String {
    if lens.current.is_empty() {
        format!("{} items, none selected", lens.count)
    } else {
        format!("{} items, viewing: {}", lens.count, lens.current)
    }
}

// Atom used directly as memo input (identity lens — all fields).
#[drv::memo]
fn summary_label_full(s: &ItemsSummary) -> String {
    format!("{}: {}", s.count, s.current)
}

// ══════════════════════════════════════════════════════════════════════
// 9. REENTRANCY — a memo body calling another memo on the same atom
//    should panic at RefCell::borrow_mut() on the inner call.
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct Reentrant {
    pub value: u32,
}

#[drv::memo]
fn inner_memo(r: &Reentrant) -> u32 {
    r.value * 2
}

#[drv::memo]
fn outer_memo(r: &Reentrant) -> u32 {
    // Reentrant: while outer_memo holds the RefMut on r.__drv, we call
    // inner_memo which also tries to borrow_mut the same RefCell.
    inner_memo(r) + 1
}

// ══════════════════════════════════════════════════════════════════════
// 10. MIXED PARAMS — memos taking lens and value parameters together.
// ══════════════════════════════════════════════════════════════════════

#[drv::atom]
pub struct MixedAtom {
    #[drv::lens(BaseLens)]
    pub base: u32,
}

// Value param AFTER a lens param.
#[drv::memo]
fn with_value_after(lens: &BaseLens, multiplier: u32) -> u32 {
    *lens.base * multiplier
}

// Value param BEFORE a lens param — order must be preserved.
#[drv::memo]
fn with_value_before(multiplier: u32, lens: &BaseLens) -> u32 {
    multiplier * *lens.base
}

// Multiple value params mixed with a lens.
#[drv::memo]
fn multi_value(offset: u32, lens: &BaseLens, scale: u32) -> u32 {
    offset + (*lens.base * scale)
}

// Reference value params — `&str` and `&[u8]` stored via ToOwned.
#[drv::memo]
fn with_str(lens: &BaseLens, prefix: &str) -> String {
    format!("{}={}", prefix, lens.base)
}

#[drv::memo]
fn with_bytes(lens: &BaseLens, bytes: &[u8]) -> usize {
    (*lens.base as usize) + bytes.len()
}

// ══════════════════════════════════════════════════════════════════════
// 11. FastEq ptr_eq — Arc<T> field should short-circuit equality via
//     Arc::ptr_eq, so cloning the atom and calling the memo again hits
//     the fast path even when T: !PartialEq would otherwise be O(n).
// ══════════════════════════════════════════════════════════════════════

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[drv::atom]
pub struct ArcAtom {
    #[drv::lens(ArcLens)]
    pub data: Arc<Vec<u32>>,
}

static ARC_MEMO_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo]
fn arc_sum(lens: &ArcLens) -> u32 {
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

#[drv::atom]
pub struct Outer {
    pub inner: Inner,
    pub name: String,
    pub count: u32,
}

// Factory lens: field names differ, types differ, reaches into nested struct.
#[drv::lens(Outer)]
struct FactoryLens<'a> {
    pub inner_value: u32,  // owned, from inner.value
    pub name_ref: &'a str, // borrow &str from String
}

#[drv::factory]
impl<'a> From<&'a Outer> for FactoryLens<'a> {
    fn from(v: &'a Outer) -> Self {
        Self {
            inner_value: v.inner.value,
            name_ref: &v.name,
        }
    }
}

static FACTORY_MEMO_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo]
fn factory_derived(lens: &FactoryLens) -> String {
    FACTORY_MEMO_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!("{}={}", lens.name_ref, lens.inner_value)
}

// Factory lens used together with a regular lens in a multi-lens memo.
#[drv::memo]
fn factory_plus_regular(fl: &FactoryLens, bl: &BaseLens) -> String {
    format!("{}-{}", fl.name_ref, bl.base)
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
        ..Default::default()
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
    let dashboard2 = Dashboard {
        theme: "light".into(),
        ..dashboard.clone()
    };

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
        ..Default::default()
    };

    assert_eq!(summary_label_full(&s), "5: hello");

    s.count = 10;
    assert_eq!(summary_label_full(&s), "10: hello");

    s.current = "world".into();
    assert_eq!(summary_label_full(&s), "10: world");
}

#[test]
fn reentrant_memo_is_safe() {
    // A memo body that calls another memo on the same atom must not panic.
    // The generated code releases the RefCell borrow before invoking the
    // user's compute, so reentrant memo calls are safe.
    let r = Reentrant {
        value: 5,
        ..Default::default()
    };
    // outer = inner(r) + 1 = (5 * 2) + 1 = 11
    assert_eq!(outer_memo(&r), 11);
    // Second call hits cache.
    assert_eq!(outer_memo(&r), 11);
}

// ── Mixed lens + value parameter tests ──

#[test]
fn mixed_value_after_lens() {
    let a = MixedAtom {
        base: 10,
        ..Default::default()
    };
    assert_eq!(with_value_after(&a, 3), 30);
    // Same multiplier: cache hit.
    assert_eq!(with_value_after(&a, 3), 30);
    // Different multiplier: recomputes.
    assert_eq!(with_value_after(&a, 4), 40);
}

#[test]
fn mixed_value_before_lens() {
    let a = MixedAtom {
        base: 10,
        ..Default::default()
    };
    assert_eq!(with_value_before(5, &a), 50);
    // Same call: cache hit.
    assert_eq!(with_value_before(5, &a), 50);
    // Different value: recomputes.
    assert_eq!(with_value_before(7, &a), 70);
}

#[test]
fn mixed_multi_value() {
    let a = MixedAtom {
        base: 10,
        ..Default::default()
    };
    // offset=1, scale=3: 1 + (10 * 3) = 31
    assert_eq!(multi_value(1, &a, 3), 31);
    assert_eq!(multi_value(1, &a, 3), 31); // hit
    assert_eq!(multi_value(2, &a, 3), 32); // different offset
    assert_eq!(multi_value(2, &a, 5), 52); // different scale
}

#[test]
fn mixed_lens_change_invalidates() {
    let mut a = MixedAtom {
        base: 10,
        ..Default::default()
    };
    assert_eq!(with_value_after(&a, 2), 20);
    // Change the lens's field — cache should invalidate.
    a.base = 100;
    assert_eq!(with_value_after(&a, 2), 200);
}

#[test]
fn value_ref_str() {
    let a = MixedAtom {
        base: 42,
        ..Default::default()
    };
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
    let a = MixedAtom {
        base: 10,
        ..Default::default()
    };
    assert_eq!(with_bytes(&a, &[1, 2, 3]), 13);
    assert_eq!(with_bytes(&a, &[1, 2, 3]), 13); // hit
    assert_eq!(with_bytes(&a, &[1, 2, 3, 4]), 14); // different bytes
}

#[test]
fn value_ref_with_owned_string() {
    // Users can also pass owned types that coerce via AutoRef.
    let a = MixedAtom {
        base: 1,
        ..Default::default()
    };
    let s: String = "hello".into();
    assert_eq!(with_str(&a, &s), "hello=1");
}

// ══════════════════════════════════════════════════════════════════════
// Send assertion — all atoms must be Send so they can move across threads.
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
    let mut a = ArcAtom {
        data: data.clone(),
        ..Default::default()
    };
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
// Factory lens tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn factory_lens_basic() {
    let a = Outer {
        inner: Inner {
            value: 42,
            label: "hello".into(),
        },
        name: "test".into(),
        ..Default::default()
    };
    assert_eq!(factory_derived(&a), "test=42");
}

#[test]
fn factory_lens_cache_hit() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(factory_derived(&a), "foo=10");
    let count_after_first = FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst);

    // Change a field the factory lens doesn't project → cache hit.
    a.count = 999;
    assert_eq!(factory_derived(&a), "foo=10");
    assert_eq!(
        FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst),
        count_after_first
    );

    // Change inner.label — also not projected → cache hit.
    a.inner.label = "changed".into();
    assert_eq!(factory_derived(&a), "foo=10");
    assert_eq!(
        FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst),
        count_after_first
    );
}

#[test]
fn factory_lens_cache_miss() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(factory_derived(&a), "foo=10");
    let c0 = FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst);

    // Change inner.value — projected as inner_value → cache miss.
    a.inner.value = 20;
    assert_eq!(factory_derived(&a), "foo=20");
    let c1 = FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst);
    assert_eq!(c1, c0 + 1);

    // Change name — projected as name_ref → cache miss.
    a.name = "bar".into();
    assert_eq!(factory_derived(&a), "bar=20");
    assert_eq!(FACTORY_MEMO_COMPUTES.load(Ordering::SeqCst), c1 + 1);
}

#[test]
fn factory_lens_mixed_with_regular() {
    let outer = Outer {
        inner: Inner {
            value: 5,
            label: "x".into(),
        },
        name: "hello".into(),
        ..Default::default()
    };
    let mixed = MixedAtom {
        base: 7,
        ..Default::default()
    };
    assert_eq!(factory_plus_regular(&outer, &mixed), "hello-7");
}

#[test]
fn factory_lens_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Outer>();
}
