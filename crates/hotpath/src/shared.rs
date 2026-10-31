use std::str::FromStr;

/// `user_metadata` keys the Criterion adapter records.
#[cfg(any(
    feature = "hotpath",
    feature = "criterion-0-5",
    feature = "criterion-0-8"
))]
pub(crate) const CRITERION_BENCHMARK_KEY: &str = "criterion_benchmark";
#[cfg(any(
    feature = "hotpath",
    feature = "criterion-0-5",
    feature = "criterion-0-8"
))]
pub(crate) const CRITERION_ITERATIONS_KEY: &str = "criterion_iterations";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    FunctionsTiming,
    FunctionsAlloc,
    FunctionsCpu,
    FunctionsIcount,
    Channels,
    Streams,
    Futures,
    RwLocks,
    Mutexes,
    Sql,
    Http,
    Server,
    Io,
    Threads,
    Debug,
}

impl Section {
    pub fn all() -> Vec<Section> {
        vec![
            Section::FunctionsTiming,
            Section::FunctionsAlloc,
            Section::FunctionsCpu,
            Section::FunctionsIcount,
            Section::Channels,
            Section::Streams,
            Section::Futures,
            Section::RwLocks,
            Section::Mutexes,
            Section::Sql,
            Section::Http,
            Section::Server,
            Section::Io,
            Section::Threads,
            Section::Debug,
        ]
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Section::FunctionsTiming => "timing",
            Section::FunctionsAlloc => "alloc",
            Section::FunctionsCpu => "cpu",
            Section::FunctionsIcount => "icount",
            Section::Channels => "channels",
            Section::Streams => "streams",
            Section::Futures => "futures",
            Section::RwLocks => "rw_locks",
            Section::Mutexes => "mutexes",
            Section::Sql => "sql",
            Section::Http => "http",
            Section::Server => "server",
            Section::Io => "io",
            Section::Threads => "threads",
            Section::Debug => "debug",
        }
    }

    #[cfg(feature = "hotpath")]
    pub(crate) fn from_name(s: &str) -> Option<Section> {
        match s.trim() {
            "functions-timing" => Some(Section::FunctionsTiming),
            "functions-alloc" => Some(Section::FunctionsAlloc),
            "functions-cpu" => Some(Section::FunctionsCpu),
            "functions-icount" => Some(Section::FunctionsIcount),
            "channels" => Some(Section::Channels),
            "streams" => Some(Section::Streams),
            "futures" => Some(Section::Futures),
            "rw_locks" => Some(Section::RwLocks),
            "mutexes" => Some(Section::Mutexes),
            "sql" => Some(Section::Sql),
            "http" => Some(Section::Http),
            "server" => Some(Section::Server),
            "io" => Some(Section::Io),
            "threads" => Some(Section::Threads),
            "debug" => Some(Section::Debug),
            _ => None,
        }
    }
}

/// How the set of report sections is determined.
#[cfg(feature = "hotpath")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SectionsMode {
    /// Base sections (functions, threads) plus every data-driven section
    /// (channels, streams, futures, rw_locks, mutexes, sql, http, server, io, debug)
    /// that has data at shutdown. `include` forces sections in, `exclude`
    /// hides them.
    Auto {
        include: Vec<Section>,
        exclude: Vec<Section>,
    },
    /// Exactly these sections, nothing else.
    Explicit(Vec<Section>),
}

