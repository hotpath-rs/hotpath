//! Integration tests for `io!` instrumentation. Covers the self-contained
//! cases: sync file I/O (Cursor/BufReader/error readers), async tokio file and
//! duplex I/O, and std/tokio TCP against in-process echo servers. The
//! service-dependent Redis test lives in `io_redis.rs`.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::{JsonIoEntry, JsonIoList};

    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    fn example(name: &str) -> Example {
        Example::new("test-io", name)
    }

    fn io(name: &str) -> JsonIoList {
        example(name)
            .json()
            .report()
            .io
            .expect("No io section in report")
    }

    fn entry<'a>(io: &'a JsonIoList, label: &str) -> &'a JsonIoEntry {
        io.data
            .iter()
            .find(|e| e.name == label)
            .unwrap_or_else(|| panic!("No '{label}' entry in io section"))
    }

    // cargo run -p test-io --example basic_io_sync --features hotpath (json)
    #[test]
    fn test_sync_json_output() {
        let io = io("basic_io_sync");

        let writer = entry(&io, "fixture-write");
        assert_eq!(writer.write.count, 10);
        assert_eq!(writer.write.bytes, 100);
        assert_eq!(writer.flush.count, 1);
        assert_eq!(writer.write.errors, 0);
        assert!(writer.type_name.contains("File"));

        let reader = entry(&io, "fixture-read");
        assert_eq!(reader.instances, 1);
        assert_eq!(reader.read.count, 10);
        assert_eq!(reader.read.bytes, 100);
        assert_eq!(reader.read.sampled_count, 10);
        assert_eq!(reader.read.sampled_bytes, 100);
        assert!(reader.read.total_ns > 0, "Sync reads should be timed");
        assert!(
            reader
                .read
                .bytes_per_sec
                .as_deref()
                .is_some_and(|rate| rate.ends_with('B') && !rate.contains('/')),
            "Timed reads should report bytes_per_sec without a /s suffix"
        );
        assert_eq!(reader.read.errors, 0);
        assert_eq!(reader.write.count, 0);
        assert!(reader.type_name.contains("File"));

        // Delegation through BufReader: all bytes arrive, EOF read included.
        let buffered = entry(&io, "buffered");
        assert_eq!(buffered.read.bytes, 100);
        assert!(buffered.read.count >= 2);

        let failing = entry(&io, "failing");
        assert_eq!(failing.read.errors, 1);
        assert_eq!(failing.read.count, 0);

        // Retryable WouldBlock produces neither an operation nor an error.
        let busy = entry(&io, "busy");
        assert_eq!(busy.read.count, 0);
        assert_eq!(busy.read.errors, 0);

        // Flush failures are recorded as flush errors, not write errors.
        let flush_fail = entry(&io, "flush-fail");
        assert_eq!(flush_fail.write.count, 1);
        assert_eq!(flush_fail.write.errors, 0);
        assert_eq!(flush_fail.flush.count, 0);
        assert_eq!(flush_fail.flush.errors, 1);

        // Two wrappers created at the same call site aggregate into a single
        // entry keyed by source + type.
        let looped = entry(&io, "looped");
        assert_eq!(looped.instances, 2);
        assert_eq!(looped.read.count, 2);
        assert_eq!(looped.read.bytes, 6);
        assert_eq!(
            io.data.iter().filter(|e| e.name.contains("looped")).count(),
            1,
            "Same-site wrappers must not create separate entries"
        );

        // `iter = true` gives every instance its own entry.
        for label in ["itered", "itered-2"] {
            let itered = entry(&io, label);
            assert_eq!(itered.instances, 1);
            assert_eq!(itered.read.count, 1);
            assert_eq!(itered.read.bytes, 2);
        }
    }

    // cargo run -p test-io --example basic_io_async --features hotpath (json)
    #[test]
    fn test_async_json_output() {
        let io = io("basic_io_async");

        let writer = entry(&io, "fixture-write");
        assert_eq!(writer.write.count, 10);
        assert_eq!(writer.write.bytes, 100);
        assert_eq!(writer.flush.count, 1);
        assert_eq!(writer.shutdown.count, 1);
        assert_eq!(writer.write.errors, 0);

        // read_exact may split an operation on a partial fill, so the count is
        // a lower bound while the byte total stays exact.
        let reader = entry(&io, "fixture-read");
        assert!(reader.read.count >= 10);
        assert_eq!(reader.read.bytes, 100);
        assert_eq!(reader.read.errors, 0);

        let client = entry(&io, "duplex-client");
        assert_eq!(client.write.count, 1);
        assert_eq!(client.write.bytes, 5);
        assert_eq!(client.write.errors, 0);
        assert_eq!(client.flush.count, 1);
        assert_eq!(client.shutdown.count, 1);

        // The server delays its response by 200ms; the read spans Pending
        // polls, so its Pending-to-Ready duration must include that wait.
        assert!(client.read.count >= 1);
        assert_eq!(client.read.bytes, 10);
        assert!(
            client.read.total_ns >= 150_000_000,
            "Read duration should include async waiting time, got {}ns",
            client.read.total_ns
        );

        let err_reader = entry(&io, "err-reader");
        assert_eq!(err_reader.read.errors, 1);
        assert_eq!(err_reader.read.count, 0);

        // Retryable Interrupted counts as neither an op nor an error; the
        // retried read is recorded as its own single operation.
        let flaky = entry(&io, "flaky-reader");
        assert_eq!(flaky.read.errors, 0);
        assert_eq!(flaky.read.count, 1);
        assert_eq!(flaky.read.bytes, 5);
    }

    // cargo run -p test-io --example basic_tcp_io --features hotpath (json)
    #[test]
    fn test_tcp_json_output() {
        let io = io("basic_tcp_io");
        let client = entry(&io, "tcp-client");

        assert!(client.type_name.contains("TcpStream"));
        // 8 rounds x 1024 bytes echoed each way; byte totals are exact while
        // the kernel may split individual reads or writes.
        assert!(client.write.count >= 8);
        assert_eq!(client.write.bytes, 8192);
        assert_eq!(client.write.errors, 0);
        assert!(client.read.count >= 8);
        assert_eq!(client.read.bytes, 8192);
        assert_eq!(client.read.errors, 0);
        assert!(client.read.total_ns > 0, "Reads should be timed");
    }

    // cargo run -p test-io --example basic_tokio_tcp_io --features hotpath (json)
    #[test]
    fn test_tokio_tcp_json_output() {
        let io = io("basic_tokio_tcp_io");
        let client = entry(&io, "tokio-tcp-client");

        assert!(client.type_name.contains("TcpStream"));
        // 8 rounds x 1024 bytes echoed each way; byte totals are exact while
        // the kernel may split individual reads or writes.
        assert!(client.write.count >= 8);
        assert_eq!(client.write.bytes, 8192);
        assert_eq!(client.write.errors, 0);
        assert!(client.read.count >= 8);
        assert_eq!(client.read.bytes, 8192);
        assert_eq!(client.read.errors, 0);
        assert!(client.read.total_ns > 0, "Reads should be timed");
        // Explicit AsyncWriteExt::shutdown call at the end of the example.
        assert_eq!(client.shutdown.count, 1);
        assert_eq!(client.shutdown.errors, 0);
    }

    // cargo run -p test-io --example basic_gzip_io --features hotpath (json)
    #[test]
    fn test_gzip_json_output() {
        let io = io("basic_gzip_io");

        // Stacked wrappers: the outer one sees plaintext writes, the inner one the
        // smaller compressed stream. Sizes are cross-checked, not hardcoded.
        let plain_write = entry(&io, "gzip-plaintext-write");
        assert!(plain_write.type_name.contains("GzEncoder"));
        assert!(plain_write.write.count >= 10);
        assert!(plain_write.write.bytes > 10_000);
        assert_eq!(plain_write.write.errors, 0);

        let compressed_write = entry(&io, "gzip-compressed-write");
        assert!(compressed_write.write.bytes > 0);
        assert!(
            compressed_write.write.bytes * 3 <= plain_write.write.bytes,
            "JSON fixture should compress at least 3x, got {} -> {}",
            plain_write.write.bytes,
            compressed_write.write.bytes
        );

        // Decompression yields exactly the plaintext the write side produced.
        let plain_read = entry(&io, "gzip-plaintext-read");
        assert!(plain_read.type_name.contains("GzDecoder"));
        assert_eq!(plain_read.read.bytes, plain_write.write.bytes);
        assert_eq!(plain_read.read.errors, 0);

        // The read-back side consumes exactly the bytes the write side produced.
        let compressed_read = entry(&io, "gzip-compressed-read");
        assert_eq!(compressed_read.read.bytes, compressed_write.write.bytes);
    }

    // cargo run -p test-io --example basic_zstd_io --features hotpath (json)
    #[test]
    fn test_zstd_json_output() {
        let io = io("basic_zstd_io");

        // Same layer cross-checks as the gzip test, over the zstd codec.
        let plain_write = entry(&io, "zstd-plaintext-write");
        assert!(plain_write.type_name.contains("Encoder"));
        assert!(plain_write.write.count >= 10);
        assert!(plain_write.write.bytes > 10_000);
        assert_eq!(plain_write.write.errors, 0);

        let compressed_write = entry(&io, "zstd-compressed-write");
        assert!(compressed_write.write.bytes > 0);
        assert!(
            compressed_write.write.bytes * 3 <= plain_write.write.bytes,
            "JSON fixture should compress at least 3x, got {} -> {}",
            plain_write.write.bytes,
            compressed_write.write.bytes
        );

        let plain_read = entry(&io, "zstd-plaintext-read");
        assert!(plain_read.type_name.contains("Decoder"));
        assert_eq!(plain_read.read.bytes, plain_write.write.bytes);
        assert_eq!(plain_read.read.errors, 0);

        let compressed_read = entry(&io, "zstd-compressed-read");
        assert_eq!(compressed_read.read.bytes, compressed_write.write.bytes);
    }

    // cargo run -p test-io --example basic_io_sync --features hotpath
    #[test]
    fn test_table_output() {
        let stdout = example("basic_io_sync").stdout();

        assert_contains_all(
            &stdout,
            &[
                "Sync io example completed!",
                "Byte-level I/O statistics",
                "fixture-write",
                "fixture-read",
                "Reads",
                "Writes",
                "Bytes",
                "Flushes",
                "Errors",
            ],
        );
    }

    // Two `io!` invocations on one physical line wrapping the same concrete
    // type must register distinct entries: the registration key includes the
    // column, so the second call site does not reuse the first one's entry. The
    // displayed source keeps the plain `file:line` form.
    //
    // cargo run -p test-io --example same_line_io --features hotpath (json)
    #[test]
    fn test_same_line_call_sites_stay_distinct() {
        let io = io("same_line_io");

        let a = entry(&io, "same-line-a");
        let b = entry(&io, "same-line-b");

        assert_ne!(
            a.location, b.location,
            "same-line call sites must not share an entry"
        );
        assert_eq!(a.read.bytes, 30, "bytes must not merge across call sites");
        assert_eq!(b.read.bytes, 70, "bytes must not merge across call sites");
        assert_eq!(a.instances, 1);
        assert_eq!(b.instances, 1);

        // The displayed source is identical for both (same file:line) and the
        // path contains no ':', so exactly one colon proves the column stays
        // out of the display string.
        assert_eq!(a.source, b.source, "one physical line renders one source");
        assert_eq!(
            a.source.matches(':').count(),
            1,
            "source must stay file:line"
        );
    }
}
