mod desktop;
mod platform;
mod ui;
mod window;

use desktop::BinderApp;
use viewkit::prelude::{ViewKitError, run};

fn main() -> Result<(), ViewKitError> {
    run::<BinderApp>()
}
