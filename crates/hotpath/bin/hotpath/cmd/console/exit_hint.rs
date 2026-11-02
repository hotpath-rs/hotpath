//! GitHub star hint printed once the TUI has exited and the terminal is restored.
//!
//! Rate limited by a tiny per-user state file (last impression time and total
//! impression count). Persistence is best-effort: a missing or corrupted file
//! counts as "never shown", and if the updated state cannot be written the hint
//! is skipped, so an unwritable state dir can never make it show repeatedly.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MIN_SESSION: Duration = Duration::from_secs(120);
/// Percentage of otherwise eligible sessions that show the hint.
const CHANCE_PERCENT: u64 = 20;
const COOLDOWN: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_IMPRESSIONS: u32 = 3;

const MESSAGE: &str = concat!(
    "Enjoying hotpath? A GitHub star helps others discover it:\n",
    env!("CARGO_PKG_REPOSITORY")
);

const STATE_FILE: &str = "tui_exit_hint";

static HINTS_DISABLED: LazyLock<bool> =
    LazyLock::new(|| hotpath::env_flag("HOTPATH_DISABLE_TUI_HINTS"));

/// `CI` is set (to anything but a false value) by GitHub Actions, GitLab CI,
/// CircleCI, Travis, Buildkite and most other providers.
static RUNNING_IN_CI: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("CI").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "false" | "FALSE"))
});

/// What the finished TUI session looked like.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Session {
    pub(crate) duration: Duration,
    /// `App::run` returned `Ok`. A panic never reaches the hint at all.
    pub(crate) ended_cleanly: bool,
    /// At least one successful fetch from the metrics server.
    pub(crate) received_data: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct State {
    /// Unix seconds of the last impression, `None` if never shown.
    last_shown: Option<u64>,
    impressions: u32,
}

impl State {
    fn parse(contents: &str) -> Option<Self> {
        let mut last_shown = None;
        let mut impressions = None;
        for line in contents.lines() {
            match line.split_once('=')? {
                ("last_shown", v) => last_shown = Some(v.trim().parse().ok()?),
                ("impressions", v) => impressions = Some(v.trim().parse().ok()?),
                _ => return None,
            }
        }
        let impressions = impressions?;
        // A recorded impression must come with its timestamp.
        if impressions > 0 && last_shown.is_none() {
            return None;
        }
        Some(Self {
            last_shown,
            impressions,
        })
    }

    fn serialize(&self) -> String {
        let mut out = String::new();
        if let Some(ts) = self.last_shown {
            out.push_str(&format!("last_shown={ts}\n"));
        }
        out.push_str(&format!("impressions={}\n", self.impressions));
        out
    }

    fn allows(&self, now: u64) -> bool {
        self.impressions < MAX_IMPRESSIONS
            && self
                .last_shown
                .is_none_or(|ts| now.saturating_sub(ts) >= COOLDOWN.as_secs())
    }

    fn after_impression(&self, now: u64) -> Self {
        Self {
            last_shown: Some(now),
            impressions: self.impressions.saturating_add(1),
        }
    }
}

/// Environment facts outside the session, split out so tests can fix them.
#[derive(Debug, Clone, Copy)]
struct Env {
    disabled: bool,
    interactive: bool,
    ci: bool,
}

/// Returns the state to persist if the hint should be shown. `roll` yields a
/// value in `0..100` and is called at most once, only after every
/// deterministic check passed.
fn decide(
    session: Session,
    env: Env,
    state: State,
    now: u64,
    roll: impl FnOnce() -> u64,
) -> Option<State> {
    let eligible = !env.disabled
        && env.interactive
        && !env.ci
        && session.ended_cleanly
        && session.received_data
        && session.duration >= MIN_SESSION
        && state.allows(now);
    (eligible && roll() < CHANCE_PERCENT).then(|| state.after_impression(now))
}

/// Called exactly once, after `ratatui::restore()`. Never fails.
pub(crate) fn maybe_print(session: Session) {
    let env = Env {
        disabled: *HINTS_DISABLED,
        interactive: std::io::stdout().is_terminal(),
        ci: *RUNNING_IN_CI,
    };
    let Some(path) = state_dir().map(|dir| dir.join(STATE_FILE)) else {
        return;
    };
    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };
    let state = load(&path);
    let Some(next) = decide(session, env, state, now.as_secs(), random_percent) else {
        return;
    };
    // Persist first: if the impression cannot be recorded, stay silent rather
    // than risk ignoring the cooldown and the impression cap.
    if store(&path, &next).is_ok() {
        println!("{MESSAGE}");
    }
}

fn load(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| State::parse(&contents))
        .unwrap_or_default()
}

/// Writes through a temp file and a rename, so an interrupted write never
/// leaves a truncated state file behind.
fn store(path: &Path, state: &State) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = std::fs::File::create(&temp)
        .and_then(|mut file| file.write_all(state.serialize().as_bytes()))
        .and_then(|()| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// `~/.hotpath-rs`, on every platform.
fn state_dir() -> Option<PathBuf> {
    std::env::home_dir()
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".hotpath-rs"))
}

/// Uniform-enough value in `0..100` without a `rand` dependency: std seeds
/// `RandomState` with per-process random keys, so hashing nothing yields a
/// fresh random `u64` on every run.
fn random_percent() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        % 100
}

#[cfg(test)]
mod tests {
    use crate::cmd::console::exit_hint::{
        decide, load, store, Env, Session, State, CHANCE_PERCENT, COOLDOWN, MAX_IMPRESSIONS,
        MIN_SESSION,
    };
    use std::cell::Cell;
    use std::path::PathBuf;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 24 * 60 * 60;
    const HIT: u64 = 0;
    const MISS: u64 = 99;

