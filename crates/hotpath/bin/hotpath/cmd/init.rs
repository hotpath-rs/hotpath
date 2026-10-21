use std::io::ErrorKind;
use std::process::Command;

const SKILL_URL_BRANCH_TEMPLATE: &str =
    "https://raw.githubusercontent.com/hotpath-rs/hotpath/init-v{minor}/skills/{skill}/SKILL.md";
const SKILL_URL_TAG_TEMPLATE: &str =
    "https://raw.githubusercontent.com/hotpath-rs/hotpath/v{version}/skills/{skill}/SKILL.md";

/// What an agent session sets up: each one is a skill of `skills/` and the
/// prompt that starts the session.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Setup {
    /// `hotpath init`: profiling in the current repo.
    Profiling,
    /// `hotpath init-cloud`: the hotpath Cloud CI integration.
    Ci,
    /// `hotpath init-cloud --forks`: the CI integration with the relay workflow
    /// that also covers pull requests from forks.
    CiForks,
    /// `hotpath init-grafana`: a Grafana dashboard on the Prometheus metrics.
    Grafana,
}

impl Setup {
    fn skill(&self) -> &'static str {
        match self {
            Self::Profiling => "hotpath_init",
            Self::Ci => "hotpath_init_ci",
            Self::CiForks => "hotpath_init_ci_forks",
            Self::Grafana => "hotpath_init_grafana",
        }
    }

    fn kickoff_prompt(&self) -> &'static str {
        match self {
            Self::Profiling => "Set up hotpath profiling in this repo.",
            Self::Ci => "Set up the hotpath Cloud CI integration in this repo.",
            Self::CiForks => {
                "Set up the hotpath Cloud CI integration in this repo, covering pull requests from forks."
            }
            Self::Grafana => "Set up a hotpath Grafana dashboard for this repo.",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Agent {
    Claude,
    Codex,
    OpenCode,
}

impl Agent {
    pub fn from_arg(arg: &str) -> Result<Self, String> {
        match arg {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            other => Err(format!(
                "Unknown agent '{other}'. Supported agents: claude, codex, opencode"
            )),
        }
    }

    fn bin(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

pub(crate) fn run(setup: Setup, agent: Agent) -> Result<(), String> {
    let branch_url = branch_skill_url(setup.skill());
    println!("Downloading setup instructions from {branch_url}");
    let skill = match download_skill(&branch_url) {
        Ok(skill) => skill,
        Err(branch_err) => {
            let tag_url = tag_skill_url(setup.skill());
            println!("Branch download failed, retrying from {tag_url}");
            download_skill(&tag_url).map_err(|tag_err| format!("{branch_err}\n{tag_err}"))?
        }
    };
    let instructions = strip_frontmatter(&skill);

    println!(
        "Starting {} session with hotpath setup instructions...",
        agent.bin()
    );

    let mut command = Command::new(agent.bin());
    match agent {
        Agent::Claude => {
            command
                .arg("--append-system-prompt")
                .arg(instructions)
                .arg(setup.kickoff_prompt());
        }
        Agent::Codex => {
            command.arg(inline_prompt(setup, instructions));
        }
        Agent::OpenCode => {
            command
                .arg("--prompt")
                .arg(inline_prompt(setup, instructions));
        }
    }

    let status = command.status().map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            format!("'{}' not found. Is it installed and on PATH?", agent.bin())
        } else {
            format!("Failed to launch '{}': {e}", agent.bin())
        }
    })?;

    if !status.success() {
        return Err(format!(
            "{} session exited with status: {status}",
            agent.bin()
        ));
    }

    Ok(())
}

fn inline_prompt(setup: Setup, instructions: &str) -> String {
    format!(
        "{instructions}\n\n{} Follow the instructions above.",
        setup.kickoff_prompt()
    )
}

fn branch_skill_url(skill: &str) -> String {
    skill_url(
        SKILL_URL_BRANCH_TEMPLATE,
        skill,
        minor_version(env!("CARGO_PKG_VERSION")),
    )
}

fn tag_skill_url(skill: &str) -> String {
    skill_url(SKILL_URL_TAG_TEMPLATE, skill, env!("CARGO_PKG_VERSION"))
}

fn skill_url(template: &str, skill: &str, version: &str) -> String {
    template
        .replace("{skill}", skill)
        .replace("{minor}", version)
        .replace("{version}", version)
}

fn minor_version(version: &str) -> &str {
    version.rsplit_once('.').map_or(version, |(minor, _)| minor)
}

fn download_skill(url: &str) -> Result<String, String> {
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", url])
        .output()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                "'curl' not found. Install curl to use the 'hotpath init' commands.".to_string()
            } else {
                format!("Failed to run curl: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to download setup instructions from {url}: {}",
            stderr.trim()
        ));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("Setup instructions are not valid UTF-8: {e}"))
}

fn strip_frontmatter(skill: &str) -> &str {
    let Some(rest) = skill.strip_prefix("---\n") else {
        return skill;
    };
    match rest.find("\n---\n") {
        Some(end) => rest[end + "\n---\n".len()..].trim_start(),
        None => skill,
    }
}

#[cfg(test)]
mod tests {
    use crate::cmd::init::{
        minor_version, skill_url, strip_frontmatter, Setup, SKILL_URL_BRANCH_TEMPLATE,
    };

    #[test]
    fn derives_minor_version() {
        assert_eq!(minor_version("0.21.4"), "0.21");
        assert_eq!(minor_version("1.0.0"), "1.0");
        assert_eq!(minor_version("0.21"), "0");
    }

    #[test]
    fn branch_url_uses_minor_version() {
        let url = skill_url(
            SKILL_URL_BRANCH_TEMPLATE,
            Setup::Ci.skill(),
            minor_version("0.21.4"),
        );
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/hotpath-rs/hotpath/init-v0.21/skills/hotpath_init_ci/SKILL.md"
        );
    }

    #[test]
    fn strips_yaml_frontmatter() {
        let skill = "---\nname: hotpath_init\ndescription: Configure hotpath.\n---\n\n# Initialize\n\nBody text.";
        assert_eq!(strip_frontmatter(skill), "# Initialize\n\nBody text.");
    }

    #[test]
    fn returns_input_without_frontmatter() {
        let skill = "# Initialize\n\nBody text.";
        assert_eq!(strip_frontmatter(skill), skill);
    }

    #[test]
    fn returns_input_with_unterminated_frontmatter() {
        let skill = "---\nname: hotpath_init\nno closing marker";
        assert_eq!(strip_frontmatter(skill), skill);
    }
}
