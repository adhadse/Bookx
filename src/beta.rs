//! Beta disclosure: which release channel a version belongs to (from the pre-release part of the
//! semantic version: `0.3.0-beta.1` is a beta) and when the one-time notice in the library is due.
//!
//! The channel is also computed at compile time by `build.rs` (`config::CHANNEL`); a test keeps the two
//! rules the same. `BOOKX_TEST_VERSION` replaces the version in the UI tests only.

/// The kind of release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Alpha,
    Beta,
    Rc,
    /// Any other pre-release part (`-dev`, `-pre.2`).
    Prerelease,
}

impl Channel {
    /// The word the build embeds.
    #[cfg(test)]
    pub fn key(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Alpha => "alpha",
            Channel::Beta => "beta",
            Channel::Rc => "rc",
            Channel::Prerelease => "prerelease",
        }
    }

    #[cfg(test)]
    pub fn from_key(k: &str) -> Channel {
        match k {
            "alpha" => Channel::Alpha,
            "beta" => Channel::Beta,
            "rc" => Channel::Rc,
            "prerelease" => Channel::Prerelease,
            _ => Channel::Stable,
        }
    }

    pub fn is_prerelease(self) -> bool {
        self != Channel::Stable
    }

    /// The tag shown in About ("Beta"), untranslated.
    pub fn tag(self) -> &'static str {
        match self {
            Channel::Stable => "",
            Channel::Alpha => crate::i18n::N_("Alpha"),
            Channel::Beta => crate::i18n::N_("Beta"),
            Channel::Rc => crate::i18n::N_("Release Candidate"),
            Channel::Prerelease => crate::i18n::N_("Pre-release"),
        }
    }
}

/// The channel of a semantic version string (`1.2.3`, `1.2.3-beta.1`, `1.2.3-rc.2+build5`).
pub fn channel_of(version: &str) -> Channel {
    let v = version.split('+').next().unwrap_or("");
    let Some((_, pre)) = v.split_once('-') else {
        return Channel::Stable;
    };
    let p = pre.to_ascii_lowercase();
    if p.starts_with("alpha") {
        Channel::Alpha
    } else if p.starts_with("beta") {
        Channel::Beta
    } else if p.starts_with("rc") {
        Channel::Rc
    } else if p.is_empty() {
        Channel::Stable
    } else {
        Channel::Prerelease
    }
}

/// The version the app shows: the real one, except under test (`BOOKX_TEST_VERSION`).
pub fn version() -> String {
    version_from(std::env::var("BOOKX_TEST_VERSION").ok())
}

fn version_from(test_version: Option<String>) -> String {
    match test_version {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => crate::config::VERSION.to_string(),
    }
}

/// The channel of [`version`].
pub fn channel() -> Channel {
    channel_of(&version())
}

/// Whether the first-run notice of this version is due: a pre-release the user has not seen yet.
pub fn notice_due(version: &str, seen: &str) -> bool {
    channel_of(version).is_prerelease() && seen != version
}

/// Whether the notice may appear in this run. The UI tests that do not ask for it (no
/// `BOOKX_TEST_VERSION`) never see it, so a beta version number cannot move their widgets.
pub fn notice_allowed(dev_script: bool, test_version: bool) -> bool {
    !dev_script || test_version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_comes_from_the_prerelease_part() {
        assert_eq!(channel_of("0.3.0"), Channel::Stable);
        assert_eq!(channel_of("0.3.0-beta.1"), Channel::Beta);
        assert_eq!(channel_of("0.3.0-BETA"), Channel::Beta);
        assert_eq!(channel_of("1.0.0-rc.2"), Channel::Rc);
        assert_eq!(channel_of("1.0.0-alpha"), Channel::Alpha);
        assert_eq!(channel_of("1.0.0-dev.4"), Channel::Prerelease);
        assert_eq!(channel_of("1.0.0+build.5"), Channel::Stable);
        assert_eq!(channel_of("1.0.0-rc.1+build.5"), Channel::Rc);
        assert_eq!(channel_of(""), Channel::Stable);
        assert_eq!(channel_of("1.0.0-"), Channel::Stable);
    }

    #[test]
    fn the_notice_is_due_once_per_beta_version() {
        assert!(notice_due("0.3.0-beta.1", ""));
        assert!(notice_due("0.3.0-beta.2", "0.3.0-beta.1"));
        assert!(!notice_due("0.3.0-beta.1", "0.3.0-beta.1"));
        assert!(!notice_due("0.3.0", ""));
        assert!(!notice_due("0.3.0", "0.3.0-beta.1"));
    }

    #[test]
    fn tests_without_a_version_override_never_see_the_notice() {
        assert!(notice_allowed(false, false));
        assert!(!notice_allowed(true, false));
        assert!(notice_allowed(true, true));
    }

    #[test]
    fn the_build_rule_and_this_rule_agree() {
        assert_eq!(
            channel_of(crate::config::VERSION),
            Channel::from_key(crate::config::CHANNEL)
        );
        for c in [
            Channel::Stable,
            Channel::Alpha,
            Channel::Beta,
            Channel::Rc,
            Channel::Prerelease,
        ] {
            assert_eq!(Channel::from_key(c.key()), c);
        }
        assert_eq!(Channel::Beta.tag(), "Beta");
        assert_eq!(Channel::Stable.tag(), "");
    }

    #[test]
    fn the_test_version_replaces_the_real_one_only_when_given() {
        assert_eq!(version_from(Some(" 9.9.9-beta.1 ".into())), "9.9.9-beta.1");
        assert_eq!(version_from(Some("  ".into())), crate::config::VERSION);
        assert_eq!(version_from(None), crate::config::VERSION);
        assert_eq!(
            channel_of(&version_from(Some("1.0.0-rc.1".into()))),
            Channel::Rc
        );
        // the process-wide readers agree with each other
        assert_eq!(channel(), channel_of(&version()));
    }
}
