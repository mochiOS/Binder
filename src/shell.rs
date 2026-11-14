//! Shell selection belongs to startup, never individual controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ShellMode {
    #[default]
    Desktop,
    Tablet,
}

impl ShellMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "desktop" => Some(Self::Desktop),
            "tablet" => Some(Self::Tablet),
            _ => None,
        }
    }

    pub(crate) fn current() -> Self {
        // The command line wins over the development environment. Device-
        // based selection can later supply the default at this single point.
        std::env::args()
            .find_map(|argument| argument.strip_prefix("--shell-mode=").and_then(Self::parse))
            .or_else(|| {
                std::env::var("BINDER_SHELL_MODE")
                    .ok()
                    .and_then(|value| Self::parse(&value))
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_selection_has_no_implicit_desktop_change() {
        assert_eq!(ShellMode::default(), ShellMode::Desktop);
        assert_eq!(ShellMode::parse("tablet"), Some(ShellMode::Tablet));
        assert_eq!(ShellMode::parse("mobile"), None);
    }
}
