pub mod actions;
pub mod details;
pub mod filter;
pub mod snapshot;
pub mod sort;
pub mod versioned;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::elasticsearch::stats::RateTracker;
use crate::elasticsearch::{AuthConfig, EsClient};
use crate::error::Result;
use crate::models::{ClusterHealth, IndexRate, NodeStats};
use crate::ui::types::Colormap;
use tokio::sync::mpsc;

use self::actions::Action;
use self::details::DetailsState;
use self::filter::FilterState;
use self::snapshot::SnapshotState;
use self::sort::{NodeSortState, SortState};
use self::versioned::Versioned;

// One history sample is drawn per terminal column, so this needs to comfortably
// exceed the width of a wide terminal or the graph can never fill its panel.
// At the default 5s refresh, 300 samples is ~25 minutes of history.
const MAX_HISTORY_POINTS: usize = 300;
pub const MIN_REFRESH_SECS: u64 = 1;
pub const MAX_REFRESH_SECS: u64 = 60;

/// Everything one refresh tick brings back. Each part succeeds or fails on
/// its own: a `_cluster/health` or `_nodes/stats` failure (a role without the
/// cluster `monitor` privilege, one unresponsive node) must not stop the
/// indices table, header rate and graph from updating.
pub struct FetchPayload {
    /// Each index with its raw rate sample for this tick; `None` where there
    /// is no honest sample yet (see `RateTracker::rates_from_snapshot`).
    pub indices: Result<Vec<(IndexRate, Option<f64>)>>,
    pub health: Result<ClusterHealth>,
    /// `None` when the nodes panel wasn't on screen, so `_nodes/stats` was
    /// never requested — distinct from `Some(Ok(vec![]))`, which would mean
    /// the cluster genuinely reported no nodes.
    pub nodes: Option<Result<Vec<NodeStats>>>,
}

/// Which table occupies the main panel. The two are alternatives rather than
/// stacked panels: the nodes view replaces the indices view in the same area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainPanel {
    #[default]
    Indices,
    Nodes,
}

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Aggregated metrics for cluster-wide indexing performance.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClusterMetrics {
    /// Total documents indexed per second across all indices
    pub rate_per_sec: f64,
    /// Total bytes indexed per second across all indices
    pub bytes_per_sec: f64,
}

/// The indices surviving every active visibility filter (exclusions, system
/// indices, the filter box) plus their aggregated metrics, computed together
/// in one pass — see `App::visible_summary`.
pub struct VisibleSummary<'a> {
    pub indices: Vec<&'a IndexRate>,
    pub metrics: ClusterMetrics,
}

/// Everything the visible set is derived from. While none of it changes,
/// neither does the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleKey {
    indices: u64,
    excluded: u64,
    filter: u64,
    show_system_indices: bool,
    main_panel: MainPanel,
}

/// The last visibility pass, kept until something it was derived from
/// changes.
///
/// The pass is cheap with an empty filter box but not with a filter in it:
/// jq mode serializes and evaluates every row (~33ms at 50k indices) and a
/// true regex runs over every name (~15ms). The loop redraws ~20x a second
/// and every cursor movement looks the selection up in the visible list, so
/// without this, a filter that hadn't changed in minutes was re-run on every
/// frame and every keypress — most of a core spent idle, and visible lag on
/// each key.
#[derive(Default)]
struct VisibleCache {
    /// What the cached pass was computed from; `None` before the first one.
    key: Option<VisibleKey>,
    /// Positions in `App::indices` of the rows that passed, in display order.
    positions: Vec<usize>,
    metrics: ClusterMetrics,
}

/// Main application state and logic controller.
///
/// This struct holds all the state necessary to render the TUI and handles
/// all user actions and background data fetching.
pub struct App {
    /// Versioned so the memoized visible set (`visible_summary`) notices any
    /// change to it, however it is made.
    pub indices: Versioned<Vec<IndexRate>>,
    pub running: bool,
    pub error: Option<String>,
    pub loading: bool,
    pub spinner_frame: usize,
    pub refresh_interval: Duration,
    pub last_refresh: Option<Instant>,
    pub rate_history: VecDeque<f64>,
    pub es_url: String,
    pub fetch_start: Option<Instant>,
    pub last_fetch_duration: Option<Duration>,
    pub show_graph: bool,
    pub show_health: bool,
    /// Whether the main table panel is on screen at all (toggled with `3`).
    /// What it *contains* is `main_panel`.
    pub show_main_panel: bool,
    pub main_panel: MainPanel,
    pub show_system_indices: bool,
    pub paused: bool,
    /// Name of the currently selected index.
    ///
    /// Tracked by name rather than by row position because the list is
    /// re-sorted on every refresh (by rate, by default), so a positional
    /// cursor would silently slide onto a different index between ticks.
    pub selected_name: Option<String>,
    /// First visible table row. Persisted across frames so the table only
    /// scrolls when the selection would leave the viewport — see `ui::draw`.
    pub table_offset: usize,
    pub nodes: Vec<NodeStats>,
    /// Node selection, keyed by name for the same reason as `selected_name`.
    pub selected_node: Option<String>,
    pub node_table_offset: usize,
    /// Versioned for the same reason as `indices`.
    pub excluded_indices: Versioned<HashSet<String>>,
    pub show_help_popup: bool,
    pub help_scroll: usize,
    pub colormap: Colormap,
    pub rate_samples: usize,
    pub cluster_health: ClusterHealth,

    // Sub-states
    pub sort: SortState,
    pub node_sort: NodeSortState,
    pub filter: FilterState,
    pub details: DetailsState,
    /// Frozen baseline the tables mark their metrics against. Inert until
    /// `s` is pressed.
    pub snapshot: SnapshotState,

    index_rate_history: HashMap<String, VecDeque<f64>>,
    /// Interior mutability so the read-only `visible_summary` can memoize.
    /// No borrow of it outlives the method that takes it.
    visible_cache: RefCell<VisibleCache>,
    /// Stateless and cheap to clone: each background task gets its own copy,
    /// so a details fetch never waits behind a slow polling tick.
    es_client: EsClient,
    /// The previous `_stats` reading rates are diffed against. Only the
    /// polling task touches it, and never across an `.await`.
    rate_tracker: Arc<Mutex<RateTracker>>,
    /// Set when something wants a fresh fetch while one is already in
    /// flight (switching to the nodes panel mid-fetch): that fetch didn't
    /// ask for what is now needed, so another starts as soon as it lands.
    refetch_pending: bool,
    fetch_rx: mpsc::Receiver<FetchPayload>,
    fetch_tx: mpsc::Sender<FetchPayload>,
}

