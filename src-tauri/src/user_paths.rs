//! Keep provider credentials and local history on the same native user profile.

use std::{ffi::OsString, path::PathBuf};

pub(crate) fn home_dir() -> Option<PathBuf> {
    // Since Rust 1.85, Windows uses USERPROFILE / GetUserProfileDirectory rather
    // than Git Bash's possibly unrelated HOME. Unix still honors HOME.
    std::env::home_dir().filter(|home| !home.as_os_str().is_empty())
}

pub(crate) fn config_dir(
    configured: Option<OsString>,
    home: Option<PathBuf>,
    default_name: &str,
) -> Option<PathBuf> {
    // An explicit directory owns its scope even when it does not exist yet.
    configured
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(default_name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_directory_never_falls_back_to_another_profile() {
        assert_eq!(
            config_dir(Some("missing-custom-home".into()), None, ".codex"),
            Some(PathBuf::from("missing-custom-home"))
        );
        assert_eq!(
            config_dir(
                Some("custom-claude".into()),
                Some(PathBuf::from("ambient-home")),
                ".claude"
            ),
            Some(PathBuf::from("custom-claude"))
        );
    }

    #[test]
    fn default_directory_uses_the_supplied_native_profile() {
        let profile = PathBuf::from(r"C:\Users\Example User");
        for configured in [None, Some("".into())] {
            assert_eq!(
                config_dir(configured, Some(profile.clone()), ".codex"),
                Some(profile.join(".codex"))
            );
        }
        assert_eq!(config_dir(None, None, ".codex"), None);
    }
}
