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
