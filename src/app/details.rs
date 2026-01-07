use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::elasticsearch::EsClient;
use crate::models::IndexDetails;

pub type DetailsResult = Result<IndexDetails, String>;

pub struct DetailsState {
    pub show_popup: bool,
    pub data: Option<IndexDetails>,
    pub loading: bool,
    pub error: Option<String>,
    pub scroll: usize,
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
            error: None,
            scroll: 0,
            rx,
            tx,
        }
    }

    pub fn fetch(
        &mut self,
        es_client: Arc<Mutex<EsClient>>,
        index_name: String,
        doc_count: u64,
        rate_per_sec: f64,
        size_bytes: u64,
    ) {
        self.show_popup = true;
        self.loading = true;
        self.error = None;
        self.data = None;
        self.scroll = 0;

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
        self.scroll = 0;
    }

    pub fn poll(&mut self) {
        match self.rx.try_recv() {
            Ok(result) => {
                self.loading = false;
                match result {
                    Ok(details) => {
                        self.data = Some(details);
                        self.error = None;
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                // No result yet
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.loading = false;
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
