use std::cell::RefCell;
use std::rc::Rc;

use crate::apps;
use crate::platform::{DesktopPlatform, SystemAction};
use viewkit::prelude::*;

use crate::window::DesktopWindows;

pub(crate) fn view(
    platform: Rc<RefCell<dyn DesktopPlatform>>,

    menu_open: State<bool>,
    windows: State<DesktopWindows>,
) -> Menu {
    let about_platform = Rc::clone(&platform);

    let windows_for_about = windows.clone();

    let about_menu_open = menu_open.clone();

    let settings_platform = Rc::clone(&platform);

    let settings_menu_open = menu_open.clone();

    Menu::new()
        .item(system_action_item(
            "Sleep",
            SystemAction::Sleep,
            Rc::clone(&platform),
            menu_open.clone(),
        ))
        .item(system_action_item(
            "Restart",
            SystemAction::Restart,
            Rc::clone(&platform),
            menu_open.clone(),
        ))
        .item(system_action_item(
            "Shutdown",
            SystemAction::ShutDown,
            Rc::clone(&platform),
            menu_open.clone(),
        ))
        .item(system_action_item(
            "Logout",
            SystemAction::LogOut,
            platform,
            menu_open,
        ))
        .separator()
        .item(MenuItem::new("About").on_select(move || {
            about_menu_open.set(false);

            let existing = {
                let desktop = windows_for_about.get();

                desktop.about_window()
            };

            if let Some(window_id) = existing {
                windows_for_about.update(|desktop| {
                    desktop.focus(window_id);
                });

                return;
            }

            let result = about_platform
                .borrow_mut()
                .launch_internal_window(apps::ABOUT_ENTRY);

            if let Err(error) = result {
                eprintln!("failed to launch About: {error:?}",);
            }
        }))
        .item(MenuItem::new("Settings").on_select(move || {
            settings_menu_open.set(false);

            if let Err(error) = settings_platform.borrow().open_system_settings() {
                eprintln!("failed to open system settings: {error:?}",);
            }
        }))
}

fn system_action_item(
    label: &'static str,
    action: SystemAction,

    platform: Rc<RefCell<dyn DesktopPlatform>>,

    menu_open: State<bool>,
) -> MenuItem {
    MenuItem::new(label).on_select(move || {
        menu_open.set(false);

        if let Err(error) = platform.borrow().perform_system_action(action) {
            eprintln!("failed to perform {action:?}: {error:?}",);
        }
    })
}
