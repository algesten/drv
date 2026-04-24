use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use imbl::HashMap as ImHashMap;
use imbl::Vector as ImVector;

// ══════════════════════════════════════════════════════════════════════
// 1. MULTIPLE INPUTS ON ONE ATOM
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Editor {
    pub scroll_row: u32,
    pub viewport_rows: u32,
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub content: ImVector<String>,
    pub tabs: ImVector<String>,
}

#[derive(drv::Input)]
struct VisibleLines<'a> {
    pub scroll_row: u32,
    pub viewport_rows: u32,
    pub content: &'a ImVector<String>,
}

impl<'a> From<&'a Editor> for VisibleLines<'a> {
    fn from(e: &'a Editor) -> Self {
        Self {
            scroll_row: e.scroll_row,
            viewport_rows: e.viewport_rows,
            content: &e.content,
        }
    }
}

#[derive(drv::Input)]
struct TabList<'a> {
    pub content: &'a ImVector<String>,
    pub tabs: &'a ImVector<String>,
}

impl<'a> From<&'a Editor> for TabList<'a> {
    fn from(e: &'a Editor) -> Self {
        Self {
            content: &e.content,
            tabs: &e.tabs,
        }
    }
}

#[drv::memo(single)]
fn visible_lines<'a>(input: VisibleLines<'a>) -> Vec<String> {
    input
        .content
        .iter()
        .skip(input.scroll_row as usize)
        .take(input.viewport_rows as usize)
        .cloned()
        .collect()
}

#[drv::memo(single)]
fn tab_list<'a>(input: TabList<'a>) -> Vec<String> {
    let mut out: Vec<String> = input.tabs.iter().cloned().collect();
    out.push(format!("({} lines)", input.content.len()));
    out
}

// ══════════════════════════════════════════════════════════════════════
// 2. STANDALONE INPUT, `impl Into<...>` FORM
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dashboard {
    pub user_name: String,
    pub notification_count: u32,
    pub theme: String,
    pub items: ImVector<String>,
}

#[derive(drv::Input)]
struct NotificationInput<'a> {
    pub user_name: &'a String,
    pub notification_count: u32,
}

impl<'a> From<&'a Dashboard> for NotificationInput<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self {
            user_name: &d.user_name,
            notification_count: d.notification_count,
        }
    }
}

// Exercises the verbose `impl Into<...>` form — equivalent to by-value input.
#[drv::memo(single)]
fn notification_badge<'a>(input: NotificationInput<'a>) -> String {
    if input.notification_count == 0 {
        format!("{}: no notifications", input.user_name)
    } else {
        format!("{}: {} new", input.user_name, input.notification_count)
    }
}

#[derive(drv::Input)]
struct ItemsInput<'a> {
    pub items: &'a ImVector<String>,
}

impl<'a> From<&'a Dashboard> for ItemsInput<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self { items: &d.items }
    }
}

#[drv::memo(single)]
fn item_count<'a>(input: ItemsInput<'a>) -> usize {
    input.items.len()
}

// ══════════════════════════════════════════════════════════════════════
// 3. CHAINING — memo output as atom feeding another memo
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    pub total: usize,
    pub label: String,
}

#[derive(drv::Input)]
struct CountInput<'a> {
    pub total: usize,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a Summary> for CountInput<'a> {
    fn from(s: &'a Summary) -> Self {
        Self {
            total: s.total,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
fn doubled_total<'a>(input: CountInput<'a>) -> usize {
    input.total * 2
}

// ══════════════════════════════════════════════════════════════════════
// 4. MULTIPLE INPUTS ON SAME ATOM — different projections
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GameState {
    pub player_x: f32,
    pub player_y: f32,
    pub score: u64,
    pub high_score: u64,
    pub frame_count: u64,
}

#[derive(drv::Input)]
struct PlayerInput<'a> {
    pub player_x: f32,
    pub player_y: f32,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a GameState> for PlayerInput<'a> {
    fn from(g: &'a GameState) -> Self {
        Self {
            player_x: g.player_x,
            player_y: g.player_y,
            _p: PhantomData,
        }
    }
}

#[derive(drv::Input)]
struct ScoreInput<'a> {
    pub score: u64,
    pub high_score: u64,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a GameState> for ScoreInput<'a> {
    fn from(g: &'a GameState) -> Self {
        Self {
            score: g.score,
            high_score: g.high_score,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
fn player_distance<'a>(input: PlayerInput<'a>) -> f32 {
    (input.player_x * input.player_x + input.player_y * input.player_y).sqrt()
}

#[drv::memo(single)]
fn score_display<'a>(input: ScoreInput<'a>) -> String {
    format!("{} / {}", input.score, input.high_score)
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
pub struct BufferStore {
    pub buffers: ImHashMap<String, BufferData>,
    pub active: Option<String>,
    pub last_save: u64,
}

#[derive(drv::Input)]
struct AllBuffersInput<'a> {
    pub buffers: &'a ImHashMap<String, BufferData>,
    pub active: &'a Option<String>,
}

impl<'a> From<&'a BufferStore> for AllBuffersInput<'a> {
    fn from(b: &'a BufferStore) -> Self {
        Self {
            buffers: &b.buffers,
            active: &b.active,
        }
    }
}

#[drv::memo(single)]
fn active_content<'a>(input: AllBuffersInput<'a>) -> Option<String> {
    input
        .active
        .as_ref()
        .and_then(|name| input.buffers.get(name))
        .map(|b| b.content.clone())
}

// ══════════════════════════════════════════════════════════════════════
// 6. SINGLE-FIELD INPUT
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Counter {
    pub value: i64,
    pub name: String,
}

#[derive(drv::Input)]
struct ValueInput<'a> {
    pub value: i64,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a Counter> for ValueInput<'a> {
    fn from(c: &'a Counter) -> Self {
        Self {
            value: c.value,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
fn is_positive<'a>(input: ValueInput<'a>) -> bool {
    input.value > 0
}

// ══════════════════════════════════════════════════════════════════════
// 7. MULTI-INPUT — memo taking inputs from different atoms
// ══════════════════════════════════════════════════════════════════════

#[derive(drv::Input)]
struct EditorTabsInput<'a> {
    pub tabs: &'a ImVector<String>,
}

impl<'a> From<&'a Editor> for EditorTabsInput<'a> {
    fn from(e: &'a Editor) -> Self {
        Self { tabs: &e.tabs }
    }
}

#[derive(drv::Input)]
struct DashboardUserInput<'a> {
    pub user_name: &'a String,
}

