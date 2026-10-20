//! hotpath-rs is a simple async Rust profiler. It instruments functions, channels, futures, and streams to quickly find bottlenecks and focus optimizations where they matter most.
//! It can provide actionable insights into time, memory, and data flow with minimal setup.
//! ## Setup & Usage
//! For a complete setup guide, examples, and advanced configuration, visit
//! [hotpath.rs](https://hotpath.rs).

// Meta crate mirrors the main crate; some code is conditionally dead
// depending on feature combinations (e.g. alloc code without global_allocator).
#![allow(dead_code)]

#[cfg(all(
    feature = "hotpath-cpu-meta",
    not(any(target_os = "macos", target_os = "linux"))
))]
compile_error!("the `hotpath-cpu-meta` feature is only supported on macOS and Linux");

#[cfg(all(feature = "hotpath-alloc-meta", not(feature = "hotpath-meta")))]
compile_error!("the `hotpath-alloc-meta` feature requires the `hotpath-meta` feature");

#[cfg(all(feature = "hotpath-cpu-meta", not(feature = "hotpath-meta")))]
compile_error!("the `hotpath-cpu-meta` feature requires the `hotpath-meta` feature");

#[cfg(all(feature = "hotpath-prometheus-meta", not(feature = "hotpath-meta")))]
compile_error!("the `hotpath-prometheus-meta` feature requires the `hotpath-meta` feature");

#[cfg(all(feature = "hotpath-cloud-meta", not(feature = "hotpath-meta")))]
compile_error!("the `hotpath-cloud-meta` feature requires the `hotpath-meta` feature");

#[cfg(feature = "hotpath-meta")]
#[doc(inline)]
pub use lib_on::*;
#[cfg(feature = "hotpath-meta")]
mod lib_on;

#[cfg(all(feature = "hotpath-meta", feature = "tokio"))]
pub use lib_on::tokio_runtime;

#[cfg(feature = "json")]
pub(crate) mod output;
#[cfg(feature = "json")]
pub use output::{
    ceil_char_boundary, floor_char_boundary, format_bytes, format_count, format_duration,
    format_percentile_header, format_percentile_key, format_rate, parse_bytes, parse_count,
    parse_duration, shorten_function_name, ProfilingMode, MAX_LOG_LEN,
};
#[cfg(feature = "hotpath-meta")]
pub use output_on::format_debug_truncated;

#[cfg(feature = "hotpath-meta")]
pub(crate) mod output_on;

#[cfg(feature = "hotpath-meta")]
#[doc(hidden)]
pub mod table;

#[cfg(feature = "hotpath-meta")]
pub(crate) mod auth;

#[cfg(feature = "hotpath-meta")]
pub(crate) mod metrics_server;

#[cfg(feature = "hotpath-prometheus-meta")]
pub(crate) mod prometheus_server;

#[cfg(feature = "hotpath-mcp-meta")]
pub(crate) mod mcp_server;

#[allow(dead_code)]
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "json")]
pub use json::Route;

#[cfg(feature = "hotpath-meta")]
#[doc(hidden)]
pub mod instant;
#[cfg(feature = "hotpath-meta")]
pub(crate) mod tid;

#[cfg(not(feature = "hotpath-meta"))]
#[doc(inline)]
pub use lib_off::*;
#[cfg(not(feature = "hotpath-meta"))]
mod lib_off;

/// Mirror of `std` paths so instrumented types can be used as drop-in
/// replacements by prefixing imports with `hotpath_meta::wrap::` (e.g.
/// `hotpath_meta::wrap::std::sync::RwLock`).
pub mod wrap {
    pub mod std {
        pub mod sync {
            #[cfg(feature = "hotpath-meta")]
            pub use crate::lib_on::{
                mutexes::wrapper::std::{Mutex, MutexGuard},
                rw_locks::wrapper::std::{RwLock, RwLockReadGuard, RwLockWriteGuard},
            };
            #[cfg(not(feature = "hotpath-meta"))]
            pub use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