#[cfg(feature = "hotpath")]
impl Default for SectionsMode {
    fn default() -> Self {
        SectionsMode::Auto {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

#[cfg(feature = "hotpath")]
impl SectionsMode {
    /// Parses a report spec: `all`, `auto`, section names, and `-section`
    /// exclusions, comma-separated. A spec containing `auto` or any `-section`
    /// token resolves to [`SectionsMode::Auto`]; a plain list of names is
    /// [`SectionsMode::Explicit`]; `all` is every section minus exclusions.
    pub(crate) fn parse(spec: &str) -> SectionsMode {
        let mut auto = false;
        let mut all = false;
        let mut include = Vec::new();
        let mut exclude = Vec::new();

        for part in spec.split(',') {
            let part = part.trim();
            match part {
                "all" => all = true,
                "auto" => auto = true,
                _ => {
                    let (target, name) = match part.strip_prefix('-') {
                        Some(rest) => (&mut exclude, rest.trim()),
                        None => (&mut include, part),
                    };
                    if let Some(s) = Section::from_name(name) {
                        if !target.contains(&s) {
                            target.push(s);
                        }
                    } else {
                        eprintln!("[hotpath] Unknown report section: '{}'", part);
                    }
                }
            }
        }

        if all {
            SectionsMode::Explicit(
                Section::all()
                    .into_iter()
                    .filter(|s| !exclude.contains(s))
                    .collect(),
            )
        } else if auto || !exclude.is_empty() {
            SectionsMode::Auto { include, exclude }
        } else {
            SectionsMode::Explicit(include)
        }
    }

    pub(crate) fn from_env() -> Option<SectionsMode> {
        std::env::var("HOTPATH_REPORT")
            .ok()
            .map(|val| SectionsMode::parse(&val))
    }

    /// Whether the section is requested by name (explicit list or a forced
    /// auto include). Auto mode does not imply it - used for build-time
    /// side effects that should stay lazy under auto.
    pub(crate) fn explicitly_contains(&self, section: Section) -> bool {
        match self {
            SectionsMode::Explicit(list) => list.contains(&section),
            SectionsMode::Auto { include, .. } => include.contains(&section),
        }
    }

    /// Whether the section is part of the report set decidable before any
    /// data exists: listed explicitly, or in auto mode and not excluded.
    #[cfg(feature = "hotpath-cpu")]
    pub(crate) fn contains_or_auto(&self, section: Section) -> bool {
        match self {
            SectionsMode::Explicit(list) => list.contains(&section),
            SectionsMode::Auto { include, exclude } => {
                include.contains(&section) || !exclude.contains(&section)
            }
        }
    }
}

/// Output format for the profiling report printed when the program exits.
///
/// # Variants
///
/// * `Table` - Human-readable table format (default)
/// * `Json` - JSON format
/// * `JsonPretty` - Pretty-printed JSON format
/// * `None` - Suppress all profiling output (metrics server and MCP server still function)
///
/// # Parsing
///
/// Can be parsed from strings via `HOTPATH_OUTPUT_FORMAT` environment variable:
/// - `"table"` → `Format::Table`
/// - `"json"` → `Format::Json`
/// - `"json-pretty"` → `Format::JsonPretty`
/// - `"none"` → `Format::None`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Table,
    Json,
    JsonPretty,
    None,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" => Ok(Format::Table),
            "json" => Ok(Format::Json),
            "json-pretty" | "jsonpretty" => Ok(Format::JsonPretty),
            "none" => Ok(Format::None),
            _ => Err(format!(
                "unknown format '{}', expected: table, json, json-pretty, none",
                s
            )),
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl Format {
    /// Returns the format from `HOTPATH_OUTPUT_FORMAT` env var, or default if not set.
    /// Panics if the env var contains an invalid value.
    #[cfg(feature = "hotpath")]
    pub(crate) fn from_env() -> Self {
        match std::env::var("HOTPATH_OUTPUT_FORMAT") {
            Ok(v) => v
                .parse()
                .unwrap_or_else(|e| panic!("HOTPATH_OUTPUT_FORMAT: {}", e)),
            Err(_) => Format::default(),
        }
    }
}

pub trait IntoF64 {
    fn into_f64(self) -> f64;
}

impl IntoF64 for f64 {
    fn into_f64(self) -> f64 {
        self
    }
}

impl IntoF64 for f32 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i8 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i16 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i32 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for i64 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for u8 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for u16 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for u32 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for u64 {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for isize {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

impl IntoF64 for usize {
    fn into_f64(self) -> f64 {
        self as f64
    }
}

#[doc(hidden)]
pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

#[cfg(feature = "hotpath")]
pub(crate) fn resolve_timeout_duration(
    default_duration: std::time::Duration,
    env_var: &str,
) -> Option<std::time::Duration> {
    let effective_duration = std::env::var(env_var)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(default_duration);

    if effective_duration.is_zero() {
        None
    } else {
        Some(effective_duration)
    }
}
