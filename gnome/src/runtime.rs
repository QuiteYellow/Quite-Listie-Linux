//! Bridge between the engine's `tokio` world and the GTK/GLib main loop.
//!
//! The shared core does blocking and network I/O on a multi-threaded tokio runtime.
//! GTK must only be touched from the main thread. The pattern, used throughout the
//! controller, is:
//!
//! 1. clone the `Arc<Mutex<UnifiedProvider>>`,
//! 2. `runtime().spawn(async move { … })` to do the work off the main thread,
//! 3. deliver the result back via an `async_channel` that is awaited inside
//!    `glib::spawn_future_local`, so the continuation runs on the main thread.
//!
//! [`spawn_to_main`] packages that round-trip for the common case.

use std::future::Future;
use std::sync::OnceLock;

use gtk::glib;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Initialise the global tokio runtime. Call once from `main` before the app runs.
///
/// Also enters the runtime context on the calling (main) thread and leaks the guard,
/// so the engine's bare `tokio::spawn` calls (e.g. `start_reminder_tasks`) work when
/// invoked directly from GTK callbacks. Mirrors the `rt.enter()` guard the KDE `main`
/// held for the whole process.
pub fn init() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    RUNTIME
        .set(rt)
        .unwrap_or_else(|_| panic!("runtime already initialised"));
    // Keep the main thread inside the runtime context for the whole program.
    std::mem::forget(runtime().enter());
}

/// Access the global tokio runtime.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get().expect("runtime not initialised — call runtime::init() first")
}

/// Run `fut` on the tokio runtime, then run `on_main(result)` back on the GLib main
/// thread. `fut`'s output must be `Send`; `on_main` runs on the main thread and may
/// touch GTK freely.
pub fn spawn_to_main<F, T, M>(fut: F, on_main: M)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    M: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded::<T>(1);
    runtime().spawn(async move {
        let result = fut.await;
        let _ = tx.send(result).await;
    });
    glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            on_main(value);
        }
    });
}
