pub mod actions;
pub mod details;
pub mod filter;
pub mod sort;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::elasticsearch::{AuthConfig, EsClient};
use crate::error::{EstiCliError, Result};
use crate::models::{ClusterHealth, IndexRate};
use crate::ui::types::Colormap;
use crate::utils::format_number;
use tokio::sync::{mpsc, Mutex};

use self::actions::Action;
use self::details::DetailsState;
use self::filter::FilterState;
use self::sort::SortState;

const MAX_HISTORY_POINTS: usize = 60;
const MIN_REFRESH_SECS: u64 = 1;
const MAX_REFRESH_SECS: u64 = 60;

pub type FetchResult = std::result::Result<(Vec<IndexRate>, ClusterHealth), EstiCliError>;

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Main application state and logic controller.
///
/// This struct holds all the state necessary to render the TUI and handles
/// all user actions and background data fetching.
pub struct App {
    pub indices: Vec<IndexRate>,
    pub running: bool,
    pub error: Option<String>,
    pub loading: bool,
    pub spinner_frame: usize,
    pub refresh_interval: Duration,
    pub last_refresh: Option<Instant>,
    pub rate_history: VecDeque<u64>,
    pub es_url: String,
    pub fetch_start: Option<Instant>,
    pub last_fetch_duration: Option<Duration>,
    pub show_graph: bool,
    pub show_health: bool,
    pub show_indices: bool,
    pub show_system_indices: bool,
    pub paused: bool,
    pub selected_index: Option<usize>,
    pub excluded_indices: HashSet<String>,
    pub show_help_popup: bool,
    pub help_scroll: usize,
    pub colormap: Colormap,
    pub rate_samples: usize,
    pub cluster_health: ClusterHealth,

    // Sub-states
    pub sort: SortState,
    pub filter: FilterState,
    pub details: DetailsState,

    index_rate_history: HashMap<String, VecDeque<f64>>,
    es_client: Arc<Mutex<EsClient>>,
    fetch_rx: mpsc::Receiver<FetchResult>,
    fetch_tx: mpsc::Sender<FetchResult>,
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
        let es_client = EsClient::new(base_url.clone(), auth, insecure, ca_cert)?;
        let (fetch_tx, fetch_rx) = mpsc::channel(1);

