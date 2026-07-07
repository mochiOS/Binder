mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use desktop::BinderApp;
use platform::ApplicationId;
use std::ffi::OsStr;

use viewkit::prelude::{ViewKitError, run};

fn run_desktop() -> Result<(), ViewKitError> {
    run::<BinderApp>()
}

fn run_about_process() -> ! {
    if let Err(error) = crate::ipc::send_create_window(ApplicationId::About) {
        eprintln!("failed to request About window: {error:?}",);

        std::process::exit(1);
    }

    loop {
        std::thread::park();
    }
}

fn run_process_role(role: &OsStr) -> Result<(), ViewKitError> {
    if role == OsStr::new("--role=about") {
        run_about_process();
    }

    eprintln!("unknown Binder process argument: {:?}", role,);

    Ok(())
}

fn main() -> Result<(), ViewKitError> {
    let mut arguments = std::env::args_os();

    let _executable = arguments.next();

    let Some(first_argument) = arguments.next() else {
        return run_desktop();
    };

    if let Some(unexpected_argument) = arguments.next() {
        eprintln!("unexpected Binder argument: {:?}", unexpected_argument,);

        return Ok(());
    }

    run_process_role(&first_argument)
}