impl<'a> From<&'a Dashboard> for DashboardUserInput<'a> {
    fn from(d: &'a Dashboard) -> Self {
        Self {
            user_name: &d.user_name,
        }
    }
}

#[drv::memo(single)]
fn combined_header<'a, 'b>(tabs: EditorTabsInput<'a>, user: DashboardUserInput<'b>) -> String {
    format!("{}: {} tabs open", user.user_name, tabs.tabs.len())
}

// ══════════════════════════════════════════════════════════════════════
// 8. TWO-LEVEL CHAINING — memo output feeds another memo
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppState {
    pub items: ImVector<String>,
    pub selected: Option<usize>,
    pub theme: String,
}

#[derive(drv::Input)]
struct AppItemsInput<'a> {
    pub items: &'a ImVector<String>,
    pub selected: &'a Option<usize>,
}

impl<'a> From<&'a AppState> for AppItemsInput<'a> {
    fn from(a: &'a AppState) -> Self {
        Self {
            items: &a.items,
            selected: &a.selected,
        }
    }
}

#[drv::memo(single)]
fn items_summary<'a>(input: AppItemsInput<'a>) -> ItemsSummary {
    ItemsSummary {
        count: input.items.len(),
        current: input
            .selected
            .and_then(|i| input.items.get(i).cloned())
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Default, drv::Input)]
pub struct ItemsSummary {
    pub count: usize,
    pub current: String,
}

#[derive(drv::Input)]
struct ItemsSummaryInput<'a> {
    pub count: usize,
    pub current: &'a String,
}

impl<'a> From<&'a ItemsSummary> for ItemsSummaryInput<'a> {
    fn from(s: &'a ItemsSummary) -> Self {
        Self {
            count: s.count,
            current: &s.current,
        }
    }
}

#[drv::memo(single)]
fn summary_label<'a>(input: ItemsSummaryInput<'a>) -> String {
    if input.current.is_empty() {
        format!("{} items, none selected", input.count)
    } else {
        format!("{} items, viewing: {}", input.count, input.current)
    }
}

// Atom used directly as memo input via the value-ref path. Clone-on-miss,
// PartialEq-on-hit. No per-field FastEq fast path — declare a mirroring
// input when that matters (see ItemsSummaryInput above).
#[drv::memo(single)]
fn summary_label_full(s: &ItemsSummary) -> String {
    format!("{}: {}", s.count, s.current)
}

// ══════════════════════════════════════════════════════════════════════
// 9. REENTRANCY — a memo body may invoke another memo on the same atom.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default, drv::Input)]
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
// 10. MIXED PARAMS — input + value + value-ref
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MixedAtom {
    pub base: u32,
}

#[derive(drv::Input)]
struct BaseInput<'a> {
    pub base: u32,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a MixedAtom> for BaseInput<'a> {
    fn from(a: &'a MixedAtom) -> Self {
        Self {
            base: a.base,
            _p: PhantomData,
        }
    }
}

#[drv::memo(single)]
fn with_value_after<'a>(input: BaseInput<'a>, multiplier: u32) -> u32 {
    input.base * multiplier
}

#[drv::memo(single)]
fn with_value_before<'a>(multiplier: u32, input: BaseInput<'a>) -> u32 {
    multiplier * input.base
}

#[drv::memo(single)]
fn multi_value<'a>(offset: u32, input: BaseInput<'a>, scale: u32) -> u32 {
    offset + (input.base * scale)
}

#[drv::memo(single)]
fn with_str<'a>(input: BaseInput<'a>, prefix: &str) -> String {
    format!("{}={}", prefix, input.base)
}

#[drv::memo(single)]
fn with_bytes<'a>(input: BaseInput<'a>, bytes: &[u8]) -> usize {
    (input.base as usize) + bytes.len()
}

// ══════════════════════════════════════════════════════════════════════
// 11. FastEq ptr_eq — Arc<T> field short-circuits equality via Arc::ptr_eq.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArcAtom {
    pub data: Arc<Vec<u32>>,
}

#[derive(drv::Input)]
struct ArcInput<'a> {
    pub data: &'a Arc<Vec<u32>>,
}

impl<'a> From<&'a ArcAtom> for ArcInput<'a> {
    fn from(a: &'a ArcAtom) -> Self {
        Self { data: &a.data }
    }
}

static ARC_MEMO_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn arc_sum<'a>(input: ArcInput<'a>) -> u32 {
    ARC_MEMO_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.data.iter().sum()
}

// ══════════════════════════════════════════════════════════════════════
// 12. FACTORY INPUTS — arbitrary projection logic
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inner {
    pub value: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outer {
    pub inner: Inner,
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CopyTest {
    pub x: u32,
    pub y: u32,
    pub label: String,
}

// Owned and borrowed fields side-by-side.
#[derive(drv::Input)]
struct CopyMixInput<'a> {
    pub x: u32,
    pub y: &'a u32,
}

impl<'a> From<&'a CopyTest> for CopyMixInput<'a> {
    fn from(c: &'a CopyTest) -> Self {
        Self { x: c.x, y: &c.y }
    }
}

#[drv::memo(single)]
fn copy_mix_sum<'a>(input: CopyMixInput<'a>) -> u32 {
    input.x + *input.y
}

// Projection input: field names differ, types differ, reaches into nested struct.
#[derive(drv::Input)]
struct ProjInput<'a> {
    pub inner_value: u32,
    pub name_ref: &'a str,
}

impl<'a> From<&'a Outer> for ProjInput<'a> {
    fn from(v: &'a Outer) -> Self {
        Self {
            inner_value: v.inner.value,
            name_ref: &v.name,
        }
    }
}

#[drv::memo(single)]
fn proj_derived<'a>(input: ProjInput<'a>) -> String {
    format!("{}={}", input.name_ref, input.inner_value)
}

static PROJ_HIT_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn proj_derived_for_hit<'a>(input: ProjInput<'a>) -> String {
    PROJ_HIT_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!("{}={}", input.name_ref, input.inner_value)
}

static PROJ_MISS_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn proj_derived_for_miss<'a>(input: ProjInput<'a>) -> String {
    PROJ_MISS_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!("{}={}", input.name_ref, input.inner_value)
}

#[drv::memo(single)]
fn proj_plus_regular<'a, 'b>(fl: ProjInput<'a>, bl: BaseInput<'b>) -> String {
    format!("{}-{}", fl.name_ref, bl.base)
}

// ══════════════════════════════════════════════════════════════════════
// 13. PROJECTIONS WITH ARBITRARY LOGIC
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OverrideAtom {
    pub n: u32,
    pub tag: String,
}

