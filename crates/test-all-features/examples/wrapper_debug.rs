//! Run with:
//!   cargo run -p test-all-features --example wrapper_debug --features hotpath

// Every wrapped type keeps the `Debug` impl of the type it replaces, so a
// `#[derive(Debug)]` struct holding `hotpath::wrap` types compiles with and
// without the `hotpath` feature.

use hotpath::wrap;

#[derive(Debug)]
#[allow(dead_code)]
struct Channels {
    std_tx: wrap::std::sync::mpsc::Sender<u32>,
    std_rx: wrap::std::sync::mpsc::Receiver<u32>,
    std_sync_tx: wrap::std::sync::mpsc::SyncSender<u32>,
    crossbeam_tx: wrap::crossbeam_channel::Sender<u32>,
    crossbeam_rx: wrap::crossbeam_channel::Receiver<u32>,
    flume_tx: wrap::flume::Sender<u32>,
    flume_rx: wrap::flume::Receiver<u32>,
    asc_tx: wrap::async_channel::Sender<u32>,
    asc_rx: wrap::async_channel::Receiver<u32>,
    tokio_tx: wrap::tokio::sync::mpsc::Sender<u32>,
    tokio_rx: wrap::tokio::sync::mpsc::Receiver<u32>,
    tokio_unbounded_tx: wrap::tokio::sync::mpsc::UnboundedSender<u32>,
    ftc_tx: wrap::futures_channel::mpsc::Sender<u32>,
    ftc_rx: wrap::futures_channel::mpsc::Receiver<u32>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct Locks {
    std_mutex: wrap::std::sync::Mutex<u32>,
    std_rw_lock: wrap::std::sync::RwLock<u32>,
    parking_lot_mutex: wrap::parking_lot::Mutex<u32>,
    parking_lot_rw_lock: wrap::parking_lot::RwLock<u32>,
    async_lock_mutex: wrap::async_lock::Mutex<u32>,
    async_lock_rw_lock: wrap::async_lock::RwLock<u32>,
    tokio_mutex: wrap::tokio::sync::Mutex<u32>,
    tokio_rw_lock: wrap::tokio::sync::RwLock<u32>,
}

fn debug<T: std::fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let _guard = hotpath::HotpathGuardBuilder::new("wrapper_debug")
        .format(hotpath::Format::Json)
        .output_path("/dev/null")
        .build();

    let (std_tx, std_rx) = hotpath::channel!(std::sync::mpsc::channel::<u32>());
    let (std_sync_tx, _std_sync_rx) =
        hotpath::channel!(std::sync::mpsc::sync_channel::<u32>(1), capacity = 1);
    let (crossbeam_tx, crossbeam_rx) = hotpath::channel!(crossbeam_channel::unbounded::<u32>());
    let (flume_tx, flume_rx) = hotpath::channel!(flume::unbounded::<u32>());
    let (asc_tx, asc_rx) = hotpath::channel!(async_channel::unbounded::<u32>());
    let (tokio_tx, tokio_rx) = hotpath::channel!(tokio::sync::mpsc::channel::<u32>(1));
    let (tokio_unbounded_tx, _tokio_unbounded_rx) =
        hotpath::channel!(tokio::sync::mpsc::unbounded_channel::<u32>());
    let (ftc_tx, ftc_rx) =
        hotpath::channel!(futures::channel::mpsc::channel::<u32>(1), capacity = 1);
    let channels = Channels {
        std_tx,
        std_rx,
        std_sync_tx,
        crossbeam_tx,
        crossbeam_rx,
        flume_tx,
        flume_rx,
        asc_tx,
        asc_rx,
        tokio_tx,
        tokio_rx,
        tokio_unbounded_tx,
        ftc_tx,
        ftc_rx,
    };
    println!("{channels:?}");

    let locks = Locks {
        std_mutex: hotpath::mutex!(std::sync::Mutex::new(1)),
        std_rw_lock: hotpath::rw_lock!(std::sync::RwLock::new(2)),
        parking_lot_mutex: hotpath::mutex!(parking_lot::Mutex::new(3)),
        parking_lot_rw_lock: hotpath::rw_lock!(parking_lot::RwLock::new(4)),
        async_lock_mutex: hotpath::mutex!(async_lock::Mutex::new(5)),
        async_lock_rw_lock: hotpath::rw_lock!(async_lock::RwLock::new(6)),
        tokio_mutex: hotpath::mutex!(tokio::sync::Mutex::new(7)),
        tokio_rw_lock: hotpath::rw_lock!(tokio::sync::RwLock::new(8)),
    };
    println!("{locks:?}");

    // Guards print the guarded value, exactly like the upstream guards.
    assert_eq!(debug(&locks.std_mutex.lock().unwrap()), "1");
    assert_eq!(debug(&locks.std_rw_lock.read().unwrap()), "2");
    assert_eq!(debug(&locks.std_rw_lock.write().unwrap()), "2");
    assert_eq!(debug(&locks.parking_lot_mutex.lock()), "3");
    assert_eq!(debug(&locks.parking_lot_rw_lock.read()), "4");
    assert_eq!(debug(&locks.parking_lot_rw_lock.write()), "4");
    assert_eq!(debug(&locks.async_lock_mutex.lock().await), "5");
    assert_eq!(debug(&locks.async_lock_rw_lock.read().await), "6");
    assert_eq!(debug(&locks.async_lock_rw_lock.write().await), "6");
    assert_eq!(debug(&locks.tokio_mutex.lock().await), "7");
    assert_eq!(debug(&locks.tokio_rw_lock.read().await), "8");
    assert_eq!(debug(&locks.tokio_rw_lock.write().await), "8");

    // Streams, futures and I/O values keep `Debug` when the wrapped value has it.
    println!(
        "{}",
        debug(&hotpath::stream!(futures::stream::iter([1u32])))
    );
    println!("{}", debug(&hotpath::future!(std::future::ready(1u32))));
    println!(
        "{}",
        debug(&hotpath::io!(std::io::Cursor::new(Vec::<u8>::new())))
    );
}
