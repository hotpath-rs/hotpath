//! TUI application state and main run loop

use hotpath::json::{
    JsonChannelLogsList, JsonChannelSentLog, JsonChannelsList, JsonDataFlowLog, JsonDebugEntry,
    JsonDebugLog, JsonFunctionAllocLogsList, JsonFunctionTimingLogsList, JsonFunctionsList,
    JsonFutureLog, JsonFutureLogsList, JsonFuturesList, JsonHttpList, JsonHttpLog,
    JsonHttpLogsList, JsonIoList, JsonMutexesList, JsonRuntimeSnapshot, JsonRwLocksList,
    JsonServerList, JsonSqlList, JsonSqlLog, JsonSqlLogsList, JsonStreamLogsList, JsonStreamsList,
    JsonThreadsList,
};
use hotpath::wrap::crossbeam_channel::{Receiver, Sender};
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use std::time::{Duration, Instant};

use super::events::{AppEvent, DataRequest};

mod data;
mod keys;
mod mouse;
mod state;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedTab {
    #[default]
    Functions,
    DataFlow,
    Io,
    Threads,
    Debug,
    Runtime,
}

impl SelectedTab {
    pub(crate) fn number(&self) -> u8 {
        match self {
            SelectedTab::Functions => 1,
            SelectedTab::DataFlow => 2,
            SelectedTab::Io => 3,
            SelectedTab::Threads => 4,
            SelectedTab::Debug => 5,
            SelectedTab::Runtime => 6,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            SelectedTab::Functions => "Functions",
            SelectedTab::DataFlow => "Data Flow",
            SelectedTab::Io => "I/O",
            SelectedTab::Threads => "Threads",
            SelectedTab::Debug => "Debug",
            SelectedTab::Runtime => "Tokio",
        }
    }

    pub(crate) fn from_env_str(s: &str) -> Option<Self> {
        let n: u8 = s.trim().parse().ok()?;
        match n {
            1 => Some(SelectedTab::Functions),
            2 => Some(SelectedTab::DataFlow),
            3 => Some(SelectedTab::Io),
            4 => Some(SelectedTab::Threads),
            5 => Some(SelectedTab::Debug),
            6 => Some(SelectedTab::Runtime),
            _ => None,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionsSubTab {
    #[default]
    Timing,
    Memory,
    Cpu,
}

impl FunctionsSubTab {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            FunctionsSubTab::Timing => "Timing",
            FunctionsSubTab::Memory => "Memory",
            FunctionsSubTab::Cpu => "CPU",
        }
    }

    pub(crate) fn cycle(&self) -> Self {
        match self {
            FunctionsSubTab::Timing => FunctionsSubTab::Memory,
            FunctionsSubTab::Memory => FunctionsSubTab::Cpu,
            FunctionsSubTab::Cpu => FunctionsSubTab::Timing,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataFlowSubTab {
    #[default]
    Channels,
    Streams,
    Futures,
    RwLocks,
    Mutexes,
}

impl DataFlowSubTab {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            DataFlowSubTab::Channels => "Channels",
            DataFlowSubTab::Streams => "Streams",
            DataFlowSubTab::Futures => "Futures",
            DataFlowSubTab::RwLocks => "RwLocks",
            DataFlowSubTab::Mutexes => "Mutexes",
        }
    }

    pub(crate) fn cycle(&self) -> Self {
        match self {
            DataFlowSubTab::Channels => DataFlowSubTab::Streams,
            DataFlowSubTab::Streams => DataFlowSubTab::Futures,
            DataFlowSubTab::Futures => DataFlowSubTab::RwLocks,
            DataFlowSubTab::RwLocks => DataFlowSubTab::Mutexes,
            DataFlowSubTab::Mutexes => DataFlowSubTab::Channels,
        }
    }

    /// RwLocks and Mutexes have no per-event logs, so the logs/inspect panes don't apply.
    pub(crate) fn has_logs(&self) -> bool {
        !matches!(self, DataFlowSubTab::RwLocks | DataFlowSubTab::Mutexes)
    }
}

/// Sub-tabs of the I/O top-level tab.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoSubTab {
    #[default]
    Sql,
    Http,
    Server,
    Bytes,
}

impl IoSubTab {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            IoSubTab::Sql => "SQL",
            IoSubTab::Http => "HTTP",
            IoSubTab::Server => "Server",
            IoSubTab::Bytes => "Bytes",
        }
    }

    pub(crate) fn cycle(&self) -> Self {
        match self {
            IoSubTab::Sql => IoSubTab::Http,
            IoSubTab::Http => IoSubTab::Server,
            IoSubTab::Server => IoSubTab::Bytes,
            IoSubTab::Bytes => IoSubTab::Sql,
        }
    }
}