#[derive(drv::Input)]
struct OverrideInput<'a> {
    pub n: u32,
    pub tag: &'a String,
}

impl<'a> From<&'a OverrideAtom> for OverrideInput<'a> {
    fn from(v: &'a OverrideAtom) -> Self {
        // Deliberately diverges from the default: double `n`, keep `tag` as-is.
        Self {
            n: v.n * 2,
            tag: &v.tag,
        }
    }
}

#[drv::memo(single)]
fn override_display<'a>(input: OverrideInput<'a>) -> String {
    format!("{}:{}", input.tag, input.n)
}

// ══════════════════════════════════════════════════════════════════════
// 14. VALUE-KEYED CACHE — ping-pong hits, LRU eviction, single-slot.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CacheBehaviorAtom {
    pub value: u32,
}

#[derive(drv::Input)]
struct CbInput<'a> {
    pub value: u32,
    _p: PhantomData<&'a ()>,
}

impl<'a> From<&'a CacheBehaviorAtom> for CbInput<'a> {
    fn from(c: &'a CacheBehaviorAtom) -> Self {
        Self {
            value: c.value,
            _p: PhantomData,
        }
    }
}

static CACHE_BEHAVIOR_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(lru = 4)]
fn cache_behavior<'a>(input: CbInput<'a>) -> u32 {
    CACHE_BEHAVIOR_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.value * 2
}

static LRU_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(lru = 2)]
fn lru_memo<'a>(input: CbInput<'a>) -> u32 {
    LRU_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.value + 1000
}

static SINGLE_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn single_memo<'a>(input: CbInput<'a>) -> u32 {
    SINGLE_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.value + 7
}

// ══════════════════════════════════════════════════════════════════════
// 15. PRE-PROJECTED INPUT AT CALL SITE
// ══════════════════════════════════════════════════════════════════════

#[drv::memo(single)]
fn preprojected<'a>(input: CbInput<'a>) -> u32 {
    input.value * 5
}

// ══════════════════════════════════════════════════════════════════════
// TESTS
// ══════════════════════════════════════════════════════════════════════

#[test]
fn inline_input_memoizes_on_irrelevant_change() {
    let mut state = Editor {
        scroll_row: 0,
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into(), "ccc".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let result = visible_lines((&state).into());
    assert_eq!(result, vec!["aaa".to_string(), "bbb".to_string()]);

    state.cursor_row = 99;
    state.cursor_col = 42;
    let result2 = visible_lines((&state).into());
    assert_eq!(result2, vec!["aaa".to_string(), "bbb".to_string()]);
}

#[test]
fn inline_input_recomputes_on_relevant_change() {
    let mut state = Editor {
        scroll_row: 0,
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into(), "ccc".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let _ = visible_lines((&state).into());

    state.scroll_row = 1;
    let result = visible_lines((&state).into());
    assert_eq!(result, vec!["bbb".to_string(), "ccc".to_string()]);
}

#[test]
fn field_in_multiple_inputs() {
    let state = Editor {
        viewport_rows: 2,
        content: ImVector::from(vec!["aaa".into(), "bbb".into()]),
        tabs: ImVector::from(vec!["main.rs".into()]),
        ..Default::default()
    };

    let lines = visible_lines((&state).into());
    assert_eq!(lines, vec!["aaa".to_string(), "bbb".to_string()]);

    let tabs = tab_list((&state).into());
    assert_eq!(tabs, vec!["main.rs".to_string(), "(2 lines)".to_string()]);
}

#[test]
fn standalone_input_basic() {
    let mut state = Dashboard {
        user_name: "alice".into(),
        notification_count: 3,
        theme: "dark".into(),
        ..Default::default()
    };

    let badge = notification_badge((&state).into());
    assert_eq!(badge, "alice: 3 new");

    state.theme = "light".into();
    let badge2 = notification_badge((&state).into());
    assert_eq!(badge2, "alice: 3 new");

    state.notification_count = 0;
    let badge3 = notification_badge((&state).into());
    assert_eq!(badge3, "alice: no notifications");
}

#[test]
fn standalone_input_imbl_vector() {
    let mut state = Dashboard {
        items: ImVector::from(vec!["a".into(), "b".into(), "c".into()]),
        ..Default::default()
    };

    assert_eq!(item_count((&state).into()), 3);

    state.user_name = "carol".into();
    assert_eq!(item_count((&state).into()), 3);

    state.items.push_back("d".into());
    assert_eq!(item_count((&state).into()), 4);
}

#[test]
fn chaining_memo_output_as_atom() {
    let mut summary = Summary {
        total: 5,
        label: "test".into(),
    };

    let doubled = doubled_total((&summary).into());
    assert_eq!(doubled, 10);

    summary.label = "changed".into();
    assert_eq!(doubled_total((&summary).into()), 10);

    summary.total = 7;
    assert_eq!(doubled_total((&summary).into()), 14);
}

#[test]
fn multiple_inputs_same_atom_independent() {
    let mut state = GameState {
        player_x: 3.0,
        player_y: 4.0,
        score: 100,
        high_score: 200,
        ..Default::default()
    };

    let dist = player_distance((&state).into());
    assert!((dist - 5.0).abs() < 0.001);

    let score = score_display((&state).into());
    assert_eq!(score, "100 / 200");

    state.score = 150;
    let dist2 = player_distance((&state).into());
    assert!((dist2 - 5.0).abs() < 0.001);

    let score2 = score_display((&state).into());
    assert_eq!(score2, "150 / 200");

    state.player_x = 0.0;
    let dist3 = player_distance((&state).into());
    assert!((dist3 - 4.0).abs() < 0.001);

    let score3 = score_display((&state).into());
    assert_eq!(score3, "150 / 200");
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

    let content = active_content((&state).into());
    assert_eq!(content, Some("hello".to_string()));

    state.last_save = 42;
    let content2 = active_content((&state).into());
    assert_eq!(content2, Some("hello".to_string()));

    state.buffers = state.buffers.update(
        "main.rs".into(),
        BufferData {
            content: "world".into(),
            modified: true,
        },
    );
    let content3 = active_content((&state).into());
    assert_eq!(content3, Some("world".to_string()));
}

#[test]
fn imbl_hashmap_no_active_buffer() {
    let state = BufferStore::default();
    let content = active_content((&state).into());
    assert_eq!(content, None);
}

#[test]
fn single_field_input() {
    let mut state = Counter {
        value: 5,
        ..Default::default()
    };

    assert!(is_positive((&state).into()));

    state.name = "changed".into();
    assert!(is_positive((&state).into()));

    state.value = -1;
    assert!(!is_positive((&state).into()));
}

#[test]
fn empty_content_visible_lines() {
    let state = Editor::default();
    let result = visible_lines((&state).into());
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

    let result = visible_lines((&state).into());
    assert!(result.is_empty());
}

#[test]
fn repeated_eval_same_state_no_recompute() {
    let state = Counter {
        value: 42,
        ..Default::default()
    };

    for _ in 0..100 {
        assert!(is_positive((&state).into()));
    }
}

#[test]
fn multi_input_basic() {
    let editor = Editor {
        tabs: ImVector::from(vec!["a.rs".into(), "b.rs".into()]),
        ..Default::default()
    };

    let dashboard = Dashboard {
        user_name: "alice".into(),
        ..Default::default()
    };

    let header = combined_header((&editor).into(), (&dashboard).into());
    assert_eq!(header, "alice: 2 tabs open");
}

#[test]
fn multi_input_memoizes_on_irrelevant_change() {
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

    let _ = combined_header((&editor).into(), (&dashboard).into());

    editor.cursor_col = 42;
    let mut dashboard2 = dashboard.clone();
    dashboard2.theme = "light".into();

    let header = combined_header((&editor).into(), (&dashboard2).into());
    assert_eq!(header, "bob: 1 tabs open");
}

#[test]
fn multi_input_recomputes_on_relevant_change() {
    let mut editor = Editor {
        tabs: ImVector::from(vec!["a.rs".into()]),
        ..Default::default()
    };

    let dashboard = Dashboard {
        user_name: "carol".into(),
        ..Default::default()
    };

    let _ = combined_header((&editor).into(), (&dashboard).into());

    editor.tabs = ImVector::from(vec!["a.rs".into(), "b.rs".into(), "c.rs".into()]);
    let header = combined_header((&editor).into(), (&dashboard).into());
    assert_eq!(header, "carol: 3 tabs open");

    let dashboard2 = Dashboard {
        user_name: "dave".into(),
        ..Default::default()
    };
    let header2 = combined_header((&editor).into(), (&dashboard2).into());
    assert_eq!(header2, "dave: 3 tabs open");
}

#[test]
fn two_level_chaining() {
    let mut app = AppState {
        items: ImVector::from(vec!["foo".into(), "bar".into(), "baz".into()]),
        selected: Some(1),
        ..Default::default()
    };

    let summary = items_summary((&app).into());
    assert_eq!(summary.count, 3);
    assert_eq!(summary.current, "bar");

    let label = summary_label((&summary).into());
    assert_eq!(label, "3 items, viewing: bar");

    app.theme = "dark".into();
    let summary2 = items_summary((&app).into());
    assert_eq!(summary2.count, 3);

    app.selected = None;
    let summary3 = items_summary((&app).into());
    assert_eq!(summary3.count, 3);
    assert_eq!(summary3.current, "");

    let label3 = summary_label((&summary3).into());
    assert_eq!(label3, "3 items, none selected");
}

#[test]
fn atom_as_memo_input_via_value_ref() {
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
    // Generated wrapper releases the borrow before running the user body,
    // so sibling-memo calls re-enter safely.
    let r = Reentrant { value: 5 };
    assert_eq!(outer_memo(&r), 11);
    assert_eq!(outer_memo(&r), 11);
}

// ── Mixed input + value parameter tests ──

#[test]
fn mixed_value_after_input() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_value_after((&a).into(), 3), 30);
    assert_eq!(with_value_after((&a).into(), 3), 30);
    assert_eq!(with_value_after((&a).into(), 4), 40);
}

