//! Data management - fetching, updating, and transforming functions/data flow

use crate::cmd::console::app::{
    App, DataFlowLogs, DataFlowSubTab, FunctionsSubTab, IoSubTab, SelectedTab,
};
use crate::cmd::console::events::{DataRequest, DataResponse};
use hotpath::dev_logging::{trace, warn};
use hotpath::json::{
    DebugEntryType, JsonChannelLogsList, JsonChannelsList, JsonDebugList,
    JsonFunctionAllocLogsList, JsonFunctionEntry, JsonFunctionTimingLogsList,
    JsonFunctionsCpuEnvelope, JsonFunctionsList, JsonFutureLogsList, JsonFuturesList, JsonHttpList,
    JsonIoList, JsonMutexesList, JsonRwLocksList, JsonServerList, JsonSqlList, JsonStreamLogsList,
    JsonStreamsList, JsonThreadsList,
};
use std::time::Instant;

#[hotpath::measure_all]
impl App {
    fn try_auto_expand_logs(&mut self) {
        if !self.auto_expand_logs {
            return;
        }
        match self.selected_tab {
            SelectedTab::Functions => match self.functions_sub_tab {
                FunctionsSubTab::Timing if !self.timing_functions.data.is_empty() => {
                    self.auto_expand_logs = false;
                    self.toggle_function_logs();
                }
                FunctionsSubTab::Memory if !self.memory_functions.data.is_empty() => {
                    self.auto_expand_logs = false;
                    self.toggle_function_logs();
                }
                _ => {}
            },
            SelectedTab::DataFlow if self.data_flow_entries_len() > 0 => {
                self.auto_expand_logs = false;
                self.toggle_data_flow_logs();
            }
            SelectedTab::Io if self.io_entries_len() > 0 => {
                self.auto_expand_logs = false;
                self.toggle_io_logs();
            }
            SelectedTab::Debug if !self.debug_stats.is_empty() => {
                self.auto_expand_logs = false;
                self.toggle_debug_logs();
            }
            SelectedTab::Threads | SelectedTab::Runtime => {
                self.auto_expand_logs = false;
            }
            _ => {}
        }
    }

    pub(crate) fn update_timing_metrics(&mut self, metrics: JsonFunctionsList) {
        let selected_function_id = self.selected_function_id();

        self.timing_functions = metrics;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let entries = &self.timing_functions.data;

        if let Some(idx) = self.auto_select_index {
            let clamped = idx.min(entries.len().saturating_sub(1));
            if !entries.is_empty() {
                self.timing_table_state.select(Some(clamped));
                self.update_and_request_function_logs();
            }
        } else if let Some(function_id) = selected_function_id {
            if let Some(new_idx) = entries.iter().position(|f| f.id == Some(function_id)) {
                self.timing_table_state.select(Some(new_idx));
            } else if !entries.is_empty() {
                self.timing_table_state.select(Some(entries.len() - 1));
            }
        } else if let Some(selected) = self.timing_table_state.selected() {
            if selected >= entries.len() && !entries.is_empty() {
                self.timing_table_state.select(Some(entries.len() - 1));
            }
        } else if !entries.is_empty() {
            self.timing_table_state.select(Some(0));
        }

        self.try_auto_expand_logs();
    }

    pub(crate) fn update_memory_metrics(&mut self, metrics: JsonFunctionsList) {
        let selected_function_id = self.selected_function_id();

        self.memory_functions = metrics;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let entries = &self.memory_functions.data;

        if let Some(idx) = self.auto_select_index {
            let clamped = idx.min(entries.len().saturating_sub(1));
            if !entries.is_empty() {
                self.memory_table_state.select(Some(clamped));
                self.update_and_request_function_logs();
            }
        } else if let Some(function_id) = selected_function_id {
            if let Some(new_idx) = entries.iter().position(|f| f.id == Some(function_id)) {
                self.memory_table_state.select(Some(new_idx));
            } else if !entries.is_empty() {
                self.memory_table_state.select(Some(entries.len() - 1));
            }
        } else if let Some(selected) = self.memory_table_state.selected() {
            if selected >= entries.len() && !entries.is_empty() {
                self.memory_table_state.select(Some(entries.len() - 1));
            }
        } else if !entries.is_empty() {
            self.memory_table_state.select(Some(0));
        }

        self.try_auto_expand_logs();
    }

