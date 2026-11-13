use std::sync::OnceLock;

use crate::{FunctionSink, FunctionSinkAlreadySet, ScopeKind};

/// Set at most once and never cleared, so a scope that saw a sink on `enter`
/// is guaranteed to see the same sink on `exit`. Reading it is a single
/// acquire load, which keeps the producer path free of locks and RMW atomics.
static SINK: OnceLock<&'static dyn FunctionSink> = OnceLock::new();

/// Installs `sink` for the rest of the process. Scopes entered before this
/// call are not reported, including their exits.
///
/// # Errors
///
/// [`FunctionSinkAlreadySet`] if a sink was installed earlier.
pub fn set_function_sink(sink: impl FunctionSink) -> Result<(), FunctionSinkAlreadySet> {
    let mut installed = false;
    SINK.get_or_init(|| {
        installed = true;
        Box::leak(Box::new(sink))
    });
    if installed {
        Ok(())
    } else {
        Err(FunctionSinkAlreadySet)
    }
}

/// The sink side of one measurement guard: present only when a sink was
/// installed when the guard was built and the scope is not skipped.
#[derive(Debug)]
pub(crate) struct SinkScope {
    kind: ScopeKind,
    token: u64,
}

impl SinkScope {
    #[inline]
    pub(crate) fn enter(name: &'static str, kind: ScopeKind, skipped: bool) -> Option<Self> {
        if skipped {
            return None;
        }
        let sink = SINK.get()?;
        let token = sink.enter(name, kind);
        Some(Self { kind, token })
    }

    #[inline]
    pub(crate) fn exit(self, name: &'static str) {
        if let Some(sink) = SINK.get() {
            sink.exit(name, self.kind, self.token);
        }
    }
}

impl ScopeKind {
    /// Only caller-scoped guards wrap a sync function body, which the compiler
    /// guarantees is await-free. Every other guard (async bodies,
    /// `measure_block!`, the `#[main]` wrapper) may outlive a suspension point.
    #[inline]
    pub(crate) fn of_guard(caller_scoped: bool) -> Self {
        if caller_scoped {
            Self::Sync
        } else {
            Self::Async
        }
    }
}
