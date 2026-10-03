mod apps;
mod control_center_preferences;
mod desktop;
mod dock_preferences;
mod ipc;
mod platform;
mod session;
mod ui;
mod window;

use desktop::BinderApp;
use std::ffi::OsStr;
use viewkit::prelude::{ViewKitError, run};

#[cfg(target_os = "mochios")]
fn init_logging() -> bool {
    mochi_user_platform::logger::init_from_env().is_some()
}

#[cfg(not(target_os = "mochios"))]
fn init_logging() -> bool {
    false
}

#[cfg(target_os = "mochios")]
fn log_desktop_failure(error: &ViewKitError) {
    let _ = mochi_user_platform::logger::write_status_fmt(format_args!(
        "Binder.app: desktop failed: {error:?}\n"
    ));
}

#[cfg(not(target_os = "mochios"))]
fn log_desktop_failure(_error: &ViewKitError) {}

fn run_desktop() -> Result<(), ViewKitError> {
    if let Some(home) = std::env::var_os("HOME") {
        let _ = std::env::set_current_dir(home);
    }
    run::<BinderApp>()
}

fn run_process_role(role: &OsStr) {
    if role == OsStr::new(apps::ABOUT_ROLE) {
        if let Err(error) = platform::run_internal_process(apps::ABOUT_ENTRY) {
            eprintln!("Binder About process failed: {error:?}",);
        }

        return;
    }

    if role == OsStr::new(apps::TEST_ROLE) {
        if let Err(error) = platform::run_internal_process(apps::TEST_ENTRY) {
            eprintln!("Binder Test process failed: {error:?}",);
        }

        return;
    }

    eprintln!("unknown Binder role: {:?}", role,);
}

fn main() -> Result<(), ViewKitError> {
    let has_logger_endpoint = init_logging();
    let mut arguments = std::env::args_os();

    let _executable = arguments.next();

    if has_logger_endpoint {
        let _logger_endpoint = arguments.next();
    }

    let Some(role) = arguments.next() else {
        return run_desktop().inspect_err(log_desktop_failure);
    };

    if role
        .to_str()
        .is_some_and(|role| role.starts_with(session::USER_ARGUMENT_PREFIX))
    {
        if let Some(argument) = arguments.next() {
            eprintln!("unexpected Binder argument: {:?}", argument,);
            return Ok(());
        }
        return run_desktop().inspect_err(log_desktop_failure);
    }

    if let Some(argument) = arguments.next() {
        eprintln!("unexpected Binder argument: {:?}", argument,);

        return Ok(());
    }

    run_process_role(&role);

    Ok(())
}