    pub(crate) fn update_cpu_envelope(&mut self, envelope: JsonFunctionsCpuEnvelope) {
        self.cpu_envelope = Some(envelope);
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;
    }

    pub(crate) fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
    }

    #[hotpath::measure(log = true)]
    pub(crate) fn get_timing_measurements(&self) -> &[JsonFunctionEntry] {
        &self.timing_functions.data
    }

    #[hotpath::measure(log = true)]
    pub(crate) fn get_memory_measurements(&self) -> &[JsonFunctionEntry] {
        &self.memory_functions.data
    }

    #[hotpath::measure(log = true)]
    pub(crate) fn selected_function_id(&self) -> Option<u32> {
        if self.selected_tab != SelectedTab::Functions {
            return None;
        }
        let (entries, table_state) = match self.functions_sub_tab {
            FunctionsSubTab::Timing => (self.get_timing_measurements(), &self.timing_table_state),
            FunctionsSubTab::Memory => (self.get_memory_measurements(), &self.memory_table_state),
            FunctionsSubTab::Cpu => return None,
        };
        table_state
            .selected()
            .and_then(|idx| entries.get(idx).and_then(|f| f.id))
    }

    pub(crate) fn selected_function_name(&self) -> Option<String> {
        if self.selected_tab != SelectedTab::Functions {
            return None;
        }
        let (entries, table_state) = match self.functions_sub_tab {
            FunctionsSubTab::Timing => (self.get_timing_measurements(), &self.timing_table_state),
            FunctionsSubTab::Memory => (self.get_memory_measurements(), &self.memory_table_state),
            FunctionsSubTab::Cpu => return None,
        };
        table_state
            .selected()
            .and_then(|idx| entries.get(idx).map(|f| f.name.clone()))
    }

    pub(crate) fn update_timing_logs(&mut self, logs: JsonFunctionTimingLogsList) {
        self.current_timing_logs = Some(logs);
    }

    pub(crate) fn update_alloc_logs(&mut self, logs: JsonFunctionAllocLogsList) {
        self.current_alloc_logs = Some(logs);
    }

    pub(crate) fn update_pinned_function(&mut self) {
        if self.show_function_logs {
            self.pinned_function_id = self.selected_function_id();
            self.pinned_function = self.selected_function_name();
        }
    }

    pub(crate) fn request_function_logs_if_open(&self) {
        if self.show_function_logs && self.selected_tab == SelectedTab::Functions {
            if let Some(function_id) = self.pinned_function_id {
                match self.functions_sub_tab {
                    FunctionsSubTab::Timing => {
                        let _ = self
                            .request_tx
                            .send(DataRequest::FetchFunctionLogsTiming(function_id));
                    }
                    FunctionsSubTab::Memory => {
                        let _ = self
                            .request_tx
                            .send(DataRequest::FetchFunctionLogsAlloc(function_id));
                    }
                    FunctionsSubTab::Cpu => {}
                }
            }
        }
    }

    pub(crate) fn update_and_request_function_logs(&mut self) {
        self.update_pinned_function();
        self.request_function_logs_if_open();
    }

    // Data Flow methods

    pub(crate) fn update_channels(&mut self, channels: JsonChannelsList) {
        let selected_id = self.selected_channel_id();
        self.channels = channels;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.channels.data.len();
        if let Some(idx) = self.auto_select_index {
            if len > 0 {
                self.channels_table_state.select(Some(idx.min(len - 1)));
            }
        } else if let Some(id) = selected_id {
            if let Some(new_idx) = self.channels.data.iter().position(|e| e.id == Some(id)) {
                self.channels_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.channels_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.channels_table_state.selected() {
            if selected >= len && len > 0 {
                self.channels_table_state.select(Some(len - 1));
            }
        }

        if self.show_data_flow_logs && self.data_flow_sub_tab == DataFlowSubTab::Channels {
            self.request_data_flow_logs();
        }
        self.try_auto_expand_logs();
    }

    pub(crate) fn update_streams(&mut self, streams: JsonStreamsList) {
        let selected_id = self.selected_stream_id();
        self.streams = streams;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.streams.data.len();
        if let Some(idx) = self.auto_select_index {
            if len > 0 {
                self.streams_table_state.select(Some(idx.min(len - 1)));
            }
        } else if let Some(id) = selected_id {
            if let Some(new_idx) = self.streams.data.iter().position(|e| e.id == Some(id)) {
                self.streams_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.streams_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.streams_table_state.selected() {
            if selected >= len && len > 0 {
                self.streams_table_state.select(Some(len - 1));
            }
        }

        if self.show_data_flow_logs && self.data_flow_sub_tab == DataFlowSubTab::Streams {
            self.request_data_flow_logs();
        }
        self.try_auto_expand_logs();
    }

    pub(crate) fn update_futures(&mut self, futures: JsonFuturesList) {
        let selected_id = self.selected_future_id();
        self.futures = futures;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.futures.data.len();
        if let Some(idx) = self.auto_select_index {
            if len > 0 {
                self.futures_table_state.select(Some(idx.min(len - 1)));
            }
        } else if let Some(id) = selected_id {
            if let Some(new_idx) = self.futures.data.iter().position(|e| e.id == Some(id)) {
                self.futures_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.futures_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.futures_table_state.selected() {
            if selected >= len && len > 0 {
                self.futures_table_state.select(Some(len - 1));
            }
        }

        if self.show_data_flow_logs && self.data_flow_sub_tab == DataFlowSubTab::Futures {
            self.request_data_flow_logs();
        }
        self.try_auto_expand_logs();
    }

    pub(crate) fn update_rw_locks(&mut self, rw_locks: JsonRwLocksList) {
        self.rw_locks = rw_locks;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.rw_locks.data.len();
        if let Some(selected) = self.rw_locks_table_state.selected() {
            if selected >= len && len > 0 {
                self.rw_locks_table_state.select(Some(len - 1));
            }
        }
    }

    pub(crate) fn update_mutexes(&mut self, mutexes: JsonMutexesList) {
        self.mutexes = mutexes;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.mutexes.data.len();
        if let Some(selected) = self.mutexes_table_state.selected() {
            if selected >= len && len > 0 {
                self.mutexes_table_state.select(Some(len - 1));
            }
        }
    }

    pub(crate) fn update_sql(&mut self, sql: JsonSqlList) {
        let selected_id = self.selected_sql_id();
        self.sql = sql;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.sql.data.len();
        // Entries re-sort by total time on every refresh; follow the selected
        // query by id so the cursor (and its open logs) don't jump rows.
        if let Some(id) = selected_id {
            if let Some(new_idx) = self.sql.data.iter().position(|e| e.id == Some(id)) {
                self.sql_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.sql_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.sql_table_state.selected() {
            if selected >= len && len > 0 {
                self.sql_table_state.select(Some(len - 1));
            }
        }

        if self.show_sql_logs {
            self.request_sql_logs();
        }
        self.try_auto_expand_logs();
    }

    pub(crate) fn request_sql_logs(&self) {
        if self.paused {
            return;
        }

        if let Some(id) = self.selected_sql_id() {
            let _ = self.request_tx.send(DataRequest::FetchSqlLogs(id));
        }
    }

    pub(crate) fn handle_sql_logs(&mut self, _id: u32, logs: hotpath::json::JsonSqlLogsList) {
        self.sql_logs = Some(logs);

        let log_count = self.sql_logs.as_ref().map(|l| l.logs.len()).unwrap_or(0);
        if let Some(selected) = self.sql_logs_table_state.selected() {
            if selected >= log_count && log_count > 0 {
                self.sql_logs_table_state.select(Some(log_count - 1));
            }
        }
    }

    pub(crate) fn update_http(&mut self, http: JsonHttpList) {
        let selected_id = self.selected_http_id();
        self.http = http;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.http.data.len();
        // Entries re-sort by total time on every refresh; follow the selected
        // endpoint by id so the cursor (and its open logs) don't jump rows.
        if let Some(id) = selected_id {
            if let Some(new_idx) = self.http.data.iter().position(|e| e.id == Some(id)) {
                self.http_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.http_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.http_table_state.selected() {
            if selected >= len && len > 0 {
                self.http_table_state.select(Some(len - 1));
            }
        }

        if self.show_http_logs {
            self.request_http_logs();
        }
        self.try_auto_expand_logs();
    }

    /// Requests logs for the HTTP-shaped subtab currently shown (HTTP or
    /// Server); both land in the shared `http_logs` pane.
    pub(crate) fn request_http_logs(&self) {
        if self.paused {
            return;
        }

        let request = match self.io_sub_tab {
            IoSubTab::Http => self.selected_http_id().map(DataRequest::FetchHttpLogs),
            IoSubTab::Server => self.selected_server_id().map(DataRequest::FetchServerLogs),
            _ => None,
        };
        if let Some(request) = request {
            let _ = self.request_tx.send(request);
        }
    }

    pub(crate) fn update_server(&mut self, server: JsonServerList) {
        let selected_id = self.selected_server_id();
        self.server = server;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.server.data.len();
        if let Some(id) = selected_id {
            if let Some(new_idx) = self.server.data.iter().position(|e| e.id == Some(id)) {
                self.server_table_state.select(Some(new_idx));
            } else if len > 0 {
                self.server_table_state.select(Some(len - 1));
            }
        } else if let Some(selected) = self.server_table_state.selected() {
            if selected >= len && len > 0 {
                self.server_table_state.select(Some(len - 1));
            }
        }

        if self.show_http_logs {
            self.request_http_logs();
        }
        self.try_auto_expand_logs();
    }

    pub(crate) fn handle_http_logs(&mut self, _id: u32, logs: hotpath::json::JsonHttpLogsList) {
        self.http_logs = Some(logs);

        let log_count = self.http_logs.as_ref().map(|l| l.logs.len()).unwrap_or(0);
        if let Some(selected) = self.http_logs_table_state.selected() {
            if selected >= log_count && log_count > 0 {
                self.http_logs_table_state.select(Some(log_count - 1));
            }
        }
    }

    pub(crate) fn update_io_bytes(&mut self, io_bytes: JsonIoList) {
        self.io_bytes = io_bytes;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        let len = self.io_bytes.data.len();
        if let Some(selected) = self.io_bytes_table_state.selected() {
            if selected >= len && len > 0 {
                self.io_bytes_table_state.select(Some(len - 1));
            }
        }
    }

    pub(crate) fn request_data_flow_logs(&self) {
        if self.paused {
            return;
        }

        let request = match self.data_flow_sub_tab {
            DataFlowSubTab::Channels => self
                .channels_table_state
                .selected()
                .and_then(|i| self.channels.data.get(i))
                .and_then(|e| e.id)
                .map(DataRequest::FetchChannelLogs),
            DataFlowSubTab::Streams => self
                .streams_table_state
                .selected()
                .and_then(|i| self.streams.data.get(i))
                .and_then(|e| e.id)
                .map(DataRequest::FetchStreamLogs),
            DataFlowSubTab::Futures => self
                .futures_table_state
                .selected()
                .and_then(|i| self.futures.data.get(i))
                .and_then(|e| e.id)
                .map(DataRequest::FetchFutureLogs),
            DataFlowSubTab::RwLocks => None,
            DataFlowSubTab::Mutexes => None,
        };

        if let Some(req) = request {
            let _ = self.request_tx.send(req);
        }
    }

    pub(crate) fn handle_data_flow_channel_logs(&mut self, _id: u32, logs: JsonChannelLogsList) {
        self.data_flow_logs = Some(DataFlowLogs::Channel(logs));
        self.ensure_data_flow_logs_selection_valid();
    }

    pub(crate) fn handle_data_flow_stream_logs(&mut self, _id: u32, logs: JsonStreamLogsList) {
        self.data_flow_logs = Some(DataFlowLogs::Stream(logs));
        self.ensure_data_flow_logs_selection_valid();
    }

    pub(crate) fn handle_data_flow_future_logs(&mut self, _id: u32, calls: JsonFutureLogsList) {
        self.data_flow_logs = Some(DataFlowLogs::Future(calls));
        self.ensure_data_flow_logs_selection_valid();
    }

    fn ensure_data_flow_logs_selection_valid(&mut self) {
        let log_count = match &self.data_flow_logs {
            Some(DataFlowLogs::Channel(l)) => l.sent_logs.len(),
            Some(DataFlowLogs::Stream(l)) => l.logs.len(),
            Some(DataFlowLogs::Future(l)) => l.calls.len(),
            None => 0,
        };

        if let Some(selected) = self.data_flow_logs_table_state.selected() {
            if selected >= log_count && log_count > 0 {
                self.data_flow_logs_table_state.select(Some(log_count - 1));
            }
        }
    }

    // Threads methods

    pub(crate) fn update_threads(&mut self, threads: JsonThreadsList) {
        let selected_thread_tid = self
            .threads_table_state
            .selected()
            .and_then(|idx| self.threads.data.get(idx))
            .map(|stat| stat.os_tid);

        self.threads = threads;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        if let Some(idx) = self.auto_select_index {
            if !self.threads.data.is_empty() {
                let clamped = idx.min(self.threads.data.len() - 1);
                self.threads_table_state.select(Some(clamped));
            }
        } else if let Some(thread_tid) = selected_thread_tid {
            if let Some(new_idx) = self
                .threads
                .data
                .iter()
                .position(|stat| stat.os_tid == thread_tid)
            {
                self.threads_table_state.select(Some(new_idx));
            } else if !self.threads.data.is_empty() {
                self.threads_table_state
                    .select(Some(self.threads.data.len() - 1));
            }
        } else if let Some(selected) = self.threads_table_state.selected() {
            if selected >= self.threads.data.len() && !self.threads.data.is_empty() {
                self.threads_table_state
                    .select(Some(self.threads.data.len() - 1));
            }
        }

        self.try_auto_expand_logs();
    }

    // Debug methods

    pub(crate) fn update_debug(&mut self, debug: JsonDebugList) {
        let selected_id = self
            .debug_table_state
            .selected()
            .and_then(|idx| self.debug_stats.get(idx))
            .and_then(|stat| stat.id);

        self.debug_stats = debug.entries;
        self.last_successful_fetch = Some(Instant::now());
        self.error_message = None;

        if let Some(idx) = self.auto_select_index {
            if !self.debug_stats.is_empty() {
                let clamped = idx.min(self.debug_stats.len() - 1);
                self.debug_table_state.select(Some(clamped));
            }
        } else if let Some(id) = selected_id {
            if let Some(new_idx) = self.debug_stats.iter().position(|stat| stat.id == Some(id)) {
                self.debug_table_state.select(Some(new_idx));
            } else if !self.debug_stats.is_empty() {
                self.debug_table_state
                    .select(Some(self.debug_stats.len() - 1));
            }
        } else if let Some(selected) = self.debug_table_state.selected() {
            if selected >= self.debug_stats.len() && !self.debug_stats.is_empty() {
                self.debug_table_state
                    .select(Some(self.debug_stats.len() - 1));
            }
        }

        if self.show_debug_logs {
            self.request_debug_logs();
        }

        self.try_auto_expand_logs();
    }

    pub(crate) fn request_debug_logs(&self) {
        if self.paused {
            return;
        }

        if let Some(selected) = self.debug_table_state.selected() {
            if !self.debug_stats.is_empty() && selected < self.debug_stats.len() {
                let stat = &self.debug_stats[selected];
                let Some(id) = stat.id else {
                    return;
                };
                let request = match stat.entry_type {
                    DebugEntryType::Dbg => DataRequest::FetchDebugDbgLogs(id),
                    DebugEntryType::Val => DataRequest::FetchDebugValLogs(id),
                    DebugEntryType::Gauge => DataRequest::FetchDebugGaugeLogs(id),
                };
                let _ = self.request_tx.send(request);
            }
        }
    }

    pub(crate) fn handle_debug_logs(&mut self, logs: Vec<hotpath::json::JsonDebugLog>) {
        self.debug_logs = Some(logs);

        if let Some(ref cached_logs) = self.debug_logs {
            let log_count = cached_logs.len();
            if let Some(selected) = self.debug_logs_table_state.selected() {
                if selected >= log_count && log_count > 0 {
                    self.debug_logs_table_state.select(Some(log_count - 1));
                }
            }
        }
    }

    // Refresh and response handling

    pub(crate) fn request_refresh_for_current_tab(&mut self) {
        let request = match self.selected_tab {
            SelectedTab::Functions => {
                self.loading_functions = true;
                match self.functions_sub_tab {
                    FunctionsSubTab::Timing => DataRequest::RefreshTiming,
                    FunctionsSubTab::Memory => DataRequest::RefreshMemory,
                    FunctionsSubTab::Cpu => DataRequest::RefreshCpu,
                }
            }
            SelectedTab::DataFlow => {
                self.loading_data_flow = true;
                match self.data_flow_sub_tab {
                    DataFlowSubTab::Channels => DataRequest::RefreshChannels,
                    DataFlowSubTab::Streams => DataRequest::RefreshStreams,
                    DataFlowSubTab::Futures => DataRequest::RefreshFutures,
                    DataFlowSubTab::RwLocks => DataRequest::RefreshRwLocks,
                    DataFlowSubTab::Mutexes => DataRequest::RefreshMutexes,
                }
            }
            SelectedTab::Io => {
                self.loading_io = true;
                match self.io_sub_tab {
                    IoSubTab::Sql => DataRequest::RefreshSql,
                    IoSubTab::Http => DataRequest::RefreshHttp,
                    IoSubTab::Server => DataRequest::RefreshServer,
                    IoSubTab::Bytes => DataRequest::RefreshIo,
                }
            }
            SelectedTab::Threads => {
                self.loading_threads = true;
                DataRequest::RefreshThreads
            }
            SelectedTab::Debug => {
                self.loading_debug = true;
                DataRequest::RefreshDebug
            }
            SelectedTab::Runtime => {
                self.loading_runtime = true;
                DataRequest::RefreshTokioRuntime
            }
        };
        trace!("Requesting refresh for tab: {}", self.selected_tab.name());
        let _ = self.request_tx.send(request);
        let _ = self.request_tx.send(DataRequest::FetchProfilerStatus);
        self.last_refresh = Instant::now();
    }

    pub(crate) fn handle_data_response(&mut self, response: DataResponse) {
        match response {
            DataResponse::FunctionsTiming(data) => {
                trace!("Received timing data: {} functions", data.data.len());
                self.loading_functions = false;
                self.update_timing_metrics(data);
                self.request_function_logs_if_open();
            }
            DataResponse::FunctionsAlloc(data) => {
                trace!("Received alloc data: {} functions", data.data.len());
                self.loading_functions = false;
                self.memory_unavailable_reason = None;
                self.update_memory_metrics(data);
                self.request_function_logs_if_open();
            }
            DataResponse::FunctionsAllocUnavailable(reason) => {
                trace!("Memory profiling unavailable: {}", reason);
                self.loading_functions = false;
                self.memory_unavailable_reason = Some(reason);
            }
            DataResponse::FunctionsCpu(envelope) => {
                trace!("Received CPU envelope: status={:?}", envelope.status);
                self.loading_functions = false;
                self.cpu_unavailable_reason = None;
                self.update_cpu_envelope(envelope);
            }
            DataResponse::FunctionsCpuUnavailable(reason) => {
                trace!("CPU profiling unavailable: {}", reason);
                self.loading_functions = false;
                self.cpu_unavailable_reason = Some(reason);
            }
            DataResponse::CpuSnapshotTriggered => {
                trace!("CPU snapshot triggered");
            }
            DataResponse::CpuSnapshotBusy => {
                trace!("CPU snapshot already in progress");
            }
            DataResponse::FunctionLogsTiming {
                function_id: _,
                logs,
            } => {
                trace!("Received function timing logs: {} entries", logs.logs.len());
                self.update_timing_logs(logs);
            }
            DataResponse::FunctionLogsTimingNotFound(_) => {
                self.current_timing_logs = None;
            }
            DataResponse::FunctionLogsAlloc {
                function_id: _,
                logs,
            } => {
                trace!("Received function alloc logs: {} entries", logs.logs.len());
                self.update_alloc_logs(logs);
            }
            DataResponse::FunctionLogsAllocNotFound(_) => {
                self.current_alloc_logs = None;
            }
            DataResponse::Channels(data) => {
                trace!("Received channels: {} entries", data.data.len());
                self.loading_data_flow = false;
                self.update_channels(data);
            }
            DataResponse::Streams(data) => {
                trace!("Received streams: {} entries", data.data.len());
                self.loading_data_flow = false;
                self.update_streams(data);
            }
            DataResponse::Futures(data) => {
                trace!("Received futures: {} entries", data.data.len());
                self.loading_data_flow = false;
                self.update_futures(data);
            }
            DataResponse::RwLocks(data) => {
                trace!("Received rw_locks: {} entries", data.data.len());
                self.loading_data_flow = false;
                self.update_rw_locks(data);
            }
            DataResponse::Mutexes(data) => {
                trace!("Received mutexes: {} entries", data.data.len());
                self.loading_data_flow = false;
                self.update_mutexes(data);
            }
            DataResponse::Sql(data) => {
                trace!("Received sql: {} entries", data.data.len());
                self.loading_io = false;
                self.update_sql(data);
            }
            DataResponse::Http(data) => {
                trace!("Received http: {} entries", data.data.len());
                self.loading_io = false;
                self.update_http(data);
            }
            DataResponse::Server(data) => {
                trace!("Received server: {} entries", data.data.len());
                self.loading_io = false;
                self.update_server(data);
            }
            DataResponse::Io(data) => {
                trace!("Received io: {} entries", data.data.len());
                self.loading_io = false;
                self.update_io_bytes(data);
            }
            DataResponse::ChannelLogs { id, logs } => {
                trace!(
                    "Received channel {} logs: {} sent, {} received",
                    id,
                    logs.sent_logs.len(),
                    logs.received_logs.len()
                );
                self.handle_data_flow_channel_logs(id, logs);
            }
            DataResponse::StreamLogs { id, logs } => {
                trace!("Received stream {} logs: {} entries", id, logs.logs.len());
                self.handle_data_flow_stream_logs(id, logs);
            }
            DataResponse::FutureLogs { id, calls } => {
                trace!(
                    "Received future {} calls: {} entries",
                    id,
                    calls.calls.len()
                );
                self.handle_data_flow_future_logs(id, calls);
            }
            DataResponse::ChannelLogsNotFound { .. }
            | DataResponse::StreamLogsNotFound { .. }
            | DataResponse::FutureLogsNotFound { .. } => {
                self.data_flow_logs = None;
            }
            DataResponse::SqlLogs { id, logs } => {
                trace!("Received sql {} logs: {} entries", id, logs.logs.len());
                self.handle_sql_logs(id, logs);
            }
            DataResponse::SqlLogsNotFound { .. } => {
                self.sql_logs = None;
            }
            DataResponse::HttpLogs { id, logs } => {
                trace!("Received http {} logs: {} entries", id, logs.logs.len());
                self.handle_http_logs(id, logs);
            }
            DataResponse::ServerLogs { id, logs } => {
                trace!("Received server {} logs: {} entries", id, logs.logs.len());
                self.handle_http_logs(id, logs);
            }
            DataResponse::HttpLogsNotFound { .. } | DataResponse::ServerLogsNotFound { .. } => {
                self.http_logs = None;
            }
            DataResponse::Threads(data) => {
                trace!("Received threads data: {} threads", data.data.len());
                self.loading_threads = false;
                self.update_threads(data);
            }
            DataResponse::Debug(data) => {
                trace!("Received debug data: {} entries", data.entries.len());
                self.loading_debug = false;
                self.update_debug(data);
            }
            DataResponse::DebugDbgLogs { id, logs } => {
                trace!("Received dbg logs for {}: {} entries", id, logs.len());
                self.handle_debug_logs(logs);
            }
            DataResponse::DebugValLogs { id, logs } => {
                trace!("Received val logs for {}: {} entries", id, logs.len());
                self.handle_debug_logs(logs);
            }
            DataResponse::DebugGaugeLogs { id, logs } => {
                trace!("Received gauge logs for {}: {} entries", id, logs.len());
                self.handle_debug_logs(logs);
            }
            DataResponse::DebugLogsNotFound { .. } => {
                self.debug_logs = None;
            }
            DataResponse::TokioRuntime(snapshot) => {
                trace!("Received tokio runtime snapshot");
                self.loading_runtime = false;
                self.tokio_runtime = Some(snapshot);
                self.last_successful_fetch = Some(Instant::now());
                self.error_message = None;
                self.try_auto_expand_logs();
            }
            DataResponse::ProfilerStatus(status) => {
                trace!(
                    "Received profiler status: pid={} uptime={}",
                    status.pid,
                    status.uptime
                );
                self.program_uptime = Some(status.uptime);
                self.program_pid = Some(status.pid);
            }
            DataResponse::Error(e) => {
                warn!("Data fetch error: {}", e);
                self.loading_functions = false;
                self.loading_data_flow = false;
                self.loading_io = false;
                self.loading_threads = false;
                self.loading_debug = false;
                self.loading_runtime = false;
                self.set_error(e);
            }
        }
    }
}
