use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::elasticsearch::EsClient;
use crate::models::IndexDetails;

pub type DetailsResult = Result<IndexDetails, String>;

pub struct DetailsState {
    pub show_popup: bool,
    pub data: Option<IndexDetails>,
    /// True only until the first fetch for the currently open index
    /// completes — drives the full-screen "Loading..." message.
    pub loading: bool,
    /// True while any fetch (initial or background refresh) is in flight.
    /// Unlike `loading`, this stays true across refreshes of an index whose
    /// data is already on screen, so the popup can show a small "refreshing"
    /// hint instead of blanking the content.
    pub refreshing: bool,
    pub error: Option<String>,
    pub scroll: usize,
    /// Index currently shown/refreshed by the popup, so a periodic refresh
    /// can be recognized as "same index" (update in place) vs. a fresh open.
    pub index_name: Option<String>,
    pub rx: mpsc::Receiver<DetailsResult>,
    pub tx: mpsc::Sender<DetailsResult>,
}

impl DetailsState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        Self {
            show_popup: false,
            data: None,
            loading: false,
            refreshing: false,
            error: None,
            scroll: 0,
            index_name: None,
            rx,
            tx,
        }
    }

    /// Fetches index details. If the popup is already open and showing this
    /// same index, this is treated as a background refresh: the existing
    /// data, scroll position, and error state are left untouched until the
    /// new result arrives, so the popup updates seamlessly instead of
    /// flashing back to a loading state. Opening a different index (or
    /// reopening after being closed) resets to a clean slate as before.
    pub fn fetch(
        &mut self,
        es_client: Arc<Mutex<EsClient>>,
        index_name: String,
        doc_count: u64,
        rate_per_sec: f64,
        size_bytes: u64,
    ) {
        // Never overlap two in-flight fetches for the details popup.
        if self.refreshing {
            return;
        }

        let is_same_index =
            self.show_popup && self.index_name.as_deref() == Some(index_name.as_str());

        self.show_popup = true;
        self.refreshing = true;
        self.index_name = Some(index_name.clone());
        if !is_same_index {
            self.loading = true;
            self.error = None;
            self.data = None;
            self.scroll = 0;
        }

        let tx = self.tx.clone();

        tokio::spawn(async move {
            let result = {
                let client = es_client.lock().await;
                client
                    .fetch_index_details(&index_name, doc_count, rate_per_sec, size_bytes)
                    .await
            };

            let details_result = result.map_err(|e| e.to_string());
            let _ = tx.send(details_result).await;
        });
    }

    pub fn close(&mut self) {
        self.show_popup = false;
        self.data = None;
        self.error = None;
        self.loading = false;
        self.refreshing = false;
        self.scroll = 0;
        self.index_name = None;
    }

    pub fn poll(&mut self) {
        match self.rx.try_recv() {
            Ok(result) => {
                self.loading = false;
                self.refreshing = false;
                match result {
                    Ok(details) => {
                        self.data = Some(details);
                        self.error = None;
                    }
                    Err(e) => {
                        // A background refresh failing shouldn't blank out
                        // data that's already on screen — only surface the
                        // error if we have nothing else to show yet.
                        if self.data.is_none() {
                            self.error = Some(e);
                        }
                    }
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // No result yet
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.loading = false;
                self.refreshing = false;
                self.error = Some("Details fetch disconnected".to_string());
            }
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_page_up(&mut self, page_size: usize) {
        self.scroll = self.scroll.saturating_sub(page_size);
    }

    pub fn scroll_page_down(&mut self, page_size: usize) {
        self.scroll = self.scroll.saturating_add(page_size);
    }
}
