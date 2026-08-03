pub(crate) const USER_ARGUMENT_PREFIX: &str = "--session-user=";

pub(crate) fn current_user_label() -> String {
    if let Some(name) = std::env::args().find_map(|argument| {
        argument
            .strip_prefix(USER_ARGUMENT_PREFIX)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    }) {
        return name;
    }

    platform_user_label()
}

#[cfg(target_os = "mochios")]
fn platform_user_label() -> String {
    String::from("Unknown User")
}

#[cfg(not(target_os = "mochios"))]
fn platform_user_label() -> String {
    std::env::var("USER").unwrap_or_else(|_| String::from("Unknown User"))
}
