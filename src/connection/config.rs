use std::time::Duration;

pub struct WsConfig {
    pub mask: bool,
    pub timeout: Duration,
    _private: (),
}

impl WsConfig {
    pub fn client() -> Self {
        Self {
            mask: true,
            timeout: Duration::from_secs(10),
            _private: (),
        }
    }
    pub fn server() -> Self {
        Self {
            mask: false,
            timeout: Duration::from_secs(10),
            _private: (),
        }
    }
}