    fn session() -> Session {
        Session {
            duration: MIN_SESSION,
            ended_cleanly: true,
            received_data: true,
        }
    }

    fn env() -> Env {
        Env {
            disabled: false,
            interactive: true,
            ci: false,
        }
    }

    fn shown(at: u64, impressions: u32) -> State {
        State {
            last_shown: Some(at),
            impressions,
        }
    }

    /// Unique scratch dir under the system temp dir, removed on drop.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("hotpath-exit-hint-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn eligible_session_follows_the_random_roll() {
        assert_eq!(
            decide(session(), env(), State::default(), NOW, || HIT),
            Some(shown(NOW, 1))
        );
        assert_eq!(
            decide(session(), env(), State::default(), NOW, || CHANCE_PERCENT
                - 1),
            Some(shown(NOW, 1))
        );
        assert_eq!(
            decide(session(), env(), State::default(), NOW, || CHANCE_PERCENT),
            None
        );
        assert_eq!(
            decide(session(), env(), State::default(), NOW, || MISS),
            None
        );
    }

    #[test]
    fn ineligible_sessions_never_roll() {
        let short = Session {
            duration: MIN_SESSION - std::time::Duration::from_millis(1),
            ..session()
        };
        let crashed = Session {
            ended_cleanly: false,
            ..session()
        };
        let no_data = Session {
            received_data: false,
            ..session()
        };
        let piped = Env {
            interactive: false,
            ..env()
        };
        let ci = Env { ci: true, ..env() };
        let disabled = Env {
            disabled: true,
            ..env()
        };
        let cases = [
            (short, env()),
            (crashed, env()),
            (no_data, env()),
            (session(), piped),
            (session(), ci),
            (session(), disabled),
        ];
        for (session, env) in cases {
            let rolled = Cell::new(false);
            let decision = decide(session, env, State::default(), NOW, || {
                rolled.set(true);
                HIT
            });
            assert_eq!(decision, None, "{session:?} {env:?}");
            assert!(!rolled.get(), "rolled for {session:?} {env:?}");
        }
    }

    #[test]
    fn cooldown_is_thirty_days() {
        assert_eq!(COOLDOWN.as_secs(), 30 * DAY);
        let last = NOW - 30 * DAY;
        assert_eq!(
            decide(session(), env(), shown(last + 1, 1), NOW, || HIT),
            None
        );
        assert_eq!(
            decide(session(), env(), shown(last, 1), NOW, || HIT),
            Some(shown(NOW, 2))
        );
        // A timestamp from the future (clock moved back) keeps the cooldown active.
        assert_eq!(
            decide(session(), env(), shown(NOW + DAY, 1), NOW, || HIT),
            None
        );
    }

    #[test]
    fn stops_after_three_impressions() {
        let long_ago = NOW - 365 * DAY;
        assert_eq!(
            decide(
                session(),
                env(),
                shown(long_ago, MAX_IMPRESSIONS - 1),
                NOW,
                || HIT
            ),
            Some(shown(NOW, MAX_IMPRESSIONS))
        );
        assert_eq!(
            decide(
                session(),
                env(),
                shown(long_ago, MAX_IMPRESSIONS),
                NOW,
                || HIT
            ),
            None
        );
    }

    #[test]
    fn shown_at_most_once_per_session() {
        let first = decide(session(), env(), State::default(), NOW, || HIT)
            .expect("first evaluation shows the hint");
        // Re-evaluating with the recorded state at the same instant is blocked.
        assert_eq!(decide(session(), env(), first, NOW, || HIT), None);
    }

    #[test]
    fn missing_state_file_is_a_first_run() {
        let dir = ScratchDir::new("missing");
        assert_eq!(load(&dir.0.join("state")), State::default());
    }

    #[test]
    fn corrupted_state_is_a_first_run() {
        let dir = ScratchDir::new("corrupted");
        let path = dir.0.join("state");
        std::fs::create_dir_all(&dir.0).unwrap();
        for contents in [
            "",
            "garbage",
            "impressions=abc\n",
            "impressions=-1\n",
            "last_shown=10\n",
            "impressions=2\n",
            "impressions=1\nunknown=1\n",
            "\u{0}\u{1}binary",
        ] {
            std::fs::write(&path, contents).unwrap();
            assert_eq!(load(&path), State::default(), "{contents:?}");
        }
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(load(&path), State::default());
    }

    #[test]
    fn state_round_trips_through_the_file() {
        let dir = ScratchDir::new("roundtrip");
        let path = dir.0.join("nested").join("state");
        store(&path, &shown(NOW, 2)).unwrap();
        assert_eq!(load(&path), shown(NOW, 2));
        store(&path, &shown(NOW + DAY, 3)).unwrap();
        assert_eq!(load(&path), shown(NOW + DAY, 3));
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "temp file left behind");
    }

    #[test]
    fn unwritable_state_dir_fails_without_panicking() {
        let dir = ScratchDir::new("unwritable");
        std::fs::create_dir_all(&dir.0).unwrap();
        // A regular file where the state dir should be makes every write fail.
        let blocker = dir.0.join("blocker");
        std::fs::write(&blocker, "").unwrap();
        assert!(store(&blocker.join("state"), &shown(NOW, 1)).is_err());
        assert_eq!(load(&blocker.join("state")), State::default());
    }
}
