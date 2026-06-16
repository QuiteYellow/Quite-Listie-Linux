use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

use crate::engine::unified_provider::UnifiedProvider;

static PROVIDER: OnceLock<Arc<Mutex<UnifiedProvider>>> = OnceLock::new();

pub fn get() -> Arc<Mutex<UnifiedProvider>> {
    PROVIDER
        .get_or_init(|| Arc::new(Mutex::new(UnifiedProvider::default())))
        .clone()
}
