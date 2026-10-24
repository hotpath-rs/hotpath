//! hotpath-rs is a simple async Rust profiler. It instruments functions, channels, futures, and streams to quickly find bottlenecks and focus optimizations where they matter most.
//! It can provide actionable insights into time, memory, and data flow with minimal setup.
//! ## Setup & Usage
//! For a complete setup guide, examples, and advanced configuration, visit
//! [hotpath.rs](https://hotpath.rs).

#[cfg(all(
    feature = "hotpath-cpu",
    not(any(target_os = "macos", target_os = "linux"))
))]
compile_error!("the `hotpath-cpu` feature is only supported on macOS and Linux");

// Referenced by the #[hotpath::main] expansion when the meta allocator layer
// is enabled, so user crates can name hotpath_meta without depending on it.
#[cfg(feature = "hotpath-meta")]
#[doc(hidden)]
pub use hotpath_meta;

#[cfg(feature = "hotpath")]
#[doc(inline)]
pub use lib_on::*;
#[cfg(feature = "hotpath")]
mod lib_on;

#[cfg(all(feature = "hotpath", feature = "tokio"))]
pub use lib_on::tokio_runtime;

#[cfg(feature = "json")]
pub(crate) mod output;
#[cfg(feature = "json")]
pub use output::{
    ceil_char_boundary, floor_char_boundary, format_bytes, format_count, format_duration,
    format_icount, format_icount_display, format_percentile_header, format_percentile_key,
    format_rate, parse_bytes, parse_count, parse_duration, parse_icount, shorten_function_name,
    ProfilingMode, MAX_LOG_LEN,
};
#[cfg(feature = "hotpath")]
pub use output_on::format_debug_truncated;

#[cfg(feature = "hotpath")]
pub(crate) mod output_on;

#[cfg(feature = "hotpath")]
pub(crate) mod table;

#[cfg(feature = "hotpath")]
pub(crate) mod auth;

#[cfg(feature = "hotpath")]
pub(crate) mod metrics_server;

#[cfg(feature = "hotpath-prometheus")]
pub(crate) mod prometheus_server;

#[cfg(feature = "hotpath-mcp")]
pub(crate) mod mcp_server;

#[allow(dead_code)]
#[cfg(feature = "json")]
pub mod json;
#[cfg(feature = "json")]
pub use json::Route;

#[cfg(feature = "hotpath")]
#[doc(hidden)]
pub mod instant;
#[cfg(feature = "hotpath")]
pub(crate) mod tid;

#[cfg(not(feature = "hotpath"))]
#[doc(inline)]
pub use lib_off::*;
#[cfg(not(feature = "hotpath"))]
mod lib_off;

/// Mirror of `std` paths so instrumented types can be used as drop-in
/// replacements by prefixing imports with `hotpath::wrap::` (e.g.
/// `hotpath::wrap::std::sync::RwLock`).
///
/// These types expose `new(value)` (capturing the caller location as the
/// registered source) so existing `Type::new(..)` call sites keep compiling,
/// but it is deprecated: prefer the [`mutex!`](crate::mutex) /
/// [`rw_lock!`](crate::rw_lock) macros, which also accept a `label`.
pub mod wrap {
    pub mod std {
        pub mod sync {
            #[cfg(feature = "hotpath")]
            pub use crate::lib_on::{
                mutexes::wrapper::std::{Mutex, MutexGuard},
                rw_locks::wrapper::std::{RwLock, RwLockReadGuard, RwLockWriteGuard},
            };
            #[cfg(not(feature = "hotpath"))]
            pub use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

            /// Instrumented `std::sync::mpsc` channel endpoints for
            /// `channel!`. With `hotpath` enabled these are the
            /// instrumented wrappers; otherwise `channel!` is a no-op and the endpoints
            /// are the raw std types, so the alias resolves the same way regardless of
            /// feature configuration.
            pub mod mpsc {
                #[cfg(feature = "hotpath")]
                pub use crate::lib_on::channels::wrapper::std_wrap::{
                    Receiver, Sender, SyncSender,
                };
                #[cfg(not(feature = "hotpath"))]
                pub use std::sync::mpsc::{Receiver, Sender, SyncSender};
            }
        }
    }