            /// Instrumented `std::sync::mpsc` channel endpoints for
            /// `channel!`. With `hotpath-meta` enabled these are the
            /// instrumented wrappers; otherwise `channel!` is a no-op and the endpoints
            /// are the raw std types, so the alias resolves the same way regardless of
            /// feature configuration.
            pub mod mpsc {
                #[cfg(feature = "hotpath-meta")]
                pub use crate::lib_on::channels::wrapper::std_wrap::{
                    Receiver, Sender, SyncSender,
                };
                #[cfg(not(feature = "hotpath-meta"))]
                pub use std::sync::mpsc::{Receiver, Sender, SyncSender};
            }
        }
    }

    /// Instrumented `tokio::sync::mpsc` channel endpoints for
    /// `channel!`. With `hotpath-meta` enabled these are the
    /// instrumented wrappers; otherwise `channel!` is a no-op and the endpoints
    /// are the raw tokio types, so the alias resolves the same way regardless of
    /// feature configuration.
    #[cfg(feature = "tokio")]
    pub mod tokio {
        pub mod sync {
            pub mod mpsc {
                #[cfg(feature = "hotpath-meta")]
                pub use crate::lib_on::channels::wrapper::tokio_wrap::{
                    Receiver, Sender, UnboundedReceiver, UnboundedSender, WeakSender,
                    WeakUnboundedSender,
                };
                #[cfg(not(feature = "hotpath-meta"))]
                pub use tokio::sync::mpsc::{
                    Receiver, Sender, UnboundedReceiver, UnboundedSender, WeakSender,
                    WeakUnboundedSender,
                };
            }

            /// Instrumented `tokio::sync::oneshot` channel endpoints for the default
            /// `channel!` mode. With `hotpath-meta` enabled these are the instrumented
            /// wrappers; otherwise `channel!` is a no-op and the endpoints are the raw
            /// tokio types, so the alias resolves the same way regardless of feature
            /// configuration.
            pub mod oneshot {
                #[cfg(feature = "hotpath-meta")]
                pub use crate::lib_on::channels::wrapper::tokio_oneshot_wrap::{Receiver, Sender};
                #[cfg(not(feature = "hotpath-meta"))]
                pub use tokio::sync::oneshot::{Receiver, Sender};
            }
        }
    }

    /// Instrumented crossbeam channel endpoints for `channel!`.
    /// With `hotpath-meta` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw crossbeam types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "crossbeam")]
    pub mod crossbeam_channel {
        #[cfg(feature = "hotpath-meta")]
        pub use crate::lib_on::channels::wrapper::crossbeam_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath-meta"))]
        pub use crossbeam_channel::{Receiver, Sender};
    }

    /// Instrumented flume channel endpoints for `channel!`.
    /// With `hotpath-meta` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw flume types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "flume")]
    pub mod flume {
        #[cfg(feature = "hotpath-meta")]
        pub use crate::lib_on::channels::wrapper::flume_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath-meta"))]
        pub use flume::{Receiver, Sender};
    }

    /// Instrumented async-channel endpoints for `channel!`.
    /// With `hotpath-meta` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw async-channel types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "async-channel")]
    pub mod async_channel {
        #[cfg(feature = "hotpath-meta")]
        pub use crate::lib_on::channels::wrapper::asc_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath-meta"))]
        pub use async_channel::{Receiver, Sender};
    }

    /// Instrumented `futures_channel` endpoints for the default `channel!` mode.
    /// With `hotpath-meta` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw futures types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "futures")]
    pub mod futures_channel {
        pub mod mpsc {
            #[cfg(feature = "hotpath-meta")]
            pub use crate::lib_on::channels::wrapper::ftc_wrap::{
                Receiver, Sender, TrySendError, UnboundedReceiver, UnboundedSender,
            };
            #[cfg(not(feature = "hotpath-meta"))]
            pub use futures_channel::mpsc::{
                Receiver, Sender, TrySendError, UnboundedReceiver, UnboundedSender,
            };
        }

        pub mod oneshot {
            #[cfg(feature = "hotpath-meta")]
            pub use crate::lib_on::channels::wrapper::ftc_oneshot_wrap::{Receiver, Sender};
            #[cfg(not(feature = "hotpath-meta"))]
            pub use futures_channel::oneshot::{Receiver, Sender};
        }
    }
}

mod shared;
pub use shared::{env_flag, Format, IntoF64, Section};

#[doc(hidden)]
pub mod dev_logging;