#[test]
fn mixed_value_before_input() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_value_before(5, (&a).into()), 50);
    assert_eq!(with_value_before(5, (&a).into()), 50);
    assert_eq!(with_value_before(7, (&a).into()), 70);
}

#[test]
fn mixed_multi_value() {
    let a = MixedAtom { base: 10 };
    assert_eq!(multi_value(1, (&a).into(), 3), 31);
    assert_eq!(multi_value(1, (&a).into(), 3), 31);
    assert_eq!(multi_value(2, (&a).into(), 3), 32);
    assert_eq!(multi_value(2, (&a).into(), 5), 52);
}

#[test]
fn mixed_input_change_invalidates() {
    let mut a = MixedAtom { base: 10 };
    assert_eq!(with_value_after((&a).into(), 2), 20);
    a.base = 100;
    assert_eq!(with_value_after((&a).into(), 2), 200);
}

#[test]
fn value_ref_str() {
    let a = MixedAtom { base: 42 };
    assert_eq!(with_str((&a).into(), "val"), "val=42");
    assert_eq!(with_str((&a).into(), "val"), "val=42");
    assert_eq!(with_str((&a).into(), "new"), "new=42");
    assert_eq!(with_str((&a).into(), "val"), "val=42");
}

#[test]
fn value_ref_bytes() {
    let a = MixedAtom { base: 10 };
    assert_eq!(with_bytes((&a).into(), &[1, 2, 3]), 13);
    assert_eq!(with_bytes((&a).into(), &[1, 2, 3]), 13);
    assert_eq!(with_bytes((&a).into(), &[1, 2, 3, 4]), 14);
}

#[test]
fn value_ref_with_owned_string() {
    let a = MixedAtom { base: 1 };
    let s: String = "hello".into();
    assert_eq!(with_str((&a).into(), &s), "hello=1");
}

// ══════════════════════════════════════════════════════════════════════
// Send assertion
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
    assert_eq!(arc_sum((&a).into()), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    assert_eq!(arc_sum((&a).into()), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    // Different Arc with identical contents — PartialEq still sees equality.
    a.data = Arc::new(vec![1u32, 2, 3, 4, 5]);
    assert_eq!(arc_sum((&a).into()), 15);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 1);

    // Contents differ — recompute.
    a.data = Arc::new(vec![1u32, 2, 3, 4, 5, 6]);
    assert_eq!(arc_sum((&a).into()), 21);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 2);

    // Same Arc as the most recent snapshot → ptr_eq fast path.
    let p = a.data.clone();
    a.data = p;
    assert_eq!(arc_sum((&a).into()), 21);
    assert_eq!(ARC_MEMO_COMPUTES.load(Ordering::SeqCst), 2);
}

// ══════════════════════════════════════════════════════════════════════
// Projection input tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn proj_input_basic() {
    let a = Outer {
        inner: Inner {
            value: 42,
            label: "hello".into(),
        },
        name: "test".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived((&a).into()), "test=42");
}