/// Clickable sub-tab label; only the active tab's sub-tab row is on screen,
/// so one list covers all three variants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubTabHit {
    Functions(FunctionsSubTab),
    DataFlow(DataFlowSubTab),
    Io(IoSubTab),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionsFocus {
    Functions,
    Logs,
    Inspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataFlowFocus {
    List,
    Logs,
    Inspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebugFocus {
    Debug,
    Logs,
    Inspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoFocus {
    List,
    Logs,
    Inspect,
}

/// Inspected function log entry for the inspect popup
#[derive(Debug, Clone)]
pub(crate) struct InspectedFunctionLog {
    pub(crate) invocation: u64,
    /// Formatted value (duration or bytes)
    pub(crate) value: String,
    pub(crate) ago: String,
    /// Allocation count (only for memory mode)
    pub(crate) alloc_count: Option<u64>,
    pub(crate) tid: Option<u64>,
    /// Debug representation of the return value (when log = true)
    pub(crate) result: Option<String>,
}

pub(crate) type CachedDebugLogs = Vec<JsonDebugLog>;

/// Logs for a data flow entry (can be channel, stream, or future)
#[derive(Debug, Clone)]
pub(crate) enum DataFlowLogs {
    Channel(JsonChannelLogsList),
    Stream(JsonStreamLogsList),
    Future(JsonFutureLogsList),
}

/// Inspected log for data flow (can be channel sent log, stream log, or future call)
#[derive(Debug, Clone)]
pub(crate) enum InspectedDataFlowLog {
    ChannelSent(JsonChannelSentLog),
    Stream(JsonDataFlowLog),
    FutureCall(JsonFutureLog),
}

pub(crate) struct App {
    pub(crate) timing_functions: JsonFunctionsList,
    pub(crate) memory_functions: JsonFunctionsList,
    pub(crate) memory_unavailable_reason: Option<String>,
    pub(crate) cpu_envelope: Option<hotpath::json::JsonFunctionsCpuEnvelope>,
    pub(crate) cpu_unavailable_reason: Option<String>,
    pub(crate) cpu_table_state: TableState,

    pub(crate) timing_table_state: TableState,
    pub(crate) memory_table_state: TableState,
    pub(crate) selected_tab: SelectedTab,
    pub(crate) functions_sub_tab: FunctionsSubTab,
    pub(crate) data_flow_sub_tab: DataFlowSubTab,
    pub(crate) io_sub_tab: IoSubTab,
    pub(crate) paused: bool,

    pub(crate) last_refresh: Instant,
    pub(crate) last_successful_fetch: Option<Instant>,
    pub(crate) error_message: Option<String>,

    pub(crate) function_logs_table_state: TableState,
    pub(crate) functions_focus: FunctionsFocus,
    pub(crate) show_function_logs: bool,
    pub(crate) current_timing_logs: Option<JsonFunctionTimingLogsList>,
    pub(crate) current_alloc_logs: Option<JsonFunctionAllocLogsList>,
    pub(crate) pinned_function: Option<String>,
    pub(crate) pinned_function_id: Option<u32>,
    pub(crate) inspected_function_log: Option<InspectedFunctionLog>,

    pub(crate) request_tx: Sender<DataRequest>,
    pub(crate) event_rx: Receiver<AppEvent>,
    pub(crate) refresh_interval: Duration,
    pub(crate) metrics_host: String,
    exit: bool,

    pub(crate) loading_functions: bool,
    pub(crate) loading_data_flow: bool,
    pub(crate) loading_io: bool,
    pub(crate) loading_threads: bool,
    pub(crate) loading_debug: bool,

    pub(crate) channels: JsonChannelsList,
    pub(crate) streams: JsonStreamsList,
    pub(crate) futures: JsonFuturesList,
    pub(crate) rw_locks: JsonRwLocksList,
    pub(crate) mutexes: JsonMutexesList,
    pub(crate) sql: JsonSqlList,
    pub(crate) http: JsonHttpList,
    pub(crate) server: JsonServerList,
    pub(crate) io_bytes: JsonIoList,
    pub(crate) channels_table_state: TableState,
    pub(crate) streams_table_state: TableState,
    pub(crate) futures_table_state: TableState,
    pub(crate) rw_locks_table_state: TableState,
    pub(crate) mutexes_table_state: TableState,
    pub(crate) sql_table_state: TableState,
    pub(crate) http_table_state: TableState,
    pub(crate) server_table_state: TableState,
    pub(crate) io_bytes_table_state: TableState,
    pub(crate) io_focus: IoFocus,
    pub(crate) show_sql_logs: bool,
    pub(crate) sql_logs: Option<JsonSqlLogsList>,
    pub(crate) sql_logs_table_state: TableState,
    pub(crate) inspected_sql_log: Option<JsonSqlLog>,
    /// Logs pane shared by the HTTP and Server subtabs: both list
    /// `JsonHttpLogsList` entries and the pane is reset on every subtab switch.
    pub(crate) show_http_logs: bool,
    pub(crate) http_logs: Option<JsonHttpLogsList>,
    pub(crate) http_logs_table_state: TableState,
    pub(crate) inspected_http_log: Option<JsonHttpLog>,
    pub(crate) data_flow_focus: DataFlowFocus,
    pub(crate) show_data_flow_logs: bool,
    pub(crate) data_flow_logs: Option<DataFlowLogs>,
    pub(crate) data_flow_logs_table_state: TableState,
    pub(crate) inspected_data_flow_log: Option<InspectedDataFlowLog>,

    pub(crate) threads: JsonThreadsList,
    pub(crate) threads_table_state: TableState,

    pub(crate) debug_stats: Vec<JsonDebugEntry>,
    pub(crate) debug_table_state: TableState,
    pub(crate) debug_focus: DebugFocus,
    pub(crate) show_debug_logs: bool,
    pub(crate) debug_logs: Option<CachedDebugLogs>,
    pub(crate) debug_logs_table_state: TableState,
    pub(crate) inspected_debug_log: Option<JsonDebugLog>,

    pub(crate) tokio_runtime: Option<JsonRuntimeSnapshot>,
    pub(crate) runtime_table_state: TableState,
    pub(crate) loading_runtime: bool,

    pub(crate) program_uptime: Option<String>,
    pub(crate) program_pid: Option<u32>,
    pub(crate) auto_expand_logs: bool,
    pub(crate) auto_select_index: Option<usize>,

    pub(crate) pending_g: Option<Instant>,

    /// Clickable label regions, rebuilt on every draw for mouse hit testing
    pub(crate) tab_hit_areas: Vec<(Rect, SelectedTab)>,
    pub(crate) sub_tab_hit_areas: Vec<(Rect, SubTabHit)>,
}

#[hotpath::measure_all]
impl App {
    pub(crate) fn new(
        metrics_host: &str,
        metrics_port: u16,
        auth_token: Option<String>,
        refresh_interval_ms: u64,
    ) -> Self {
        let (request_tx, request_rx) = hotpath::channel!(
            crossbeam_channel::unbounded::<DataRequest>(),
            label = "tui_requests",
            log = true
        );
        let (event_tx, event_rx) = hotpath::channel!(
            crossbeam_channel::unbounded::<AppEvent>(),
            label = "tui_events",
            log = true
        );

        let base_url = format!("{}:{}", metrics_host.trim_end_matches('/'), metrics_port);
        super::http_worker::spawn_http_worker(
            request_rx,
            event_tx.clone(),
            base_url.clone(),
            auth_token,
        );
        super::input::spawn_input_reader(event_tx);

        let initial_tab = std::env::var("HOTPATH_TUI_TAB")
            .ok()
            .and_then(|val| SelectedTab::from_env_str(&val))
            .unwrap_or_default();
        let auto_select_index: Option<usize> = std::env::var("HOTPATH_TUI_AUTO_EXPAND_LOGS")
            .ok()
            .map(|val| {
                val.trim().parse::<usize>().unwrap_or_else(|_| {
                    panic!(
                        "HOTPATH_TUI_AUTO_EXPAND_LOGS must be an integer, got: {:?}",
                        val
                    )
                })
            });
        let auto_expand_logs = auto_select_index.is_some();

        let empty_functions = JsonFunctionsList {
            profiling_mode: hotpath::ProfilingMode::Timing,
            time_elapsed: "0 ns".to_string(),
            total_elapsed_ns: 0,
            total_allocated: None,
            total_icount: None,
            description: "Waiting for data...".to_string(),
            caller_name: "unknown".to_string(),
            percentiles: vec![95.0],
            data: Vec::new(),
            included_count: 0,
            total_count: 0,
            tool: None,
        };

        Self {
            timing_functions: empty_functions.clone(),
            memory_functions: empty_functions,
            memory_unavailable_reason: None,
            cpu_envelope: None,
            cpu_unavailable_reason: None,
            cpu_table_state: TableState::default().with_selected(0),
            timing_table_state: TableState::default().with_selected(0),
            memory_table_state: TableState::default().with_selected(0),
            selected_tab: initial_tab,
            functions_sub_tab: FunctionsSubTab::default(),
            data_flow_sub_tab: DataFlowSubTab::default(),
            io_sub_tab: IoSubTab::default(),
            paused: false,
            last_refresh: Instant::now(),
            last_successful_fetch: None,
            error_message: None,
            function_logs_table_state: TableState::default(),
            functions_focus: FunctionsFocus::Functions,
            show_function_logs: false,
            current_timing_logs: None,
            current_alloc_logs: None,
            pinned_function: None,
            pinned_function_id: None,
            inspected_function_log: None,
            request_tx,
            event_rx,
            refresh_interval: Duration::from_millis(refresh_interval_ms),
            metrics_host: base_url,
            exit: false,
            loading_functions: false,
            loading_data_flow: false,
            loading_io: false,
            loading_threads: false,
            loading_debug: false,
            channels: JsonChannelsList {
                current_elapsed_ns: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            streams: JsonStreamsList {
                current_elapsed_ns: 0,
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            futures: JsonFuturesList {
                current_elapsed_ns: 0,
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            rw_locks: JsonRwLocksList {
                current_elapsed_ns: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            mutexes: JsonMutexesList {
                current_elapsed_ns: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            sql: JsonSqlList {
                current_elapsed_ns: 0,
                total_ns: 0,
                total_calls: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            http: JsonHttpList {
                current_elapsed_ns: 0,
                total_ns: 0,
                total_calls: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            server: JsonServerList {
                current_elapsed_ns: 0,
                total_ns: 0,
                total_calls: 0,
                total_alloc_bytes: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            io_bytes: JsonIoList {
                current_elapsed_ns: 0,
                percentiles: vec![],
                data: vec![],
                total_count: 0,
                included_count: 0,
            },
            channels_table_state: TableState::default().with_selected(0),
            streams_table_state: TableState::default().with_selected(0),
            futures_table_state: TableState::default().with_selected(0),
            rw_locks_table_state: TableState::default().with_selected(0),
            mutexes_table_state: TableState::default().with_selected(0),
            sql_table_state: TableState::default().with_selected(0),
            http_table_state: TableState::default().with_selected(0),
            server_table_state: TableState::default().with_selected(0),
            io_bytes_table_state: TableState::default().with_selected(0),
            io_focus: IoFocus::List,
            show_sql_logs: false,
            sql_logs: None,
            sql_logs_table_state: TableState::default(),
            inspected_sql_log: None,
            show_http_logs: false,
            http_logs: None,
            http_logs_table_state: TableState::default(),
            inspected_http_log: None,
            data_flow_focus: DataFlowFocus::List,
            show_data_flow_logs: false,
            data_flow_logs: None,
            data_flow_logs_table_state: TableState::default(),
            inspected_data_flow_log: None,
            threads: JsonThreadsList {
                current_elapsed_ns: 0,
                sample_interval_ms: 250,
                data: vec![],
                total_count: 0,
                included_count: 0,
                thread_count: 0,
                rss_bytes: None,
                rss_bytes_max: None,
                total_alloc_bytes: None,
                total_dealloc_bytes: None,
                alloc_dealloc_diff: None,
            },
            threads_table_state: TableState::default().with_selected(0),
            debug_stats: Vec::new(),
            debug_table_state: TableState::default().with_selected(0),
            debug_focus: DebugFocus::Debug,
            show_debug_logs: false,
            debug_logs: None,
            debug_logs_table_state: TableState::default(),
            inspected_debug_log: None,

            tokio_runtime: None,
            runtime_table_state: TableState::default().with_selected(0),
            loading_runtime: false,

            program_uptime: None,
            program_pid: None,
            auto_expand_logs,
            auto_select_index,
            pending_g: None,
            tab_hit_areas: Vec::new(),
            sub_tab_hit_areas: Vec::new(),
        }
    }

    pub(crate) fn exit(&mut self) {
        self.exit = true;
    }

    pub(crate) fn active_function_count(&self) -> usize {
        match self.functions_sub_tab {
            FunctionsSubTab::Timing => self.timing_functions.data.len(),
            FunctionsSubTab::Memory => self.memory_functions.data.len(),
            FunctionsSubTab::Cpu => self
                .cpu_envelope
                .as_ref()
                .and_then(|e| e.report.as_ref().map(|r| r.data.len()))
                .unwrap_or(0),
        }
    }

    pub(crate) fn active_table_state_mut(&mut self) -> &mut TableState {
        match self.selected_tab {
            SelectedTab::Functions => match self.functions_sub_tab {
                FunctionsSubTab::Timing => &mut self.timing_table_state,
                FunctionsSubTab::Memory => &mut self.memory_table_state,
                FunctionsSubTab::Cpu => &mut self.cpu_table_state,
            },
            SelectedTab::DataFlow => match self.data_flow_sub_tab {
                DataFlowSubTab::Channels => &mut self.channels_table_state,
                DataFlowSubTab::Streams => &mut self.streams_table_state,
                DataFlowSubTab::Futures => &mut self.futures_table_state,
                DataFlowSubTab::RwLocks => &mut self.rw_locks_table_state,
                DataFlowSubTab::Mutexes => &mut self.mutexes_table_state,
            },
            SelectedTab::Io => match self.io_sub_tab {
                IoSubTab::Sql => &mut self.sql_table_state,
                IoSubTab::Http => &mut self.http_table_state,
                IoSubTab::Server => &mut self.server_table_state,
                IoSubTab::Bytes => &mut self.io_bytes_table_state,
            },
            SelectedTab::Threads => &mut self.threads_table_state,
            SelectedTab::Debug => &mut self.debug_table_state,
            SelectedTab::Runtime => &mut self.runtime_table_state,
        }
    }

    pub(crate) fn data_flow_entries_len(&self) -> usize {
        match self.data_flow_sub_tab {
            DataFlowSubTab::Channels => self.channels.data.len(),
            DataFlowSubTab::Streams => self.streams.data.len(),
            DataFlowSubTab::Futures => self.futures.data.len(),
            DataFlowSubTab::RwLocks => self.rw_locks.data.len(),
            DataFlowSubTab::Mutexes => self.mutexes.data.len(),
        }
    }

    pub(crate) fn io_entries_len(&self) -> usize {
        match self.io_sub_tab {
            IoSubTab::Sql => self.sql.data.len(),
            IoSubTab::Http => self.http.data.len(),
            IoSubTab::Server => self.server.data.len(),
            IoSubTab::Bytes => self.io_bytes.data.len(),
        }
    }

    pub(crate) fn data_flow_table_state(&self) -> &TableState {
        match self.data_flow_sub_tab {
            DataFlowSubTab::Channels => &self.channels_table_state,
            DataFlowSubTab::Streams => &self.streams_table_state,
            DataFlowSubTab::Futures => &self.futures_table_state,
            DataFlowSubTab::RwLocks => &self.rw_locks_table_state,
            DataFlowSubTab::Mutexes => &self.mutexes_table_state,
        }
    }

    pub(crate) fn data_flow_table_state_mut(&mut self) -> &mut TableState {
        match self.data_flow_sub_tab {
            DataFlowSubTab::Channels => &mut self.channels_table_state,
            DataFlowSubTab::Streams => &mut self.streams_table_state,
            DataFlowSubTab::Futures => &mut self.futures_table_state,
            DataFlowSubTab::RwLocks => &mut self.rw_locks_table_state,
            DataFlowSubTab::Mutexes => &mut self.mutexes_table_state,
        }
    }

    pub(crate) fn selected_channel_id(&self) -> Option<u32> {
        let idx = self.channels_table_state.selected()?;
        self.channels.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn selected_stream_id(&self) -> Option<u32> {
        let idx = self.streams_table_state.selected()?;
        self.streams.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn selected_future_id(&self) -> Option<u32> {
        let idx = self.futures_table_state.selected()?;
        self.futures.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn selected_sql_id(&self) -> Option<u32> {
        let idx = self.sql_table_state.selected()?;
        self.sql.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn selected_http_id(&self) -> Option<u32> {
        let idx = self.http_table_state.selected()?;
        self.http.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn selected_server_id(&self) -> Option<u32> {
        let idx = self.server_table_state.selected()?;
        self.server.data.get(idx).map(|e| e.id)
    }

    pub(crate) fn run(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> std::io::Result<()> {
        use crossbeam_channel::RecvTimeoutError;

        self.request_refresh_for_current_tab();

        while !self.exit {
            terminal.draw(|frame| super::views::render_ui(frame, self))?;

            // `recv_timeout` on the wrap receiver routes through the instrumented
            // endpoint, so latency is tracked; a timeout refreshes the current tab.
            match self.event_rx.recv_timeout(self.refresh_interval) {
                Ok(AppEvent::Key(key_code)) => self.handle_key_event(key_code),
                Ok(AppEvent::Mouse(mouse_event)) => self.handle_mouse_event(mouse_event),
                Ok(AppEvent::Data(response)) => self.handle_data_response(*response),
                Err(RecvTimeoutError::Timeout) => {
                    if !self.paused {
                        self.request_refresh_for_current_tab();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {}
            }
        }

        Ok(())
    }
}
