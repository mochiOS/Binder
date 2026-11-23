//! Read/decode off the UI thread; compositor objects remain on their UI owner.
use std::{
    cell::{Cell, RefCell},
    io,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
};

use viewkit::{dispatch::UiSubscription, platform::mochios::DesktopBackground, prelude::ImageData};

struct Readiness {
    gate: RefCell<crate::startup_readiness::Gate>,
    target: Cell<Option<mochi_user_platform::service_ready::Target>>,
}
impl Readiness {
    fn publish(&self, complete: bool) {
        if complete && let Some(target) = self.target.take() {
            if let Err(error) = mochi_user_platform::service_ready::notify(target, 0) {
                let _ = mochi_user_platform::logger::write_status_fmt(format_args!(
                    "Binder.app: desktop ready notification failed: {error:?}\n"
                ));
            }
        }
    }
    fn background_finished(&self) {
        let complete = self.gate.borrow_mut().background_finished();
        self.publish(complete);
    }
    fn frame_presented(&self) {
        let complete = self.gate.borrow_mut().frame_presented();
        self.publish(complete);
    }
}

pub(super) struct Background {
    readiness: Rc<Readiness>,
    requests: SyncSender<()>,
    generation: Arc<AtomicU64>,
    _subscription: UiSubscription,
    _surface: Rc<RefCell<Option<DesktopBackground>>>,
}

impl Background {
    /// Decode during shell initialization; compositor work stays on the UI thread.
    pub(super) fn start(
        target: Option<mochi_user_platform::service_ready::Target>,
    ) -> io::Result<Self> {
        let readiness = Rc::new(Readiness {
            gate: RefCell::default(),
            target: Cell::new(target),
        });
        let completed = Rc::clone(&readiness);
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
                    completed.background_finished();
                    continue;
                };
                let raster_start = std::time::Instant::now();
                match DesktopBackground::from_image(image) {
                    Ok(background) => {
                        *display.borrow_mut() = Some(background);
                        let _ = mochi_user_platform::logger::write_status_fmt(format_args!(
                            "Binder.app: desktop background presented raster_ms={} boot_ms={:?}\n",
                            raster_start.elapsed().as_millis(),
                            mochi_user_platform::time::monotonic_milliseconds()
                        ));
                    }
                    Err(error) => mochi_user_platform::logln!(
                        "Binder.app: desktop background failed: {error:?}"
                    ),
                }
                completed.background_finished();
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
                    let decode_start = std::time::Instant::now();
                    let image = crate::ui::wallpaper::Wallpaper::load_default_image();
                    let _ = mochi_user_platform::logger::write_status_fmt(format_args!(
                        "Binder.app: wallpaper decoded elapsed_ms={} boot_ms={:?}\n",
                        decode_start.elapsed().as_millis(),
                        mochi_user_platform::time::monotonic_milliseconds()
                    ));
                    if result.send((version, image)).is_err() || !notify.notify() {
                        break;
                    }
                }
            })?;
        let background = Self {
            readiness,
            requests: request,
            generation,
            _subscription: subscription,
            _surface: surface,
        };
        background.reload()?;
        Ok(background)
    }

    pub(super) fn first_frame_presented(&self) {
        self.readiness.frame_presented();
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