    #[cfg(feature = "parking_lot")]
    pub mod parking_lot {
        #[cfg(not(feature = "hotpath"))]
        pub use crate::lib_off::parking_lot::{
            Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
        };
        #[cfg(feature = "hotpath")]
        pub use crate::lib_on::{
            mutexes::wrapper::parking_lot::{Mutex, MutexGuard},
            rw_locks::wrapper::parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard},
        };
    }

    #[cfg(feature = "async-lock")]
    pub mod async_lock {
        #[cfg(not(feature = "hotpath"))]
        pub use crate::lib_off::async_lock::{
            Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
        };
        #[cfg(feature = "hotpath")]
        pub use crate::lib_on::{
            mutexes::wrapper::async_lock::{Mutex, MutexGuard},
            rw_locks::wrapper::async_lock::{RwLock, RwLockReadGuard, RwLockWriteGuard},
        };
    }

    #[cfg(feature = "tokio")]
    pub mod tokio {
        pub mod sync {
            #[cfg(not(feature = "hotpath"))]
            pub use crate::lib_off::tokio::sync::{
                Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
            };
            #[cfg(feature = "hotpath")]
            pub use crate::lib_on::{
                mutexes::wrapper::tokio::{Mutex, MutexGuard},
                rw_locks::wrapper::tokio::{RwLock, RwLockReadGuard, RwLockWriteGuard},
            };

            /// Instrumented `tokio::sync::mpsc` channel endpoints for
            /// `channel!`. With `hotpath` enabled these are the
            /// instrumented wrappers; otherwise `channel!` is a no-op and the endpoints
            /// are the raw tokio types, so the alias resolves the same way regardless of
            /// feature configuration.
            pub mod mpsc {
                #[cfg(feature = "hotpath")]
                pub use crate::lib_on::channels::wrapper::tokio_wrap::{
                    Receiver, Sender, UnboundedReceiver, UnboundedSender, WeakSender,
                    WeakUnboundedSender,
                };
                #[cfg(not(feature = "hotpath"))]
                pub use tokio::sync::mpsc::{
                    Receiver, Sender, UnboundedReceiver, UnboundedSender, WeakSender,
                    WeakUnboundedSender,
                };
            }

            /// Instrumented `tokio::sync::oneshot` channel endpoints for the default
            /// `channel!` mode. With `hotpath` enabled these are the instrumented
            /// wrappers; otherwise `channel!` is a no-op and the endpoints are the raw
            /// tokio types, so the alias resolves the same way regardless of feature
            /// configuration.
            pub mod oneshot {
                #[cfg(feature = "hotpath")]
                pub use crate::lib_on::channels::wrapper::tokio_oneshot_wrap::{Receiver, Sender};
                #[cfg(not(feature = "hotpath"))]
                pub use tokio::sync::oneshot::{Receiver, Sender};
            }
        }
    }

    /// Instrumented crossbeam channel endpoints for `channel!`.
    /// With `hotpath` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw crossbeam types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "crossbeam")]
    pub mod crossbeam_channel {
        #[cfg(feature = "hotpath")]
        pub use crate::lib_on::channels::wrapper::crossbeam_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath"))]
        pub use crossbeam_channel::{Receiver, Sender};
    }

    /// Instrumented flume channel endpoints for `channel!`.
    /// With `hotpath` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw flume types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "flume")]
    pub mod flume {
        #[cfg(feature = "hotpath")]
        pub use crate::lib_on::channels::wrapper::flume_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath"))]
        pub use flume::{Receiver, Sender};
    }

    /// Instrumented async-channel endpoints for `channel!`.
    /// With `hotpath` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw async-channel types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "async-channel")]
    pub mod async_channel {
        #[cfg(feature = "hotpath")]
        pub use crate::lib_on::channels::wrapper::asc_wrap::{Receiver, Sender};
        #[cfg(not(feature = "hotpath"))]
        pub use async_channel::{Receiver, Sender};
    }

    /// Instrumented `futures_channel` endpoints for the default `channel!` mode.
    /// With `hotpath` enabled these are the instrumented wrappers; otherwise
    /// `channel!` is a no-op and the endpoints are the raw futures types, so the
    /// alias resolves the same way regardless of feature configuration.
    #[cfg(feature = "futures")]
    pub mod futures_channel {
        pub mod mpsc {
            #[cfg(feature = "hotpath")]
            pub use crate::lib_on::channels::wrapper::ftc_wrap::{
                Receiver, Sender, TrySendError, UnboundedReceiver, UnboundedSender,
            };
            #[cfg(not(feature = "hotpath"))]
            pub use futures_channel::mpsc::{
                Receiver, Sender, TrySendError, UnboundedReceiver, UnboundedSender,
            };
        }

        pub mod oneshot {
            #[cfg(feature = "hotpath")]
            pub use crate::lib_on::channels::wrapper::ftc_oneshot_wrap::{Receiver, Sender};
            #[cfg(not(feature = "hotpath"))]
            pub use futures_channel::oneshot::{Receiver, Sender};
        }
    }

    /// Instrumented reqwest 0.12 client for `http!(...)`. With `hotpath`
    /// enabled `Client` is reqwest-middleware's `ClientWithMiddleware`;
    /// otherwise `http!` is a no-op and `Client` is the raw `reqwest::Client`,
    /// so the alias resolves the same way regardless of feature configuration.
    ///
    /// `Error` is the error returned by `send()` and `execute()`: reqwest-middleware's
    /// `Error` with profiling on, or `reqwest::Error` with it off. Methods such as
    /// `without_url()` work in both modes. Response methods still return raw
    /// `reqwest::Error`.
    #[cfg(feature = "reqwest-0-12")]
    pub mod reqwest_012 {
        pub use reqwest_012::Response;
        #[cfg(not(feature = "hotpath"))]
        pub use reqwest_012::{Client, Error, RequestBuilder};
        #[cfg(feature = "hotpath")]
        pub use reqwest_middleware_04::{ClientWithMiddleware as Client, Error, RequestBuilder};
    }

    /// Instrumented reqwest 0.13 client for `http!(...)`. With `hotpath`
    /// enabled `Client` is reqwest-middleware's `ClientWithMiddleware`;
    /// otherwise `http!` is a no-op and `Client` is the raw `reqwest::Client`,
    /// so the alias resolves the same way regardless of feature configuration.
    ///
    /// `Error` is the error returned by `send()` and `execute()`: reqwest-middleware's
    /// `Error` with profiling on, or `reqwest::Error` with it off. Methods such as
    /// `without_url()` work in both modes. Response methods still return raw
    /// `reqwest::Error`.
    #[cfg(feature = "reqwest-0-13")]
    pub mod reqwest_013 {
        pub use reqwest::Response;
        #[cfg(not(feature = "hotpath"))]
        pub use reqwest::{Client, Error, RequestBuilder};
        #[cfg(feature = "hotpath")]
        pub use reqwest_middleware_05::{ClientWithMiddleware as Client, Error, RequestBuilder};
    }

    #[cfg(all(feature = "reqwest-0-12", not(feature = "reqwest-0-13")))]
    pub use self::reqwest_012 as reqwest;
    /// Unversioned alias for the newest enabled reqwest generation, so apps
    /// can write `hotpath::wrap::reqwest::Client` regardless of which
    /// `reqwest-0.1x` feature they enable.
    #[cfg(feature = "reqwest-0-13")]
    pub use self::reqwest_013 as reqwest;
}

mod shared;
pub use shared::{env_flag, Format, IntoF64, Section};

#[doc(hidden)]
pub mod dev_logging;