#[test]
fn proj_input_cache_hit() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived_for_hit((&a).into()), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);

    a.count = 999;
    assert_eq!(proj_derived_for_hit((&a).into()), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);

    a.inner.label = "changed".into();
    assert_eq!(proj_derived_for_hit((&a).into()), "foo=10");
    assert_eq!(PROJ_HIT_COMPUTES.load(Ordering::SeqCst), 1);
}

#[test]
fn proj_input_cache_miss() {
    let mut a = Outer {
        inner: Inner {
            value: 10,
            label: "x".into(),
        },
        name: "foo".into(),
        ..Default::default()
    };
    assert_eq!(proj_derived_for_miss((&a).into()), "foo=10");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 1);

    a.inner.value = 20;
    assert_eq!(proj_derived_for_miss((&a).into()), "foo=20");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 2);

    a.name = "bar".into();
    assert_eq!(proj_derived_for_miss((&a).into()), "bar=20");
    assert_eq!(PROJ_MISS_COMPUTES.load(Ordering::SeqCst), 3);
}

#[test]
fn proj_input_mixed_with_regular() {
    let outer = Outer {
        inner: Inner {
            value: 5,
            label: "x".into(),
        },
        name: "hello".into(),
        ..Default::default()
    };
    let mixed = MixedAtom { base: 7 };
    assert_eq!(
        proj_plus_regular((&outer).into(), (&mixed).into()),
        "hello-7"
    );
}

#[test]
fn proj_input_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Outer>();
}

// ══════════════════════════════════════════════════════════════════════
// Copy-by-value + explicit &T in standalone input
// ══════════════════════════════════════════════════════════════════════

#[test]
fn proj_can_transform_values() {
    let a = OverrideAtom {
        n: 7,
        tag: "hi".into(),
    };
    assert_eq!(override_display((&a).into()), "hi:14");
}

#[test]
fn copy_by_value_in_input() {
    let a = CopyTest {
        x: 10,
        y: 20,
        ..Default::default()
    };
    assert_eq!(copy_mix_sum((&a).into()), 30);
}

// ══════════════════════════════════════════════════════════════════════
// VALUE-KEYED CACHE tests
// ══════════════════════════════════════════════════════════════════════

#[test]
fn ping_pong_cache_hit_across_values() {
    CACHE_BEHAVIOR_COMPUTES.store(0, Ordering::SeqCst);

    let a1 = CacheBehaviorAtom { value: 10 };
    let a2 = CacheBehaviorAtom { value: 20 };

    assert_eq!(cache_behavior((&a1).into()), 20);
    assert_eq!(cache_behavior((&a2).into()), 40);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);

    assert_eq!(cache_behavior((&a1).into()), 20);
    assert_eq!(cache_behavior((&a2).into()), 40);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);

    let a1_clone = CacheBehaviorAtom { value: 10 };
    assert_eq!(cache_behavior((&a1_clone).into()), 20);
    assert_eq!(CACHE_BEHAVIOR_COMPUTES.load(Ordering::SeqCst), 2);
}

#[test]
fn single_strategy_last_call_only() {
    SINGLE_COMPUTES.store(0, Ordering::SeqCst);

    let a = CacheBehaviorAtom { value: 1 };
    let b = CacheBehaviorAtom { value: 2 };

    assert_eq!(single_memo((&a).into()), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 1);

    assert_eq!(single_memo((&a).into()), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 1);

    assert_eq!(single_memo((&b).into()), 9);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 2);

    assert_eq!(single_memo((&a).into()), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 3);

    let a_clone = CacheBehaviorAtom { value: 1 };
    assert_eq!(single_memo((&a_clone).into()), 8);
    assert_eq!(SINGLE_COMPUTES.load(Ordering::SeqCst), 3);
}

#[test]
fn lru_evicts_least_recently_used() {
    LRU_COMPUTES.store(0, Ordering::SeqCst);

    let a = CacheBehaviorAtom { value: 1 };
    let b = CacheBehaviorAtom { value: 2 };
    let c = CacheBehaviorAtom { value: 3 };

    assert_eq!(lru_memo((&a).into()), 1001);
    assert_eq!(lru_memo((&b).into()), 1002);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 2);

    assert_eq!(lru_memo((&a).into()), 1001);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 2);

    assert_eq!(lru_memo((&c).into()), 1003);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 3);

    assert_eq!(lru_memo((&a).into()), 1001);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 3);

    assert_eq!(lru_memo((&b).into()), 1002);
    assert_eq!(LRU_COMPUTES.load(Ordering::SeqCst), 4);
}

// ══════════════════════════════════════════════════════════════════════
// 16. NESTED INPUTS — a parent drv::Input with child drv::Input fields.
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NestedAtom {
    pub a: ImVector<u32>,
    pub b: ImVector<u32>,
    pub unrelated: u32,
}

#[derive(drv::Input)]
struct NestChildA<'a> {
    pub a: &'a ImVector<u32>,
}

impl<'a> From<&'a NestedAtom> for NestChildA<'a> {
    fn from(n: &'a NestedAtom) -> Self {
        Self { a: &n.a }
    }
}

#[derive(drv::Input)]
struct NestChildB<'a> {
    pub b: &'a ImVector<u32>,
}

impl<'a> From<&'a NestedAtom> for NestChildB<'a> {
    fn from(n: &'a NestedAtom) -> Self {
        Self { b: &n.b }
    }
}

#[derive(drv::Input)]
struct NestParent<'a> {
    pub child_a: NestChildA<'a>,
    pub child_b: NestChildB<'a>,
}

impl<'a> From<&'a NestedAtom> for NestParent<'a> {
    fn from(n: &'a NestedAtom) -> Self {
        Self {
            child_a: NestChildA::from(n),
            child_b: NestChildB::from(n),
        }
    }
}

static NESTED_SUM_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn nested_sum<'a>(input: NestParent<'a>) -> u32 {
    NESTED_SUM_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.child_a.a.iter().sum::<u32>() + input.child_b.b.iter().sum::<u32>()
}

#[test]
fn nested_inputs_compose() {
    NESTED_SUM_COMPUTES.store(0, Ordering::SeqCst);

    let mut atom = NestedAtom {
        a: ImVector::from(vec![1u32, 2, 3]),
        b: ImVector::from(vec![10u32, 20]),
        unrelated: 0,
    };

    assert_eq!(nested_sum((&atom).into()), 36);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 1);

    // Same input → cache hit.
    assert_eq!(nested_sum((&atom).into()), 36);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 1);

    // Mutating unrelated → still a hit (not projected).
    atom.unrelated = 99;
    assert_eq!(nested_sum((&atom).into()), 36);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 1);

    // Mutating a → miss.
    atom.a.push_back(4);
    assert_eq!(nested_sum((&atom).into()), 40);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 2);

    // Cloning atom + pushing to clone diverges pointers → miss.
    let mut clone = atom.clone();
    clone.a.push_back(5);
    assert_eq!(nested_sum((&clone).into()), 45);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 3);

    // Going back to the earlier-cached shape — miss again because cache
    // is single-slot.
    assert_eq!(nested_sum((&atom).into()), 40);
    assert_eq!(NESTED_SUM_COMPUTES.load(Ordering::SeqCst), 4);
}

