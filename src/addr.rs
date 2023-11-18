#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Addr {
    Socket(std::net::SocketAddr),
    Untyped(String),
}