impl App {
    /// Creates a new App instance with the given configuration.
    ///
    /// This initializes the Elasticsearch client and background channels.
    pub fn new(
        base_url: String,
        auth: AuthConfig,
        insecure: bool,
        ca_cert: Option<PathBuf>,
        refresh_secs: u64,
        colormap: Colormap,
        rate_samples: usize,
    ) -> Result<Self> {
        let es_client = EsClient::new(&base_url, auth, insecure, ca_cert)?;
        let (fetch_tx, fetch_rx) = mpsc::channel(1);

        Ok(Self {
            indices: Versioned::default(),
            running: true,
            error: None,
            loading: false,
            spinner_frame: 0,
            refresh_interval: Duration::from_secs(refresh_secs),
            last_refresh: None,
            rate_history: VecDeque::with_capacity(MAX_HISTORY_POINTS),
            es_url: es_client.display_url(),
            fetch_start: None,
            last_fetch_duration: None,
            show_graph: true,
            show_health: true,
            show_main_panel: true,
            main_panel: MainPanel::default(),
            show_system_indices: false,
            paused: false,
            selected_name: None,
            table_offset: 0,
            nodes: Vec::new(),
            selected_node: None,
            node_table_offset: 0,
            excluded_indices: Versioned::default(),
            show_help_popup: false,
            help_scroll: 0,
            colormap,
            rate_samples: rate_samples.max(1), // At least 1 sample
            cluster_health: ClusterHealth::default(),

            sort: SortState::default(),
            node_sort: NodeSortState::default(),
            filter: FilterState::default(),
            details: DetailsState::new(),
            snapshot: SnapshotState::default(),

            index_rate_history: HashMap::new(),
            visible_cache: RefCell::default(),
            es_client,
            rate_tracker: Arc::new(Mutex::new(RateTracker::default())),
            refetch_pending: false,
            fetch_rx,
            fetch_tx,
        })
    }

    // Advance the spinner animation (call on each frame when loading)
    pub fn tick_spinner(&mut self) {
        if self.loading {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
        }
    }

    // Get the current spinner character (spinner when loading, checkmark when idle)
    pub fn spinner_char(&self) -> char {
        if self.loading {
            SPINNER_FRAMES[self.spinner_frame]
        } else {
            '✓' // Checkmark when idle
        }
    }

    /// Returns whether an index survives all active visibility filters
    /// (excluded indices, system indices, jq filter). This is the single
    /// source of truth for "is this index shown" — `visible_summary` and
    /// `filtered_indices` both delegate to it so the checks can't drift.
    ///
    /// The filter box applies to whichever table is in the main panel, so a
    /// node-name filter typed on the nodes panel must not also narrow the
    /// indices — that would silently zero the header's rate and the graph,
    /// which are computed from this set whichever panel is showing.
    fn is_visible(&self, index: &IndexRate) -> bool {
        if self.excluded_indices.contains(&index.name) {
            return false;
        }
        if !self.show_system_indices && is_system_index(&index.name) {
            return false;
        }
        self.main_panel != MainPanel::Indices || self.filter.is_match(index)
    }

    fn visible_key(&self) -> VisibleKey {
        VisibleKey {
            indices: self.indices.version(),
            excluded: self.excluded_indices.version(),
            filter: self.filter.version(),
            show_system_indices: self.show_system_indices,
            main_panel: self.main_panel,
        }
    }

    /// Re-runs the visibility pass over every index, but only if something
    /// it depends on has changed since the last one. Every input is either
    /// versioned or compared by value in `VisibleKey`, so there is no
    /// invalidation call for a new mutation site to forget.
    fn refresh_visible(&self) {
        let key = self.visible_key();
        if self.visible_cache.borrow().key == Some(key) {
            return;
        }

        let mut positions = Vec::with_capacity(self.indices.len());
        let mut metrics = ClusterMetrics::default();

        for (position, index) in self.indices.iter().enumerate() {
            if !self.is_visible(index) {
                continue;
            }

            metrics.rate_per_sec += index.rate_per_sec;
            if index.doc_count > 0 {
                let avg_doc_size = index.size_bytes as f64 / index.doc_count as f64;
                metrics.bytes_per_sec += avg_doc_size * index.rate_per_sec;
            }

            positions.push(position);
        }

        self.visible_cache.replace(VisibleCache {
            key: Some(key),
            positions,
            metrics,
        });
    }

