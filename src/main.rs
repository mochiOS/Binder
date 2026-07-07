mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use std::ffi::OsStr;

use desktop::BinderApp;

use platform::ApplicationId;

use viewkit::prelude::{ViewKitError, run};

fn run_desktop() -> Result<(), ViewKitError> {
    run::<BinderApp>()
}

fn run_process_role(role: &OsStr) -> Result<(), ViewKitError> {
    let application = if role == OsStr::new("--role=about") {
        ApplicationId::About
    } else {
        eprintln!("unknown Binder role: {:?}", role,);

        return Ok(());
    };

    if let Err(error) = platform::run_application_process(application) {
        eprintln!("Binder child process failed: {error:?}",);
    }

    Ok(())
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

    run_process_role(&role)
}