// ── Mixed parent: nested drv::Input + reference field + plain owned field ──

#[derive(drv::Input)]
struct NestMixed<'a> {
    pub child_a: NestChildA<'a>,
    pub label: &'a String,
    pub tag: u32,
}

impl<'a> NestMixed<'a> {
    fn new(atom: &'a NestedAtom, label: &'a String, tag: u32) -> Self {
        Self {
            child_a: NestChildA::from(atom),
            label,
            tag,
        }
    }
}

static MIXED_NESTED_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn mixed_nested<'a>(input: NestMixed<'a>) -> String {
    MIXED_NESTED_COMPUTES.fetch_add(1, Ordering::SeqCst);
    format!(
        "{}:{}:{}",
        input.label,
        input.tag,
        input.child_a.a.iter().sum::<u32>()
    )
}

#[test]
fn nested_inputs_alongside_plain_and_reference_fields() {
    MIXED_NESTED_COMPUTES.store(0, Ordering::SeqCst);

    let atom = NestedAtom {
        a: ImVector::from(vec![1u32, 2, 3]),
        b: ImVector::new(),
        unrelated: 0,
    };
    let label = String::from("hello");

    assert_eq!(mixed_nested(NestMixed::new(&atom, &label, 7)), "hello:7:6");
    assert_eq!(MIXED_NESTED_COMPUTES.load(Ordering::SeqCst), 1);

    // Same → hit.
    assert_eq!(mixed_nested(NestMixed::new(&atom, &label, 7)), "hello:7:6");
    assert_eq!(MIXED_NESTED_COMPUTES.load(Ordering::SeqCst), 1);

    // Change tag (plain owned field) → miss.
    assert_eq!(mixed_nested(NestMixed::new(&atom, &label, 8)), "hello:8:6");
    assert_eq!(MIXED_NESTED_COMPUTES.load(Ordering::SeqCst), 2);

    // Change label (reference field) → miss.
    let label2 = String::from("world");
    assert_eq!(mixed_nested(NestMixed::new(&atom, &label2, 8)), "world:8:6");
    assert_eq!(MIXED_NESTED_COMPUTES.load(Ordering::SeqCst), 3);
}

// ── Three-level nesting: grandparent → parent → child ──

#[derive(drv::Input)]
struct NestGrand<'a> {
    pub inner: NestParent<'a>,
    pub extra: u32,
}

static GRAND_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn grand_sum<'a>(input: NestGrand<'a>) -> u32 {
    GRAND_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.inner.child_a.a.iter().sum::<u32>()
        + input.inner.child_b.b.iter().sum::<u32>()
        + input.extra
}

#[test]
fn three_level_nested_inputs() {
    GRAND_COMPUTES.store(0, Ordering::SeqCst);

    let mut atom = NestedAtom {
        a: ImVector::from(vec![1u32, 2]),
        b: ImVector::from(vec![10u32]),
        unrelated: 0,
    };

    let g = NestGrand {
        inner: NestParent::from(&atom),
        extra: 100,
    };
    assert_eq!(grand_sum(g), 113);
    assert_eq!(GRAND_COMPUTES.load(Ordering::SeqCst), 1);

    // Same shape → hit.
    let g2 = NestGrand {
        inner: NestParent::from(&atom),
        extra: 100,
    };
    assert_eq!(grand_sum(g2), 113);
    assert_eq!(GRAND_COMPUTES.load(Ordering::SeqCst), 1);

    // Change `extra` at grandparent level → miss.
    let g3 = NestGrand {
        inner: NestParent::from(&atom),
        extra: 200,
    };
    assert_eq!(grand_sum(g3), 213);
    assert_eq!(GRAND_COMPUTES.load(Ordering::SeqCst), 2);

    // Change at leaf (atom.a) → miss.
    atom.a.push_back(3);
    let g4 = NestGrand {
        inner: NestParent::from(&atom),
        extra: 200,
    };
    assert_eq!(grand_sum(g4), 216);
    assert_eq!(GRAND_COMPUTES.load(Ordering::SeqCst), 3);
}

// ── Nested input passed by reference: `fn memo(&NestParent<'a>)` ──

static NESTED_BY_REF_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn nested_by_ref<'a>(input: &NestParent<'a>) -> u32 {
    NESTED_BY_REF_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.child_a.a.len() as u32 + input.child_b.b.len() as u32
}

#[test]
fn nested_input_by_reference() {
    NESTED_BY_REF_COMPUTES.store(0, Ordering::SeqCst);

    let atom = NestedAtom {
        a: ImVector::from(vec![1u32, 2, 3]),
        b: ImVector::from(vec![10u32, 20]),
        unrelated: 0,
    };

    let parent = NestParent::from(&atom);
    assert_eq!(nested_by_ref(&parent), 5);
    assert_eq!(NESTED_BY_REF_COMPUTES.load(Ordering::SeqCst), 1);

    let parent2 = NestParent::from(&atom);
    assert_eq!(nested_by_ref(&parent2), 5);
    assert_eq!(NESTED_BY_REF_COMPUTES.load(Ordering::SeqCst), 1);
}

// ══════════════════════════════════════════════════════════════════════
// 17. TUPLE-STRUCT NEWTYPES as drv::Input.
// ══════════════════════════════════════════════════════════════════════

/// Newtype wrapping a primitive — common pattern for typed ids.
#[derive(Debug, Copy, Clone, PartialEq, drv::Input)]
pub struct UserId(pub u64);

/// Newtype wrapping an owned container.
#[derive(Debug, Clone, PartialEq, drv::Input)]
pub struct Tags(pub Vec<String>);

#[drv::memo(single)]
fn user_id_doubled(id: UserId) -> u64 {
    id.0 * 2
}

#[drv::memo(single)]
fn tag_count(t: &Tags) -> usize {
    t.0.len()
}

#[test]
fn tuple_struct_newtype_id_input() {
    let id = UserId(21);
    assert_eq!(user_id_doubled(id), 42);
    assert_eq!(user_id_doubled(UserId(21)), 42); // hit
    assert_eq!(user_id_doubled(UserId(100)), 200); // miss
}

