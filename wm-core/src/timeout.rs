#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Message<T> {
    Set(T),
    Reset(T),
    Unset(T),
}
