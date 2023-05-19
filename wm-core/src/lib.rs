pub mod actor;
pub mod app;
pub mod dispatch;
pub mod route;

pub mod transport {
    use std::net::SocketAddr;

    pub type Message<'m> = (SocketAddr, &'m [u8]);
    pub type OwnedMessage = (SocketAddr, Vec<u8>);
}

pub mod timeout {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum Message<T> {
        Set(T),
        Reset(T),
        Unset(T),
    }
}

pub use app::App;
pub use dispatch::Dispatch;