#[test]
fn tuple_struct_newtype_with_owned_container() {
    let t = Tags(vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(tag_count(&t), 3);
    let t2 = Tags(vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(tag_count(&t2), 3); // hit by value equality
    let t3 = Tags(vec!["a".into(), "b".into()]);
    assert_eq!(tag_count(&t3), 2); // miss
}

/// Multi-field tuple struct.
#[derive(Debug, Clone, PartialEq, drv::Input)]
pub struct Pair(pub u32, pub String);

#[drv::memo(single)]
fn pair_label(p: &Pair) -> String {
    format!("{}:{}", p.0, p.1)
}

#[test]
fn tuple_struct_multi_field() {
    let p = Pair(7, "hi".into());
    assert_eq!(pair_label(&p), "7:hi");
    assert_eq!(pair_label(&Pair(7, "hi".into())), "7:hi"); // hit
    assert_eq!(pair_label(&Pair(7, "bye".into())), "7:bye"); // miss
    assert_eq!(pair_label(&Pair(8, "bye".into())), "8:bye"); // miss
}

// ══════════════════════════════════════════════════════════════════════
// 18. GENERIC drv::Input — type params, const generics, where-clauses.
// ══════════════════════════════════════════════════════════════════════

#[derive(drv::Input)]
struct GenericOne<'a, T: drv::ToStatic + Clone + PartialEq + 'static> {
    pub xs: &'a Vec<T>,
}

#[drv::memo(single)]
fn generic_sum<'a>(input: GenericOne<'a, u32>) -> u32 {
    input.xs.iter().sum()
}

#[derive(drv::Input)]
struct GenericWhere<T>
where
    T: drv::ToStatic + Clone + PartialEq + 'static,
{
    pub a: T,
    pub b: T,
}

#[drv::memo(single)]
fn generic_where_eq(input: GenericWhere<u32>) -> bool {
    input.a == input.b
}

#[derive(drv::Input)]
struct WithArray {
    pub arr: [u32; 4],
}

#[drv::memo(single)]
fn array_sum(input: &WithArray) -> u32 {
    input.arr.iter().sum()
}

#[test]
fn generic_type_params() {
    let data = vec![1u32, 2, 3];
    assert_eq!(generic_sum(GenericOne { xs: &data }), 6);
    assert_eq!(generic_sum(GenericOne { xs: &data }), 6);
}

#[test]
fn generic_where_clause() {
    assert!(generic_where_eq(GenericWhere { a: 7, b: 7 }));
    assert!(!generic_where_eq(GenericWhere { a: 7, b: 8 }));
}

#[test]
fn const_generic_array_field() {
    let w = WithArray { arr: [1, 2, 3, 4] };
    assert_eq!(array_sum(&w), 10);
}

// ══════════════════════════════════════════════════════════════════════
// 19. CONTAINER-NESTED drv::Input — Vec<MyInput<'a>> etc.
// ══════════════════════════════════════════════════════════════════════

#[derive(drv::Input)]
struct Item<'a> {
    pub name: &'a String,
    pub qty: u32,
}

#[derive(drv::Input)]
struct Basket<'a> {
    pub items: Vec<Item<'a>>,
}

static BASKET_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn basket_total<'a>(input: Basket<'a>) -> u32 {
    BASKET_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.items.iter().map(|i| i.qty).sum()
}

#[test]
fn vec_of_drv_input() {
    BASKET_COMPUTES.store(0, Ordering::SeqCst);

    let apple = "apple".to_string();
    let banana = "banana".to_string();

    let basket = Basket {
        items: vec![
            Item {
                name: &apple,
                qty: 3,
            },
            Item {
                name: &banana,
                qty: 5,
            },
        ],
    };
    assert_eq!(basket_total(basket), 8);
    assert_eq!(BASKET_COMPUTES.load(Ordering::SeqCst), 1);

    // Rebuild an equivalent basket — should hit cache.
    let basket2 = Basket {
        items: vec![
            Item {
                name: &apple,
                qty: 3,
            },
            Item {
                name: &banana,
                qty: 5,
            },
        ],
    };
    assert_eq!(basket_total(basket2), 8);
    assert_eq!(BASKET_COMPUTES.load(Ordering::SeqCst), 1);

    // Change a qty → miss.
    let basket3 = Basket {
        items: vec![
            Item {
                name: &apple,
                qty: 4,
            },
            Item {
                name: &banana,
                qty: 5,
            },
        ],
    };
    assert_eq!(basket_total(basket3), 9);
    assert_eq!(BASKET_COMPUTES.load(Ordering::SeqCst), 2);
}

// ══════════════════════════════════════════════════════════════════════
// 20. COVERAGE FILLERS — unit struct, std maps/sets, tuple fields,
//     array-of-drv::Input, Option<drv::Input>.
// ══════════════════════════════════════════════════════════════════════

#[derive(drv::Input)]
pub struct UnitMarker;

#[drv::memo(single)]
fn unit_marker_memo(marker: UnitMarker) -> u32 {
    let _ = marker;
    7
}

#[test]
fn unit_struct_derive() {
    assert_eq!(unit_marker_memo(UnitMarker), 7);
    assert_eq!(unit_marker_memo(UnitMarker), 7);
}

#[derive(drv::Input)]
struct TupleField<'a> {
    pub pair: (u32, &'a String),
}

#[drv::memo(single)]
fn tuple_field_memo<'a>(input: TupleField<'a>) -> String {
    format!("{}:{}", input.pair.0, input.pair.1)
}

#[test]
fn tuple_type_as_field() {
    let s = "hello".to_string();
    assert_eq!(tuple_field_memo(TupleField { pair: (42, &s) }), "42:hello");
    assert_eq!(tuple_field_memo(TupleField { pair: (42, &s) }), "42:hello");
}

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(drv::Input)]
struct StdMaps<'a> {
    pub m: &'a HashMap<String, u32>,
    pub b: &'a BTreeMap<u32, String>,
    pub s: &'a HashSet<u32>,
    pub t: &'a BTreeSet<u32>,
}

#[drv::memo(single)]
fn std_maps_sum<'a>(input: StdMaps<'a>) -> u32 {
    input.m.values().sum::<u32>()
        + input.b.keys().sum::<u32>()
        + input.s.iter().sum::<u32>()
        + input.t.iter().sum::<u32>()
}