    /// The visible indices and their aggregated cluster metrics.
    ///
    /// Memoized: the filtering pass runs once per change to its inputs (a
    /// refresh tick, an edit to the filter, an exclusion), not once per call,
    /// so calling this every frame and on every keypress costs only
    /// collecting references to the surviving rows.
    pub fn visible_summary(&self) -> VisibleSummary<'_> {
        self.refresh_visible();
        let cache = self.visible_cache.borrow();
        VisibleSummary {
            indices: cache
                .positions
                .iter()
                .map(|&position| &self.indices[position])
                .collect(),
            metrics: cache.metrics,
        }
    }

    pub fn total_cluster_rate(&self) -> f64 {
        self.refresh_visible();
        self.visible_cache.borrow().metrics.rate_per_sec
    }

    // Starts a background fetch of index rates from Elasticsearch.
    pub fn start_fetch(&mut self) {
        if self.loading {
            return;
        }

        self.loading = true;
        self.fetch_start = Some(Instant::now());
        let client = self.es_client.clone();
        let tracker = Arc::clone(&self.rate_tracker);
        let tx = self.fetch_tx.clone();
        // `_nodes/stats` is only worth requesting while its panel is on
        // screen; the indices/health calls stay unconditional because the
        // header rate and the cluster graph are driven by them whichever
        // panel is showing.
        let want_nodes = self.nodes_focused();

        tokio::spawn(async move {
            let tick = client.fetch_tick(want_nodes).await;
            let indices = tick.snapshot.map(|(now, snapshot)| {
                tracker
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .rates_from_snapshot(now, snapshot)
            });

            let _ = tx
                .send(FetchPayload {
                    indices,
                    health: tick.health,
                    nodes: tick.nodes,
                })
                .await;
        });
    }

    // Check for fetch results (non-blocking)
    pub fn poll_fetch_result(&mut self) {
        match self.fetch_rx.try_recv() {
            Ok(payload) => {
                self.loading = false;
                self.last_refresh = Some(Instant::now());

                if let Some(start) = self.fetch_start.take() {
                    self.last_fetch_duration = Some(start.elapsed());
                }

                self.apply_fetch(payload);

                // Something asked for fresh data while this fetch was in
                // flight; start the next one now rather than a full refresh
                // interval later.
                if self.refetch_pending {
                    self.refetch_pending = false;
                    self.last_refresh = None;
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.loading = false;
                self.error = Some("Fetch task disconnected".to_string());
            }
        }
    }

    /// Applies whichever parts of a tick succeeded and reports the ones that
    /// didn't, most important first. Crate-visible so the perf harness can
    /// time a refresh tick without a cluster.
    pub(crate) fn apply_fetch(&mut self, payload: FetchPayload) {
        let FetchPayload {
            indices,
            health,
            nodes,
        } = payload;
        let mut errors = Vec::new();

        let indices_fetched = match indices {
            Ok(rates) => {
                let mut indices = self.update_indices_with_rates(rates);
                self.sort.sort(&mut indices);
                self.indices = indices.into();
                true
            }
            Err(e) => {
                errors.push(e.to_string());
                false
            }
        };

        match health {
            Ok(health) => self.cluster_health = health,
            Err(e) => errors.push(format!("cluster health: {}", e)),
        }

        // `None` means the request was skipped because the panel is hidden,
        // so the last known nodes stay put; a failed request leaves them put
        // too.
        let nodes_fetched = match nodes {
            Some(Ok(mut nodes)) => {
                self.node_sort.sort(&mut nodes);
                self.nodes = nodes;
                self.clamp_node_selection();
                true
            }
            Some(Err(e)) => {
                errors.push(format!("nodes: {}", e));
                false
            }
            None => false,
        };

        // When the cluster can't be reached at all, every section fails with
        // the same message; say it once.
        let mut unique: Vec<String> = Vec::new();
        for error in errors {
            let repeated = unique
                .iter()
                .any(|seen| error.ends_with(seen.as_str()) || seen.ends_with(error.as_str()));
            if !repeated {
                unique.push(error);
            }
        }
        self.error = (!unique.is_empty()).then(|| unique.join(" · "));

        // Reconcile the snapshot baseline with what the cluster now reports:
        // rows that have appeared since adopt this reading, rows that have
        // gone are dropped. Node stats are only offered when they were
        // actually fetched this tick.
        let observed_nodes = nodes_fetched.then_some(self.nodes.as_slice());
        self.snapshot.observe(&self.indices, observed_nodes);

        if !indices_fetched {
            return;
        }

        // Prune index_rate_history for indices that no longer exist. Every
        // current index has just touched its entry and names are unique (they
        // are `_stats` map keys), so the map can only hold stale entries if it
        // has outgrown the list. On most ticks nothing was deleted, and the
        // name set below — one string hash per index — is skipped.
        if self.index_rate_history.len() > self.indices.len() {
            let current_index_names: HashSet<&str> =
                self.indices.iter().map(|i| i.name.as_str()).collect();
            self.index_rate_history
                .retain(|name, _| current_index_names.contains(name.as_str()));
        }

        self.clamp_selection();

        let total_rate = self.total_cluster_rate();
        if self.rate_history.len() >= MAX_HISTORY_POINTS {
            self.rate_history.pop_front();
        }
        self.rate_history.push_back(total_rate);

        // Keep an open details popup live: refresh it in step with the main
        // list instead of leaving it frozen at whatever it showed when it was
        // opened.
        self.refresh_open_details();
    }

    /// Smooths each index's raw rate sample into its rolling average.
    ///
    /// A tick with no sample for an index (its first appearance, a counter
    /// that went backwards) adds nothing to the window: averaging in a
    /// placeholder 0 would under-report a busy index for `rate_samples`
    /// ticks after startup and after every rollover.
    fn update_indices_with_rates(
        &mut self,
        rates: Vec<(IndexRate, Option<f64>)>,
    ) -> Vec<IndexRate> {
        rates
            .into_iter()
            .map(|(mut index, sample)| {
                let history = self
                    .index_rate_history
                    .entry(index.name.clone())
                    .or_insert_with(|| VecDeque::with_capacity(self.rate_samples));

                if let Some(sample) = sample {
                    if history.len() >= self.rate_samples {
                        history.pop_front();
                    }
                    history.push_back(sample);
                }

                index.rate_per_sec = if history.is_empty() {
                    0.0
                } else {
                    history.iter().sum::<f64>() / history.len() as f64
                };
                index
            })
            .collect()
    }

    // Get the current fetch elapsed time (while loading) or last fetch duration
    pub fn fetch_duration_display(&self) -> String {
        if self.loading {
            if let Some(start) = self.fetch_start {
                let elapsed = start.elapsed().as_secs_f64();
                format!("{:.1}s", elapsed)
            } else {
                "0.0s".to_string()
            }
        } else if let Some(duration) = self.last_fetch_duration {
            format!("{:.1}s", duration.as_secs_f64())
        } else {
            "-".to_string()
        }
    }

    pub fn increase_refresh_rate(&mut self) {
        let current_secs = self.refresh_interval.as_secs();
        if current_secs > MIN_REFRESH_SECS {
            self.refresh_interval = Duration::from_secs(current_secs - 1);
        }
    }

    pub fn decrease_refresh_rate(&mut self) {
        let current_secs = self.refresh_interval.as_secs();
        if current_secs < MAX_REFRESH_SECS {
            self.refresh_interval = Duration::from_secs(current_secs + 1);
        }
    }

    // Checks if the application should trigger a new background fetch.
    pub fn should_refresh(&self) -> bool {
        if self.paused {
            return false;
        }
        match self.last_refresh {
            None => true,
            Some(last) => last.elapsed() >= self.refresh_interval,
        }
    }

    // Sort delegation. Each of these acts on whichever panel is on screen,
    // so the two panels keep independent sort columns and directions.
    pub fn next_column(&mut self) {
        match self.main_panel {
            MainPanel::Indices => {
                self.sort.next_column();
                self.resort();
            }
            MainPanel::Nodes => {
                self.node_sort.next_column();
                self.resort_nodes();
            }
        }
    }

    pub fn prev_column(&mut self) {
        match self.main_panel {
            MainPanel::Indices => {
                self.sort.prev_column();
                self.resort();
            }
            MainPanel::Nodes => {
                self.node_sort.prev_column();
                self.resort_nodes();
            }
        }
    }

    pub fn toggle_sort_order(&mut self) {
        match self.main_panel {
            MainPanel::Indices => {
                self.sort.toggle_order();
                self.resort();
            }
            MainPanel::Nodes => {
                self.node_sort.toggle_order();
                self.resort_nodes();
            }
        }
    }

    fn resort(&mut self) {
        let mut indices = std::mem::take(&mut self.indices);
        self.sort.sort(&mut indices);
        self.indices = indices;
    }

    fn resort_nodes(&mut self) {
        let mut nodes = std::mem::take(&mut self.nodes);
        self.node_sort.sort(&mut nodes);
        self.nodes = nodes;
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn toggle_graph(&mut self) {
        self.show_graph = !self.show_graph;
    }

    pub fn toggle_health(&mut self) {
        self.show_health = !self.show_health;
    }

    pub fn toggle_main_panel(&mut self) {
        self.show_main_panel = !self.show_main_panel;
    }

    /// Swaps the main panel between the indices and nodes tables. Also
    /// reveals the panel if it was hidden — pressing the key expecting to see
    /// nodes and getting nothing would be baffling.
    pub fn toggle_nodes(&mut self) {
        self.main_panel = match self.main_panel {
            MainPanel::Indices => MainPanel::Nodes,
            MainPanel::Nodes => MainPanel::Indices,
        };
        self.show_main_panel = true;

        // The node readings on hand are from the last time this panel was
        // up, if ever, so pull fresh ones in on the next loop iteration
        // instead of leaving stale (or no) rows until the interval elapses.
        // A fetch already in flight didn't ask for nodes, so if there is one
        // another is queued straight behind it.
        if self.nodes_focused() {
            if self.loading {
                self.refetch_pending = true;
            } else {
                self.last_refresh = None;
            }
        }
    }

    /// Whether the indices table is the one on screen. Actions on the
    /// selected index (details, exclusion) only make sense then; otherwise
    /// they would act on an off-screen selection the user can't see.
    pub fn indices_focused(&self) -> bool {
        self.show_main_panel && self.main_panel == MainPanel::Indices
    }

    /// Whether the nodes panel is the one currently on screen. Gates the
    /// `_nodes/stats` request in `start_fetch`.
    pub fn nodes_focused(&self) -> bool {
        self.show_main_panel && self.main_panel == MainPanel::Nodes
    }

    pub fn toggle_system_indices(&mut self) {
        self.show_system_indices = !self.show_system_indices;
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    // Navigation acts on whichever panel is on screen. Each panel keeps its
    // own cursor, so switching back and forth doesn't lose your place.
    pub fn select_up(&mut self) {
        self.move_cursor(-1);
    }

    pub fn select_down(&mut self) {
        self.move_cursor(1);
    }

    pub fn select_page_up(&mut self, page_size: usize) {
        self.move_cursor(-(page_size as i32));
    }

    pub fn select_page_down(&mut self, page_size: usize) {
        self.move_cursor(page_size as i32);
    }

    fn move_cursor(&mut self, delta: i32) {
        match self.main_panel {
            MainPanel::Indices => self.move_selection(delta),
            MainPanel::Nodes => self.move_node_selection(delta),
        }
    }

    pub fn select_first(&mut self) {
        match self.main_panel {
            MainPanel::Indices => {
                if let Some(name) = self.filtered_indices().first().map(|i| i.name.clone()) {
                    self.selected_name = Some(name);
                }
            }
            MainPanel::Nodes => {
                if let Some(name) = self.visible_nodes().first().map(|n| n.name.clone()) {
                    self.selected_node = Some(name);
                }
            }
        }
    }

    pub fn select_last(&mut self) {
        match self.main_panel {
            MainPanel::Indices => {
                if let Some(name) = self.filtered_indices().last().map(|i| i.name.clone()) {
                    self.selected_name = Some(name);
                }
            }
            MainPanel::Nodes => {
                if let Some(name) = self.visible_nodes().last().map(|n| n.name.clone()) {
                    self.selected_node = Some(name);
                }
            }
        }
    }

    /// Nodes surviving the filter box. The same filter drives both panels —
    /// regex against the node name, jq against the serialized node — so
    /// entering filter mode does something visible whichever panel is up.
    /// Exclusions (`x`) remain index-only and are not applied here.
    pub fn visible_nodes(&self) -> Vec<&NodeStats> {
        self.nodes
            .iter()
            .filter(|node| self.filter.matches(&node.name, node))
            .collect()
    }

    /// Row position of the node selection within `visible`.
    pub fn selected_node_position_in(&self, visible: &[&NodeStats]) -> Option<usize> {
        let name = self.selected_node.as_deref()?;
        visible.iter().position(|node| node.name == name)
    }

    fn clamp_node_selection(&mut self) {
        let Some(name) = self.selected_node.as_deref() else {
            return;
        };
        if !self.nodes.iter().any(|node| node.name == name) {
            self.selected_node = None;
        }
    }

    fn move_node_selection(&mut self, delta: i32) {
        let next_name = {
            let visible = self.visible_nodes();
            match visible.len() {
                0 => None,
                len => {
                    let last = (len - 1) as i32;
                    let next = match self.selected_node_position_in(&visible) {
                        Some(current) => (current as i32 + delta).clamp(0, last),
                        None => {
                            if delta > 0 {
                                0
                            } else {
                                last
                            }
                        }
                    };
                    Some(visible[next as usize].name.clone())
                }
            }
        };
        self.selected_node = next_name;
    }

    /// Row position of the selection within `visible`, or `None` if nothing
    /// is selected or the selected index isn't part of that list. Widgets
    /// that already hold the visible slice should use this rather than
    /// re-filtering.
    pub fn selected_position_in(&self, visible: &[&IndexRate]) -> Option<usize> {
        let name = self.selected_name.as_deref()?;
        visible.iter().position(|index| index.name == name)
    }

    /// Row position of the selection in the currently visible list.
    pub fn selected_position(&self) -> Option<usize> {
        self.selected_position_in(&self.filtered_indices())
    }

    /// Clears the selection if the selected index no longer exists in the
    /// cluster. A selection that is merely filtered out is deliberately kept,
    /// so it reappears when the filter is cleared.
    fn clamp_selection(&mut self) {
        let Some(name) = self.selected_name.as_deref() else {
            return;
        };
        if !self.indices.iter().any(|index| index.name == name) {
            self.selected_name = None;
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let next_name = {
            let visible = self.filtered_indices();
            match visible.len() {
                0 => None,
                len => {
                    let last = (len - 1) as i32;
                    let next = match self.selected_position_in(&visible) {
                        Some(current) => (current as i32 + delta).clamp(0, last),
                        // Nothing selected, or the selection is currently
                        // filtered out: enter the list from the end the
                        // movement is coming from.
                        None => {
                            if delta > 0 {
                                0
                            } else {
                                last
                            }
                        }
                    };
                    Some(visible[next as usize].name.clone())
                }
            }
        };
        self.selected_name = next_name;
    }

    // Filter delegation
    pub fn enter_filter_mode(&mut self) {
        self.filter.enter();
    }

    pub fn exit_filter_mode(&mut self) {
        self.filter.exit();
    }

    pub fn toggle_filter_mode(&mut self) {
        self.filter.toggle_mode();
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
    }

    pub fn filtered_indices(&self) -> Vec<&IndexRate> {
        self.visible_summary().indices
    }

    // Details delegation
    pub fn show_index_details(&mut self) {
        if !self.indices_focused() {
            return;
        }
        let Some(name) = self.selected_name.clone() else {
            return;
        };
        let Some((doc_count, rate_per_sec, size_bytes)) = self
            .indices
            .iter()
            .find(|index| index.name == name && self.is_visible(index))
            .map(|index| (index.doc_count, index.rate_per_sec, index.size_bytes))
        else {
            return;
        };

        self.details.fetch(
            self.es_client.clone(),
            name,
            doc_count,
            rate_per_sec,
            size_bytes,
        );
    }

    pub fn close_details_popup(&mut self) {
        self.details.close();
    }

    /// Re-fetches the currently displayed index's details using its latest
    /// stats, if the popup is open. Looks the index up by name (not by
    /// table position, which can shift as data reorders) against the
    /// index list; if the index has since disappeared, the popup simply
    /// keeps showing its last known data instead of erroring out.
    fn refresh_open_details(&mut self) {
        if !self.details.show_popup {
            return;
        }
        let Some(name) = self.details.index_name.clone() else {
            return;
        };
        let Some(index) = self.indices.iter().find(|i| i.name == name) else {
            return;
        };

        self.details.fetch(
            self.es_client.clone(),
            name,
            index.doc_count,
            index.rate_per_sec,
            index.size_bytes,
        );
    }

    pub fn poll_details_result(&mut self) {
        self.details.poll();
    }

    pub fn details_scroll_up(&mut self) {
        self.details.scroll_up();
    }

    pub fn details_scroll_down(&mut self) {
        self.details.scroll_down();
    }

    pub fn details_scroll_page_up(&mut self, page_size: usize) {
        self.details.scroll_page_up(page_size);
    }

    pub fn details_scroll_page_down(&mut self, page_size: usize) {
        self.details.scroll_page_down(page_size);
    }

    pub fn toggle_exclude_selected(&mut self) {
        if !self.indices_focused() {
            return;
        }
        let Some(name) = self.selected_name.clone() else {
            return;
        };

        if self.excluded_indices.remove(&name) {
            return;
        }

        // Excluding hides the index, so the selection has to be re-anchored.
        // Remember the row it occupied and keep the cursor there, falling
        // back to the last row if it was the bottom one.
        let Some(position) = self.selected_position() else {
            return;
        };
        self.excluded_indices.insert(name);

        let next_name = {
            let visible = self.filtered_indices();
            visible
                .get(position)
                .or_else(|| visible.last())
                .map(|index| index.name.clone())
        };
        self.selected_name = next_name;
    }

    pub fn clear_exclusions(&mut self) {
        self.excluded_indices.clear();
    }

    pub fn excluded_count(&self) -> usize {
        self.excluded_indices.len()
    }

    /// Freezes the readings currently on screen as the baseline every later
    /// tick is compared against. Pressing it again re-baselines from now.
    ///
    /// Both panels are captured at once, not just the one on screen, so
    /// switching views after taking a snapshot shows movement from the same
    /// moment. Node readings are only current while the nodes panel is up
    /// (`_nodes/stats` isn't requested otherwise), so when it isn't, the node
    /// half is left for `SnapshotState::observe` to fill in from the first
    /// fetch that requests nodes, rather than freezing whatever the panel
    /// showed the last time it was open.
    pub fn take_snapshot(&mut self) {
        let fresh_nodes = self.nodes_focused().then_some(self.nodes.as_slice());
        self.snapshot.capture(&self.indices, fresh_nodes);
    }

    pub fn clear_snapshot(&mut self) {
        self.snapshot.clear();
    }

    pub fn toggle_help_popup(&mut self) {
        self.show_help_popup = !self.show_help_popup;
        if self.show_help_popup {
            self.help_scroll = 0;
        }
    }

    pub fn help_scroll_up(&mut self) {
        self.help_scroll = self.help_scroll.saturating_sub(1);
    }

    pub fn help_scroll_down(&mut self) {
        self.help_scroll = self.help_scroll.saturating_add(1);
    }

    pub fn next_colormap(&mut self) {
        self.colormap = self.colormap.next();
    }

    pub fn prev_colormap(&mut self) {
        self.colormap = self.colormap.prev();
    }

    pub fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit(),
            Action::SelectUp => self.select_up(),
            Action::SelectDown => self.select_down(),
            Action::SelectPageUp(page_size) => self.select_page_up(page_size),
            Action::SelectPageDown(page_size) => self.select_page_down(page_size),
            Action::SelectFirst => self.select_first(),
            Action::SelectLast => self.select_last(),
            Action::ToggleHelp => self.toggle_help_popup(),
            Action::HelpScrollUp => self.help_scroll_up(),
            Action::HelpScrollDown => self.help_scroll_down(),
            Action::TogglePause => self.toggle_pause(),
            Action::ToggleGraph => self.toggle_graph(),
            Action::ToggleHealth => self.toggle_health(),
            Action::ToggleIndices => self.toggle_main_panel(),
            Action::ToggleNodes => self.toggle_nodes(),
            Action::ToggleSystemIndices => self.toggle_system_indices(),
            Action::ShowDetails => self.show_index_details(),
            Action::ToggleExclude => self.toggle_exclude_selected(),
            Action::TakeSnapshot => self.take_snapshot(),
            Action::ClearSnapshot => self.clear_snapshot(),
            Action::ClearExclusions => self.clear_exclusions(),
            Action::IncreaseRefreshRate => self.increase_refresh_rate(),
            Action::DecreaseRefreshRate => self.decrease_refresh_rate(),
            Action::NextColormap => self.next_colormap(),
            Action::PrevColormap => self.prev_colormap(),
            Action::NextColumn => self.next_column(),
            Action::PrevColumn => self.prev_column(),
            Action::ToggleSortOrder => self.toggle_sort_order(),
            Action::EnterFilterMode => self.enter_filter_mode(),
            Action::ExitFilterMode => self.exit_filter_mode(),
            Action::ClearFilter => self.clear_filter(),
            Action::ToggleFilterMode => self.toggle_filter_mode(),
            Action::CloseDetails => self.close_details_popup(),
            Action::DetailsScrollUp => self.details_scroll_up(),
            Action::DetailsScrollDown => self.details_scroll_down(),
            Action::DetailsScrollPageUp(page_size) => self.details_scroll_page_up(page_size),
            Action::DetailsScrollPageDown(page_size) => self.details_scroll_page_down(page_size),
        }
    }
}

/// Whether an index is hidden by default as a system index (toggled with
/// `.`): every dot-prefixed name, Elasticsearch's convention for indices
/// nobody created by hand. That includes data stream backing indices
/// (`.ds-<stream>-…`) and failure-store indices (`.fs-<stream>-…`), so on a
/// cluster fed through data streams the header rate and graph, which follow
/// the visible set, leave that ingestion out until `.` is pressed.
fn is_system_index(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NodeStats;
    use crate::ui::types::{SortColumn, SortOrder};

    fn setup_mock_app() -> App {
        let mut app = App::new(
            "http://localhost:9200".to_string(),
            AuthConfig::None,
            false,
            None,
            5,
            Colormap::Turbo,
            10,
        )
        .unwrap();

        app.indices = vec![
            IndexRate {
                name: "index-1".to_string(),
                doc_count: 100,
                rate_per_sec: 1.0,
                size_bytes: 1024,
                health: "green".to_string(),
            },
            IndexRate {
                name: "index-2".to_string(),
                doc_count: 200,
                rate_per_sec: 2.0,
                size_bytes: 2048,
                health: "green".to_string(),
            },
            IndexRate {
                name: "index-3".to_string(),
                doc_count: 300,
                rate_per_sec: 3.0,
                size_bytes: 3072,
                health: "green".to_string(),
            },
        ]
        .into();
        app
    }

    #[test]
    fn test_selection_movement() {
        let mut app = setup_mock_app();

        // Initial state
        assert_eq!(app.selected_name, None);

        // Move down selects first
        app.select_down();
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));

        // Move down again
        app.select_down();
        assert_eq!(app.selected_name.as_deref(), Some("index-2"));

        // Move up
        app.select_up();
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));

        // Move up at top stays at top
        app.select_up();
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));

        // Select last
        app.select_last();
        assert_eq!(app.selected_name.as_deref(), Some("index-3"));

        // Move down at bottom stays at bottom
        app.select_down();
        assert_eq!(app.selected_name.as_deref(), Some("index-3"));

        // Select first
        app.select_first();
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));
    }

    #[test]
    fn test_pagination() {
        let mut app = setup_mock_app();
        app.select_first();

        // Page down
        app.select_page_down(2);
        assert_eq!(app.selected_name.as_deref(), Some("index-3"));

        // Page up
        app.select_page_up(2);
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));
    }

    #[test]
    fn test_selection_survives_resort() {
        let mut app = setup_mock_app();
        app.sort.column = SortColumn::Rate;
        app.sort.order = SortOrder::Descending;
        app.resort();

        // Highest rate first, so the cursor starts on index-3.
        app.select_first();
        assert_eq!(app.selected_name.as_deref(), Some("index-3"));
        assert_eq!(app.selected_position(), Some(0));

        // index-1 overtakes it and the list re-sorts, as it would on any
        // refresh tick. The cursor must stay on index-3, not on row 0.
        app.indices
            .iter_mut()
            .find(|i| i.name == "index-1")
            .unwrap()
            .rate_per_sec = 99.0;
        app.resort();

        assert_eq!(app.selected_name.as_deref(), Some("index-3"));
        assert_eq!(app.selected_position(), Some(1));
    }

    #[test]
    fn test_selection_dropped_when_index_disappears() {
        let mut app = setup_mock_app();
        app.select_first();
        assert!(app.selected_name.is_some());

        app.indices
            .retain(|i| i.name != app.selected_name.clone().unwrap());
        app.clamp_selection();
        assert_eq!(app.selected_name, None);

        // A selection that is only filtered out is kept, so it comes back
        // when the filter is cleared.
        app.select_first();
        let selected = app.selected_name.clone();
        app.excluded_indices.insert("nothing-matches".to_string());
        app.clamp_selection();
        assert_eq!(app.selected_name, selected);
    }

    #[test]
    fn test_exclusion_impact_on_selection() {
        let mut app = setup_mock_app();
        app.select_down(); // Select index-1
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));

        // Exclude index-1
        app.toggle_exclude_selected();
        assert!(app.excluded_indices.contains("index-1"));

        // The excluded index is gone from the list, so the cursor re-anchors
        // to whatever row took its place.
        assert_eq!(app.selected_name.as_deref(), Some("index-2"));
        assert_eq!(app.selected_position(), Some(0));
        let filtered = app.filtered_indices();
        assert_eq!(filtered[0].name, "index-2");
    }

    fn mock_node(name: &str, heap: u64) -> NodeStats {
        NodeStats {
            name: name.to_string(),
            heap_used_percent: heap,
            ..Default::default()
        }
    }

    #[test]
    fn test_nodes_only_fetched_while_panel_is_focused() {
        let mut app = setup_mock_app();

        // Indices panel showing: no reason to ask for _nodes/stats.
        assert!(!app.nodes_focused());

        app.toggle_nodes();
        assert!(app.nodes_focused());

        // Hiding the panel entirely also stops the request.
        app.toggle_main_panel();
        assert!(!app.nodes_focused());

        // ...and pressing the nodes key again brings the panel back rather
        // than silently switching hidden content.
        app.toggle_main_panel();
        assert!(app.nodes_focused());
        app.toggle_nodes();
        assert_eq!(app.main_panel, MainPanel::Indices);
        assert!(app.show_main_panel);
    }

    #[test]
    fn test_snapshot_off_the_nodes_panel_does_not_freeze_stale_nodes() {
        let mut app = setup_mock_app();
        // Readings left over from the last time the nodes panel was open.
        app.nodes = vec![mock_node("node-a", 10)];
        app.handle_action(Action::TakeSnapshot);

        // Those were never "now", so nothing is compared against them...
        app.nodes[0].heap_used_percent = 90;
        assert_eq!(
            app.snapshot.node_trends(&app.nodes[0]).heap,
            snapshot::Trend::Unchanged
        );

        // ...and the first real nodes fetch becomes the node baseline.
        app.snapshot.observe(&app.indices, Some(&app.nodes));
        app.nodes[0].heap_used_percent = 95;
        assert_eq!(
            app.snapshot.node_trends(&app.nodes[0]).heap,
            snapshot::Trend::Up
        );
    }

    #[test]
    fn test_snapshot_marks_movement_in_both_panels_from_one_baseline() {
        let mut app = setup_mock_app();
        app.nodes = vec![mock_node("node-a", 10), mock_node("node-b", 20)];

        // Nothing is marked until a baseline exists.
        assert!(!app.snapshot.is_active());
        assert_eq!(
            app.snapshot.index_trends(&app.indices[0]).doc_count,
            snapshot::Trend::Unchanged
        );

        // Taken from the nodes panel, where the node readings are current.
        app.handle_action(Action::ToggleNodes);
        app.handle_action(Action::TakeSnapshot);
        assert!(app.snapshot.is_active());
        assert!(app.snapshot.taken_at().is_some());

        // One index climbs, another falls; the untouched one stays unmarked.
        app.indices[0].doc_count = 500;
        app.indices[1].doc_count = 1;
        assert_eq!(
            app.snapshot.index_trends(&app.indices[0]).doc_count,
            snapshot::Trend::Up
        );
        assert_eq!(
            app.snapshot.index_trends(&app.indices[1]).doc_count,
            snapshot::Trend::Down
        );
        assert_eq!(
            app.snapshot.index_trends(&app.indices[2]).doc_count,
            snapshot::Trend::Unchanged
        );

        // The same press captured the indices too, so switching back after
        // the fact still shows movement from that same moment.
        app.nodes[0].heap_used_percent = 90;
        assert_eq!(
            app.snapshot.node_trends(&app.nodes[0]).heap,
            snapshot::Trend::Up
        );
        assert_eq!(
            app.snapshot.node_trends(&app.nodes[1]).heap,
            snapshot::Trend::Unchanged
        );

        app.handle_action(Action::ClearSnapshot);
        assert!(!app.snapshot.is_active());
        assert_eq!(
            app.snapshot.index_trends(&app.indices[0]).doc_count,
            snapshot::Trend::Unchanged
        );
    }

    #[test]
    fn test_retaking_a_snapshot_rebaselines_from_now() {
        let mut app = setup_mock_app();
        app.handle_action(Action::TakeSnapshot);

        app.indices[0].doc_count = 500;
        assert_eq!(
            app.snapshot.index_trends(&app.indices[0]).doc_count,
            snapshot::Trend::Up
        );

        // Pressing `s` again replaces the baseline rather than stacking, so
        // the arrow clears until the value moves again.
        app.handle_action(Action::TakeSnapshot);
        assert_eq!(
            app.snapshot.index_trends(&app.indices[0]).doc_count,
            snapshot::Trend::Unchanged
        );
    }

    #[test]
    fn test_navigation_follows_focused_panel() {
        let mut app = setup_mock_app();
        app.nodes = vec![mock_node("node-a", 10), mock_node("node-b", 20)];

        // On the indices panel, j/k move the index cursor only.
        app.select_down();
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));
        assert_eq!(app.selected_node, None);

        // Switching panels moves the cursor to the node list, leaving the
        // index selection where it was.
        app.toggle_nodes();
        app.select_down();
        assert_eq!(app.selected_node.as_deref(), Some("node-a"));
        assert_eq!(app.selected_name.as_deref(), Some("index-1"));

        app.select_last();
        assert_eq!(app.selected_node.as_deref(), Some("node-b"));

        // Switching back resumes the index cursor from where it was left.
        app.toggle_nodes();
        app.select_down();
        assert_eq!(app.selected_name.as_deref(), Some("index-2"));
        assert_eq!(app.selected_node.as_deref(), Some("node-b"));
    }

    #[test]
    fn test_sorting_applies_to_focused_panel() {
        let mut app = setup_mock_app();
        let index_column = app.sort.column;
        let node_column = app.node_sort.column;

        app.toggle_nodes();
        app.next_column();
        assert_ne!(app.node_sort.column, node_column);
        assert_eq!(
            app.sort.column, index_column,
            "index sort must be untouched"
        );
    }

    #[test]
    fn test_filter_applies_to_nodes_panel() {
        let mut app = setup_mock_app();
        app.nodes = vec![
            mock_node("es-hot-1", 10),
            mock_node("es-hot-2", 20),
            mock_node("es-warm-1", 30),
        ];
        app.toggle_nodes();
        assert_eq!(app.visible_nodes().len(), 3);

        // The same filter box drives whichever panel is focused, so entering
        // a regex must narrow the node list too.
        app.filter.input = app.filter.input.clone().with_value("hot".to_string());
        app.filter.recompile();

        let visible: Vec<&str> = app
            .visible_nodes()
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(visible, vec!["es-hot-1", "es-hot-2"]);

        // Navigation stays inside the filtered set.
        app.select_last();
        assert_eq!(app.selected_node.as_deref(), Some("es-hot-2"));
        app.select_down();
        assert_eq!(app.selected_node.as_deref(), Some("es-hot-2"));
    }

    #[test]
    fn test_node_selection_dropped_when_node_disappears() {
        let mut app = setup_mock_app();
        app.nodes = vec![mock_node("node-a", 10), mock_node("node-b", 20)];
        app.toggle_nodes();
        app.select_first();
        assert_eq!(app.selected_node.as_deref(), Some("node-a"));

        app.nodes.retain(|n| n.name != "node-a");
        app.clamp_node_selection();
        assert_eq!(app.selected_node, None);
    }

    #[test]
    fn test_total_cluster_rate_excludes_hidden() {
        let mut app = setup_mock_app();
        // Add a system index
        app.indices.push(IndexRate {
            name: ".system-index".to_string(),
            doc_count: 50,
            rate_per_sec: 10.0,
            size_bytes: 512,
            health: "green".to_string(),
        });

        // Current rates: index-1(1.0), index-2(2.0), index-3(3.0) = 6.0
        // .system-index is 10.0 but hidden by default
        assert_eq!(app.total_cluster_rate(), 6.0);

        // Show system indices
        app.show_system_indices = true;
        assert_eq!(app.total_cluster_rate(), 16.0);

        // Hide again and exclude index-1
        app.show_system_indices = false;
        app.excluded_indices.insert("index-1".to_string());
        assert_eq!(app.total_cluster_rate(), 5.0); // 2.0 + 3.0
    }

    #[test]
    fn test_every_dot_prefixed_index_waits_for_the_toggle() {
        let mut app = setup_mock_app();
        let with_rate = |name: &str, rate_per_sec: f64| IndexRate {
            name: name.to_string(),
            doc_count: 1,
            rate_per_sec,
            size_bytes: 1,
            health: "green".to_string(),
        };
        app.indices
            .push(with_rate(".ds-logs-app-default-2026.10.10-000001", 100.0));
        app.indices
            .push(with_rate(".fs-logs-app-default-2026.10.10-000002", 1000.0));
        app.indices.push(with_rate(
            ".ds-.logs-deprecation-2026.10.10-000001",
            10_000.0,
        ));
        app.indices.push(with_rate(".kibana_1", 100_000.0));

        // Data stream backing indices are dot-prefixed too, so they stay
        // behind the `.` toggle with the rest.
        assert_eq!(app.total_cluster_rate(), 6.0);

        app.show_system_indices = true;
        assert_eq!(
            app.total_cluster_rate(),
            6.0 + 100.0 + 1000.0 + 10_000.0 + 100_000.0
        );
    }

    #[test]
    fn test_node_filter_does_not_narrow_the_indices() {
        let mut app = setup_mock_app();
        app.nodes = vec![mock_node("es-hot-1", 10)];
        app.toggle_nodes();
        app.filter.input = app.filter.input.clone().with_value("hot".to_string());
        app.filter.recompile();

        // Typed on the nodes panel, so the header rate and graph (computed
        // from the indices) must not drop to zero.
        assert_eq!(app.visible_nodes().len(), 1);
        assert_eq!(app.total_cluster_rate(), 6.0);

        // Back on the indices panel the same box filters indices.
        app.toggle_nodes();
        assert_eq!(app.total_cluster_rate(), 0.0);
    }

    #[test]
    fn test_index_actions_do_nothing_on_the_nodes_panel() {
        let mut app = setup_mock_app();
        app.select_first();
        app.toggle_nodes();

        // The index selection is off screen; acting on it would surprise.
        app.handle_action(Action::ToggleExclude);
        app.handle_action(Action::ShowDetails);
        assert!(app.excluded_indices.is_empty());
        assert!(!app.details.show_popup);
    }

    #[test]
    fn test_rate_smoothing_ignores_ticks_without_a_sample() {
        let mut app = setup_mock_app();
        let index = app.indices[0].clone();

        // First sighting: no sample yet, and nothing enters the window.
        let rates = app.update_indices_with_rates(vec![(index.clone(), None)]);
        assert_eq!(rates[0].rate_per_sec, 0.0);

        // The first real sample is the average, not half of it.
        let rates = app.update_indices_with_rates(vec![(index, Some(10.0))]);
        assert_eq!(rates[0].rate_per_sec, 10.0);
    }

    #[test]
    fn test_rate_history_is_pruned_when_an_index_is_deleted() {
        let mut app = setup_mock_app();
        let tick = |names: &[&str]| FetchPayload {
            indices: Ok(names
                .iter()
                .map(|name| {
                    let index = IndexRate {
                        name: name.to_string(),
                        ..app_index()
                    };
                    (index, Some(1.0))
                })
                .collect()),
            health: Ok(ClusterHealth::default()),
            nodes: None,
        };

        app.apply_fetch(tick(&["a", "b"]));
        app.apply_fetch(tick(&["a", "b"]));
        assert_eq!(app.index_rate_history.len(), 2);

        // `b` was deleted: its smoothing window must not outlive it.
        app.apply_fetch(tick(&["a"]));
        assert_eq!(app.index_rate_history.len(), 1);
        assert!(app.index_rate_history.contains_key("a"));
    }

    fn app_index() -> IndexRate {
        IndexRate {
            name: String::new(),
            doc_count: 1,
            rate_per_sec: 0.0,
            size_bytes: 1,
            health: "green".to_string(),
        }
    }

    #[test]
    fn test_a_failing_section_does_not_discard_the_rest_of_the_tick() {
        use crate::error::EstiCliError;

        let mut app = setup_mock_app();
        let fresh = IndexRate {
            name: "fresh".to_string(),
            doc_count: 1,
            rate_per_sec: 0.0,
            size_bytes: 1,
            health: "green".to_string(),
        };
        app.apply_fetch(FetchPayload {
            indices: Ok(vec![(fresh, Some(4.0))]),
            health: Err(EstiCliError::Config("forbidden".to_string())),
            nodes: Some(Err(EstiCliError::Config("timed out".to_string()))),
        });

        assert_eq!(app.indices.len(), 1);
        assert_eq!(app.indices[0].name, "fresh");
        assert_eq!(app.rate_history.back().copied(), Some(4.0));
        let error = app.error.as_deref().unwrap();
        assert!(
            error.contains("cluster health") && error.contains("nodes"),
            "{}",
            error
        );
    }

    fn visible_names(app: &App) -> Vec<&str> {
        app.filtered_indices()
            .iter()
            .map(|index| index.name.as_str())
            .collect()
    }

    #[test]
    fn test_memoized_visible_set_follows_every_input() {
        let mut app = setup_mock_app();
        assert_eq!(visible_names(&app), ["index-1", "index-2", "index-3"]);

        // The filter box.
        app.filter.input = "index-2".into();
        app.filter.recompile();
        assert_eq!(visible_names(&app), ["index-2"]);
        app.filter.clear();
        assert_eq!(visible_names(&app), ["index-1", "index-2", "index-3"]);

        // Exclusions, including one written straight into the set.
        app.excluded_indices.insert("index-1".to_string());
        assert_eq!(visible_names(&app), ["index-2", "index-3"]);
        app.clear_exclusions();

        // Rows pushed directly, and the system-index toggle.
        let system = IndexRate {
            name: ".kibana_1".to_string(),
            ..app.indices[0].clone()
        };
        app.indices.push(system);
        assert_eq!(visible_names(&app), ["index-1", "index-2", "index-3"]);
        app.toggle_system_indices();
        assert_eq!(
            visible_names(&app),
            ["index-1", "index-2", "index-3", ".kibana_1"]
        );
        app.toggle_system_indices();

        // A field edited in place under a jq filter that reads it.
        app.filter.toggle_mode();
        app.filter.input = ".doc_count > 250".into();
        app.filter.recompile();
        assert_eq!(visible_names(&app), ["index-3"]);
        app.indices[0].doc_count = 1000;
        assert_eq!(visible_names(&app), ["index-1", "index-3"]);

        // Sorting reorders the rows the positions point at (the mock rows
        // start unsorted, so it takes the second toggle, to descending by
        // rate, to move them).
        app.toggle_sort_order();
        app.toggle_sort_order();
        assert_eq!(visible_names(&app), ["index-3", "index-1"]);

        // Switching panels takes the filter off the indices (the system
        // index stays behind its own toggle).
        app.toggle_nodes();
        assert_eq!(visible_names(&app), ["index-3", "index-2", "index-1"]);

        // And the header metrics are invalidated along with the rows.
        app.toggle_nodes();
        assert_eq!(app.total_cluster_rate(), 1.0 + 3.0);
        let index_3 = app
            .indices
            .iter()
            .position(|index| index.name == "index-3")
            .unwrap();
        app.indices[index_3].rate_per_sec = 10.0;
        assert_eq!(app.total_cluster_rate(), 1.0 + 10.0);
    }

    #[test]
    fn test_visible_set_is_not_refiltered_while_nothing_changes() {
        let mut app = setup_mock_app();
        app.filter.input = "index".into();
        app.filter.recompile();

        let pass = |app: &App| {
            app.visible_summary();
            app.visible_cache.borrow().positions.as_ptr()
        };
        let first = pass(&app);

        // Redraws, cursor movement and the filter caret blinking are what
        // happen many times a second; none of them may re-run the filter.
        assert_eq!(pass(&app), first);
        app.select_down();
        app.select_last();
        assert_eq!(pass(&app), first);
        app.filter.handle_key(&crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Left,
                crossterm::event::KeyModifiers::NONE,
            ),
        ));
        assert_eq!(pass(&app), first);

        // An edit to the text does.
        app.filter.input = "index-".into();
        app.filter.recompile();
        assert_ne!(pass(&app), first);
    }
}
