//! The one executor for all background work. Tasks are cheap, so callers
//! spawn tasks per request and let the runtime schedule them. Built lazily
//! so tests that never fetch pay nothing, and never dropped.

pub fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Spawn a background task on the shared runtime. Unit tests never run
/// it: the task is dropped, so no test can reach the network.
pub fn spawn_bg<F: std::future::Future<Output = ()> + Send + 'static>(task: F) {
    if cfg!(any(test, feature = "test-support")) {
        drop(task);
        return;
    }
    rt().spawn(task);
}