#[test]
fn std_hashmap_hashset_btreemap_btreeset_fields() {
    let m: HashMap<String, u32> = [("a".to_string(), 1), ("b".to_string(), 2)]
        .into_iter()
        .collect();
    let b: BTreeMap<u32, String> = [(10u32, "x".to_string()), (20u32, "y".to_string())]
        .into_iter()
        .collect();
    let s: HashSet<u32> = [100u32, 200].into_iter().collect();
    let t: BTreeSet<u32> = [1000u32, 2000].into_iter().collect();

    assert_eq!(
        std_maps_sum(StdMaps {
            m: &m,
            b: &b,
            s: &s,
            t: &t
        }),
        3333
    );
    assert_eq!(
        std_maps_sum(StdMaps {
            m: &m,
            b: &b,
            s: &s,
            t: &t
        }),
        3333
    );
}

#[derive(drv::Input)]
struct ArcArray<'a> {
    pub arr: &'a [Arc<Vec<u32>>; 3],
}

#[drv::memo(single)]
fn arc_array_sum<'a>(input: ArcArray<'a>) -> u32 {
    input.arr.iter().map(|a| a.iter().sum::<u32>()).sum()
}

#[test]
fn array_of_nontrivial_element() {
    let arr: [Arc<Vec<u32>>; 3] = [
        Arc::new(vec![1, 2]),
        Arc::new(vec![3, 4]),
        Arc::new(vec![5, 6]),
    ];
    assert_eq!(arc_array_sum(ArcArray { arr: &arr }), 21);
    assert_eq!(arc_array_sum(ArcArray { arr: &arr }), 21);
}

#[derive(drv::Input)]
struct OptionalChild<'a> {
    pub maybe: Option<NestChildA<'a>>,
    pub flag: bool,
}

#[drv::memo(single)]
fn optional_child_len<'a>(input: OptionalChild<'a>) -> usize {
    match input.maybe {
        Some(c) if input.flag => c.a.len(),
        _ => 0,
    }
}

#[test]
fn option_of_drv_input() {
    let v = ImVector::from(vec![1u32, 2, 3, 4]);
    let child = NestChildA { a: &v };

    assert_eq!(
        optional_child_len(OptionalChild {
            maybe: Some(child),
            flag: true,
        }),
        4
    );
    assert_eq!(
        optional_child_len(OptionalChild {
            maybe: None,
            flag: true,
        }),
        0
    );
}

// ══════════════════════════════════════════════════════════════════════
// 21. MAP-VALUED drv::Input — HashMap<K, MyInput<'a>> etc.
// ══════════════════════════════════════════════════════════════════════

#[derive(Clone, drv::Input)]
struct Row<'a> {
    pub label: &'a String,
    pub qty: u32,
}

#[derive(drv::Input)]
struct RowHashMap<'a> {
    pub rows: HashMap<String, Row<'a>>,
}

static ROW_MAP_COMPUTES: AtomicUsize = AtomicUsize::new(0);

#[drv::memo(single)]
fn row_map_total<'a>(input: RowHashMap<'a>) -> u32 {
    ROW_MAP_COMPUTES.fetch_add(1, Ordering::SeqCst);
    input.rows.values().map(|r| r.qty).sum()
}

#[test]
fn hashmap_of_drv_input() {
    ROW_MAP_COMPUTES.store(0, Ordering::SeqCst);

    let apple = "apple".to_string();
    let banana = "banana".to_string();

    let rows: HashMap<String, Row<'_>> = [
        (
            "fruit".to_string(),
            Row {
                label: &apple,
                qty: 5,
            },
        ),
        (
            "long".to_string(),
            Row {
                label: &banana,
                qty: 3,
            },
        ),
    ]
    .into_iter()
    .collect();

    assert_eq!(row_map_total(RowHashMap { rows: rows.clone() }), 8);
    assert_eq!(ROW_MAP_COMPUTES.load(Ordering::SeqCst), 1);

    // Same content → cache hit.
    assert_eq!(row_map_total(RowHashMap { rows: rows.clone() }), 8);
    assert_eq!(ROW_MAP_COMPUTES.load(Ordering::SeqCst), 1);

    // Change a qty → miss.
    let mut mutated = rows.clone();
    mutated.get_mut("fruit").unwrap().qty = 50;
    assert_eq!(row_map_total(RowHashMap { rows: mutated }), 53);
    assert_eq!(ROW_MAP_COMPUTES.load(Ordering::SeqCst), 2);

    // Remove an entry (changes length) → miss.
    let mut shrunk = rows.clone();
    shrunk.remove("long");
    assert_eq!(row_map_total(RowHashMap { rows: shrunk }), 5);
    assert_eq!(ROW_MAP_COMPUTES.load(Ordering::SeqCst), 3);

    // Different key, same value count → miss (pigeonhole: length equal but
    // one key differs, so `self.iter().all(other.get(k).is_some())` fails).
    let orange = "orange".to_string();
    let rekeyed: HashMap<String, Row<'_>> = [
        (
            "fruit".to_string(),
            Row {
                label: &apple,
                qty: 5,
            },
        ),
        (
            "citrus".to_string(),
            Row {
                label: &orange,
                qty: 3,
            },
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(row_map_total(RowHashMap { rows: rekeyed }), 8);
    assert_eq!(ROW_MAP_COMPUTES.load(Ordering::SeqCst), 4);
}

#[derive(drv::Input)]
struct RowBTreeMap<'a> {
    pub rows: BTreeMap<u32, Row<'a>>,
}

#[drv::memo(single)]
fn row_btree_total<'a>(input: RowBTreeMap<'a>) -> u32 {
    input.rows.values().map(|r| r.qty).sum()
}

#[test]
fn btreemap_of_drv_input() {
    let a = "x".to_string();
    let b = "y".to_string();

    let rows: BTreeMap<u32, Row<'_>> = [
        (1u32, Row { label: &a, qty: 10 }),
        (2u32, Row { label: &b, qty: 20 }),
    ]
    .into_iter()
    .collect();

    assert_eq!(row_btree_total(RowBTreeMap { rows: rows.clone() }), 30);
    assert_eq!(row_btree_total(RowBTreeMap { rows }), 30);
}

#[test]
fn preprojected_input_at_call_site() {
    // Callers can pass a pre-projected input directly (by value) instead of
    // letting the memo wrapper convert `&atom` via the From impl.
    let atom = CacheBehaviorAtom { value: 4 };
    let input: CbInput<'_> = (&atom).into();
    assert_eq!(preprojected(input), 20);

    // `&atom` path works too (ergonomic form).
    assert_eq!(preprojected((&atom).into()), 20);

    // Different underlying atom, same value → still a hit (value-keyed).
    let other = CacheBehaviorAtom { value: 4 };
    assert_eq!(preprojected((&other).into()), 20);
}
