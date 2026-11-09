/// reqwest's `rustls-no-provider` mode requires a process provider even when
/// rustls was compiled with ring. Preserve an explicitly preinstalled provider.
pub fn initialize() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Another caller may install a provider concurrently; either outcome
        // leaves the immutable process default available to HTTP clients.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn initialization_is_idempotent_and_concurrent() {
        super::initialize();
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(super::initialize))
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(std::sync::Arc::ptr_eq(
            provider,
            rustls::crypto::CryptoProvider::get_default().unwrap()
        ));
    }

    #[test]
    fn provider_initialization_precedes_app_startup() {
        let source = include_str!("../../iceon-ui/src/lib.rs");
        let startup = source.split("pub fn run() {").nth(1).unwrap().trim_start();
        assert!(startup.starts_with("iceon_core::tls::initialize();"));
        assert!(startup.find("runtime.enter()").unwrap() < startup.find("CatalogEngine::").unwrap());
    }
}
