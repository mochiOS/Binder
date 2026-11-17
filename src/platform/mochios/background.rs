//! Read/decode off the UI thread; compositor objects remain on their UI owner.
use std::{
    cell::RefCell,
    io,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc,
    },
};

use viewkit::{dispatch::UiSubscription, platform::mochios::DesktopBackground, prelude::ImageData};

pub(super) struct Background {
    requests: SyncSender<()>,
    generation: Arc<AtomicU64>,
    _subscription: UiSubscription,
    _surface: Rc<RefCell<Option<DesktopBackground>>>,
}

impl Background {
    /// Start only after the shell has presented its first usable frame.
    pub(super) fn start() -> io::Result<Self> {
        let (request, requests) = mpsc::sync_channel(1);
        let (result, results) = mpsc::sync_channel::<(u64, Option<ImageData>)>(1);
        let generation = Arc::new(AtomicU64::new(0));
        let surface = Rc::new(RefCell::new(None));
        let current = Arc::clone(&generation);
        let display = Rc::clone(&surface);
        let subscription = UiSubscription::new(move || {
            while let Ok((version, image)) = results.try_recv() {
                if version != current.load(Ordering::Acquire) {
                    continue;
                }
                let Some(image) = image else {
                    mochi_user_platform::logln!("Binder.app: desktop wallpaper unavailable");
                    continue;
                };
                match DesktopBackground::from_image(image) {
                    Ok(background) => {
                        *display.borrow_mut() = Some(background);
                        mochi_user_platform::logln!("Binder.app: desktop background presented");
                    }
                    Err(error) => mochi_user_platform::logln!(
                        "Binder.app: desktop background failed: {error:?}"
                    ),
                }
            }
        })
        .map_err(io::Error::other)?;
        let notify = subscription.notifier();
        let current = Arc::clone(&generation);
        std::thread::Builder::new()
            .name("binder-wallpaper".into())
            .spawn(move || {
                while requests.recv().is_ok() {
                    // One active load and one queued reload, even during rapid
                    // appearance changes. Never publish an obsolete result.
                    let _ = requests.try_recv();
                    let version = current.load(Ordering::Acquire);
                    let image = crate::ui::wallpaper::Wallpaper::load_default_image();
                    if result.send((version, image)).is_err() || !notify.notify() {
                        break;
                    }
                }
            })?;
        let background = Self {
            requests: request,
            generation,
            _subscription: subscription,
            _surface: surface,
        };
        background.reload()?;
        Ok(background)
    }

    pub(super) fn reload(&self) -> io::Result<()> {
        // This owner is UI-thread-only; the worker only reads the generation.
        let next = self
            .generation
            .load(Ordering::Relaxed)
            .checked_add(1)
            .ok_or_else(|| io::Error::other("wallpaper generation exhausted"))?;
        self.generation.store(next, Ordering::Release);
        match self.requests.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => {
                Err(io::Error::other("wallpaper worker stopped"))
            }
        }
    }
}
