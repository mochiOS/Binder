mod desktop;
mod ipc;
mod platform;
mod ui;
mod window;

use binder_client::{BinderClient, Error as BinderClientError, WindowEvent, WindowOptions};
use desktop::BinderApp;
use std::collections::HashSet;
use std::ffi::OsStr;
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

fn run_test_process() -> Result<(), BinderClientError> {
    let mut client = BinderClient::connect()?;

    let mut windows = HashSet::new();

    for index in 0..3 {
        let window = client.create_test_window(
            WindowOptions::new(format!("Test Window {}", index + 1), 360, 220).resizable(true),
        )?;

        windows.insert(window);
    }

    loop {
        match client.next_event()? {
            WindowEvent::CloseRequested { window } if windows.contains(&window) => {
                client.close_window(window)?;

                windows.remove(&window);

                if windows.is_empty() {
                    return Ok(());
                }
            }

            WindowEvent::Resized { .. } => {}

            WindowEvent::FocusChanged { .. } => {}

            _ => {}
        }
    }
}

fn run_process_role(role: &OsStr) {
    if role == OsStr::new("--role=about") {
        if let Err(error) = run_about_process() {
            eprintln!("Binder child process failed: {error}",);
        }

        return;
    }

    if role == OsStr::new("--role=test") {
        if let Err(error) = run_test_process() {
            eprintln!("Binder test process failed: {error}",);
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
