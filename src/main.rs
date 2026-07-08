mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use std::ffi::OsStr;

use binder_client::{BinderClient, Error as BinderClientError, WindowEvent, WindowOptions};

use desktop::BinderApp;

use viewkit::prelude::{ViewKitError, run};

fn run_desktop() -> Result<(), ViewKitError> {
    run::<BinderApp>()
}

fn run_about_process() -> Result<(), BinderClientError> {
    let mut client = BinderClient::connect()?;

    let window =
        client.create_window(WindowOptions::new("About mochiOS", 420, 300).resizable(true))?;

    loop {
        match client.next_event()? {
            WindowEvent::CloseRequested {
                window: event_window,
            } if event_window == window => {
                client.close_window(window)?;

                return Ok(());
            }

            WindowEvent::Resized {
                window: event_window,
                ..
            } if event_window == window => {}

            WindowEvent::FocusChanged {
                window: event_window,
                ..
            } if event_window == window => {}

            _ => {}
        }
    }
}

fn run_process_role(role: &OsStr) {
    if role != OsStr::new("--role=about") {
        eprintln!("unknown Binder role: {:?}", role,);

        return;
    }

    if let Err(error) = run_about_process() {
        eprintln!("Binder child process failed: {error}",);
    }
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
