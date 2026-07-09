mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use desktop::BinderApp;
use std::ffi::OsStr;
use viewkit::prelude::{ViewKitError, run};
use window::WindowContent;

fn run_desktop() -> Result<(), ViewKitError> {
    run::<BinderApp>()
}

fn run_process_role(role: &OsStr) {
    if role == OsStr::new("--role=about") {
        if let Err(error) = platform::run_internal_process(WindowContent::About) {
            eprintln!("Binder About process failed: {error:?}",);
        }

        return;
    }

    if role == OsStr::new("--role=test") {
        if let Err(error) = platform::run_internal_process(WindowContent::Test) {
            eprintln!("Binder Test process failed: {error:?}",);
        }

        return;
    }

    eprintln!("unknown Binder role: {:?}", role,);
}

fn main() -> Result<(), ViewKitError> {
    let mut arguments = std::env::args_os();

    let _executable = arguments.next();

    let Some(role) = arguments.next() else {
        return run_desktop();
    };

    if let Some(argument) = arguments.next() {
        eprintln!("unexpected Binder argument: {:?}", argument,);

        return Ok(());
    }

    run_process_role(&role);

    Ok(())
}
