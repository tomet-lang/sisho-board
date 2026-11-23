//! Core ecosystem tools (`tomet`, `twrit`, `catasto`) and plans inspected by `sisho-board`.

use std::fmt;
use std::path::Path;

use crate::runner::Step;

/// A semantic version `(major, minor, patch)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse a version from a `<name> <major>.<minor>.<patch>` line.
    pub fn parse_from_line(line: &str) -> Option<Self> {
        for token in line.split_whitespace() {
            let token = token.strip_prefix('v').unwrap_or(token);
            let head = token.split(['-', '+']).next()?;
            let mut parts = head.split('.');
            if let (Some(maj), Some(min), Some(pat), None) = (
                parts.next().and_then(|s| s.parse().ok()),
                parts.next().and_then(|s| s.parse().ok()),
                parts.next().and_then(|s| s.parse().ok()),
                parts.next(),
            ) {
                return Some(Self::new(maj, min, pat));
            }
        }
        None
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// How a tool's reported version compares against its minimum required version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Fine(Version),
    TooOld { found: Version, needs: Version },
    Unreported { needs: Version },
}

impl Verdict {
    pub fn is_fine(&self) -> bool {
        matches!(self, Self::Fine(_))
    }
}

/// A core tool in the Tomet ecosystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Tomet,
    Twrit,
    Catasto,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Tomet, Tool::Twrit, Tool::Catasto];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Tomet => "tomet",
            Tool::Twrit => "twrit",
            Tool::Catasto => "catasto",
        }
    }

    pub fn version_step(self) -> Step {
        Step::new(format!("{} --version", self.name()), self.name()).arg("--version")
    }

    pub const fn min_version(self) -> Version {
        match self {
            Tool::Tomet => Version::new(0, 5, 0),
            Tool::Twrit => Version::new(0, 2, 0),
            Tool::Catasto => Version::new(0, 1, 0),
        }
    }

    pub fn judge(self, reported: Option<&str>) -> Verdict {
        let needs = self.min_version();
        let Some(found) = reported.and_then(Version::parse_from_line) else {
            return Verdict::Unreported { needs };
        };
        if found >= needs {
            Verdict::Fine(found)
        } else {
            Verdict::TooOld { found, needs }
        }
    }
}

/// The steps of `sisho check` for `path`.
pub fn check_steps(path: &Path) -> Vec<Step> {
    vec![
        Step::new("tomet check", Tool::Tomet.name())
            .arg("check")
            .arg(path),
        Step::new("tomet format --check", Tool::Tomet.name())
            .args(["format", "--check"])
            .arg(path),
        Step::new("tomet export --check", Tool::Tomet.name())
            .args(["export", "--check"])
            .arg(path),
        Step::new("twrit check", Tool::Twrit.name())
            .arg("check")
            .arg(path),
        Step::new("catasto check", Tool::Catasto.name())
            .arg("check")
            .arg(path),
    ]
}

/// The steps of `sisho fix` for `path`.
pub fn fix_steps(path: &Path) -> Vec<Step> {
    vec![
        Step::new("twrit check --fix", Tool::Twrit.name())
            .args(["check", "--fix"])
            .arg(path),
        Step::new("tomet export", Tool::Tomet.name())
            .arg("export")
            .arg(path),
        Step::new("tomet format -i", Tool::Tomet.name())
            .args(["format", "-i"])
            .arg(path),
    ]
}

/// `tomet check-links --json <path>`.
pub fn check_links_step(path: &Path) -> Step {
    Step::new("tomet check-links --json", Tool::Tomet.name())
        .args(["check-links", "--json"])
        .arg(path)
}
