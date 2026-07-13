mod apps;
mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use desktop::BinderApp;
use std::ffi::OsStr;
use viewkit::prelude::{run, ViewKitError};

fn run_desktop() -> Result<(), ViewKitError> {
    eprintln!("Binder.app: desktop start");
    let result = run::<BinderApp>();
    if let Err(error) = &result {
        eprintln!("Binder.app: desktop exited with error: {error:?}");
    } else {
        eprintln!("Binder.app: desktop exited");
    }
    result
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