        Ok(Self {
            indices: Vec::new(),
            running: true,
            error: None,
            loading: false,
            spinner_frame: 0,
            refresh_interval: Duration::from_secs(refresh_secs),
            last_refresh: None,
            rate_history: VecDeque::with_capacity(MAX_HISTORY_POINTS),
            es_url: base_url,
            fetch_start: None,
            last_fetch_duration: None,
            show_graph: true,
            show_health: true,
            show_indices: true,
            show_system_indices: false,
            paused: false,
            selected_index: None,
            excluded_indices: HashSet::new(),
            show_help_popup: false,
            help_scroll: 0,
            colormap,
            rate_samples: rate_samples.max(1), // At least 1 sample
            cluster_health: ClusterHealth::default(),

            sort: SortState::default(),
            filter: FilterState::default(),
            details: DetailsState::new(),

            index_rate_history: HashMap::new(),
            es_client: Arc::new(Mutex::new(es_client)),
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

    // Returns the total indexing rate across all non-excluded indices.
    pub fn total_cluster_rate(&self) -> f64 {
        self.indices
            .iter()
            .filter(|i| {
                // Filter excluded indices
                if self.excluded_indices.contains(&i.name) {
                    return false;
                }
                // Filter system indices if not showing them
                if !self.show_system_indices && i.name.starts_with('.') {
                    return false;
                }
                // Apply regex filter from FilterState
                self.filter.is_match(i)
            })
            .map(|i| i.rate_per_sec)
            .sum()
    }

    // Returns a human-readable string of the total cluster indexing rate.
    pub fn total_cluster_rate_human(&self) -> String {
        format_number(self.total_cluster_rate())
    }

    // Starts a background fetch of index rates from Elasticsearch.
    pub fn start_fetch(&mut self) {
        if self.loading {
            return;
        }

        self.loading = true;
        self.fetch_start = Some(Instant::now());
        let client = Arc::clone(&self.es_client);
        let tx = self.fetch_tx.clone();

        tokio::spawn(async move {
            let result = {
                let mut client = client.lock().await;
                let rates_res = client.fetch_index_rates().await;
                let health_res = client.fetch_cluster_health().await;

                match (rates_res, health_res) {
                    (Ok(rates), Ok(health)) => Ok((rates, health)),
                    (Err(e), _) => Err(e),
                    (_, Err(e)) => Err(e),
                }
            };

            let _ = tx.send(result).await;
        });
    }

    // Check for fetch results (non-blocking)
    pub fn poll_fetch_result(&mut self) {
        match self.fetch_rx.try_recv() {
            Ok(result) => {
                self.loading = false;
                self.last_refresh = Some(Instant::now());

                if let Some(start) = self.fetch_start.take() {
                    self.last_fetch_duration = Some(start.elapsed());
                }

                match result {
                    Ok((mut indices, health)) => {
                        self.update_indices_with_rates(&mut indices);
                        self.sort.sort(&mut indices);
                        self.indices = indices;
                        self.cluster_health = health;
                        self.error = None;

                        // Prune index_rate_history for indices that no longer exist
                        let current_index_names: HashSet<String> =
                            self.indices.iter().map(|i| i.name.clone()).collect();
                        self.index_rate_history
                            .retain(|name, _| current_index_names.contains(name));

                        let total_rate = self.total_cluster_rate() as u64;
                        if self.rate_history.len() >= MAX_HISTORY_POINTS {
                            self.rate_history.pop_front();
                        }
                        self.rate_history.push_back(total_rate);
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                    }
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.loading = false;
                self.error = Some("Fetch task disconnected".to_string());
            }
        }
    }

    fn update_indices_with_rates(&mut self, indices: &mut [IndexRate]) {
        for index in indices {
            let history = self
                .index_rate_history
                .entry(index.name.clone())
                .or_insert_with(|| VecDeque::with_capacity(self.rate_samples));

            if history.len() >= self.rate_samples {
                history.pop_front();
            }
            history.push_back(index.rate_per_sec);

            if !history.is_empty() {
                let sum: f64 = history.iter().sum();
                index.rate_per_sec = sum / history.len() as f64;
            }
        }
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

    pub fn rate_history_vec(&self) -> Vec<u64> {
        self.rate_history.iter().copied().collect()
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

    // Sort delegation
    pub fn next_column(&mut self) {
        self.sort.next_column();
        self.resort();
    }

    pub fn prev_column(&mut self) {
        self.sort.prev_column();
        self.resort();
    }

    pub fn toggle_sort_order(&mut self) {
        self.sort.toggle_order();
        self.resort();
    }

    fn resort(&mut self) {
        let mut indices = std::mem::take(&mut self.indices);
        self.sort.sort(&mut indices);
        self.indices = indices;
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

    pub fn toggle_indices(&mut self) {
        self.show_indices = !self.show_indices;
    }

    pub fn toggle_system_indices(&mut self) {
        self.show_system_indices = !self.show_system_indices;
        // Reset selection when toggling to avoid out-of-bounds
        self.selected_index = None;
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
    }

    pub fn select_up(&mut self) {
        self.move_selection(-1);
    }

    pub fn select_down(&mut self) {
        self.move_selection(1);
    }

    pub fn select_page_up(&mut self, page_size: usize) {
        self.move_selection(-(page_size as i32));
    }

    pub fn select_page_down(&mut self, page_size: usize) {
        self.move_selection(page_size as i32);
    }

    pub fn select_first(&mut self) {
        if !self.filtered_indices().is_empty() {
            self.selected_index = Some(0);
        }
    }

    pub fn select_last(&mut self) {
        let count = self.filtered_indices().len();
        if count > 0 {
            self.selected_index = Some(count.saturating_sub(1));
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.filtered_indices().len();
        if count == 0 {
            self.selected_index = None;
            return;
        }

        let next = match self.selected_index {
            Some(current) => (current as i32 + delta)
                .max(0)
                .min(count.saturating_sub(1) as i32),
            None => {
                if delta > 0 {
                    0
                } else {
                    count.saturating_sub(1) as i32
                }
            }
        };
        self.selected_index = Some(next as usize);
    }

    // Filter delegation
    pub fn enter_filter_mode(&mut self) {
        self.filter.enter();
    }

    pub fn exit_filter_mode(&mut self) {
        self.filter.exit();
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
    }

    pub fn filtered_indices(&self) -> Vec<&IndexRate> {
        self.indices
            .iter()
            .filter(|i| {
                // Filter excluded indices
                if self.excluded_indices.contains(&i.name) {
                    return false;
                }
                // Filter system indices if not showing them
                if !self.show_system_indices && i.name.starts_with('.') {
                    return false;
                }
                // Apply regex filter from FilterState
                self.filter.is_match(i)
            })
            .collect()
    }

    // Details delegation
    pub fn show_index_details(&mut self) {
        if let Some(selected) = self.selected_index {
            let filtered = self.filtered_indices();
            if let Some(index) = filtered.get(selected) {
                let index_name = index.name.clone();
                let doc_count = index.doc_count;
                let rate_per_sec = index.rate_per_sec;
                let size_bytes = index.size_bytes;

                self.details.fetch(
                    self.es_client.clone(),
                    index_name,
                    doc_count,
                    rate_per_sec,
                    size_bytes,
                );
            }
        }
    }

    pub fn close_details_popup(&mut self) {
        self.details.close();
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
        if let Some(selected) = self.selected_index {
            let filtered = self.filtered_indices();
            if let Some(index) = filtered.get(selected) {
                let name = index.name.clone();
                if self.excluded_indices.contains(&name) {
                    self.excluded_indices.remove(&name);
                } else {
                    self.excluded_indices.insert(name);
                    // Move selection to next item if possible
                    let new_count = self.filtered_indices().len();
                    if new_count == 0 {
                        self.selected_index = None;
                    } else if selected >= new_count {
                        self.selected_index = Some(new_count.saturating_sub(1));
                    }
                }
            }
        }
    }

    pub fn clear_exclusions(&mut self) {
        self.excluded_indices.clear();
    }

    pub fn excluded_count(&self) -> usize {
        self.excluded_indices.len()
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
            Action::SelectPageUp => self.select_page_up(20),
            Action::SelectPageDown => self.select_page_down(20),
            Action::SelectFirst => self.select_first(),
            Action::SelectLast => self.select_last(),
            Action::ToggleHelp => self.toggle_help_popup(),
            Action::HelpScrollUp => self.help_scroll_up(),
            Action::HelpScrollDown => self.help_scroll_down(),
            Action::TogglePause => self.toggle_pause(),
            Action::ToggleGraph => self.toggle_graph(),
            Action::ToggleHealth => self.toggle_health(),
            Action::ToggleIndices => self.toggle_indices(),
            Action::ToggleSystemIndices => self.toggle_system_indices(),
            Action::ShowDetails => self.show_index_details(),
            Action::ToggleExclude => self.toggle_exclude_selected(),
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
            Action::CloseDetails => self.close_details_popup(),
            Action::DetailsScrollUp => self.details_scroll_up(),
            Action::DetailsScrollDown => self.details_scroll_down(),
            Action::DetailsScrollPageUp => self.details_scroll_page_up(10),
            Action::DetailsScrollPageDown => self.details_scroll_page_down(10),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ];
        app
    }

    #[test]
    fn test_selection_movement() {
        let mut app = setup_mock_app();

        // Initial state
        assert_eq!(app.selected_index, None);

        // Move down selects first
        app.select_down();
        assert_eq!(app.selected_index, Some(0));

        // Move down again
        app.select_down();
        assert_eq!(app.selected_index, Some(1));

        // Move up
        app.select_up();
        assert_eq!(app.selected_index, Some(0));

        // Move up at top stays at top
        app.select_up();
        assert_eq!(app.selected_index, Some(0));

        // Select last
        app.select_last();
        assert_eq!(app.selected_index, Some(2));

        // Move down at bottom stays at bottom
        app.select_down();
        assert_eq!(app.selected_index, Some(2));

        // Select first
        app.select_first();
        assert_eq!(app.selected_index, Some(0));
    }

    #[test]
    fn test_pagination() {
        let mut app = setup_mock_app();
        app.select_first();

        // Page down
        app.select_page_down(2);
        assert_eq!(app.selected_index, Some(2));

        // Page up
        app.select_page_up(2);
        assert_eq!(app.selected_index, Some(0));
    }

    #[test]
    fn test_exclusion_impact_on_selection() {
        let mut app = setup_mock_app();
        app.select_down(); // Select index-1
        assert_eq!(app.selected_index, Some(0));

        // Exclude index-1
        app.toggle_exclude_selected();
        assert!(app.excluded_indices.contains("index-1"));

        // Selection should move to next available or stay within bounds
        // In our implementation, it re-evaluates filtered_indices.
        // After excluding index-1, index-2 becomes the new index 0.
        assert_eq!(app.selected_index, Some(0));
        let filtered = app.filtered_indices();
        assert_eq!(filtered[0].name, "index-2");
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
}
