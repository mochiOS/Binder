mod desktop;
mod platform;
mod ui;
mod window;

use desktop::BinderApp;
use std::ffi::OsStr;
use viewkit::prelude::ViewKitError;

fn run_desktop() -> Result<(), ViewKitError> {
    viewkit::prelude::run::<BinderApp>()
}

fn run_about_process() -> ! {
    eprintln!("Binder About process started: pid={}", std::process::id(),);

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
