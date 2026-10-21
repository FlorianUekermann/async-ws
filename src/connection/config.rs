use std::time::Duration;

#[non_exhaustive]
pub struct WsConfig {
    pub mask: bool,
    pub timeout: Duration,
}

impl WsConfig {
    pub fn client() -> Self {
        Self {
            mask: true,
            timeout: Duration::from_secs(10),
        }
    }
    pub fn server() -> Self {
        Self {
            mask: false,
            timeout: Duration::from_secs(10),
        }
    }
}
