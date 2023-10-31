#[async_trait::async_trait]
pub trait App {
    fn execute(&mut self, op: &[u8]) -> Vec<u8>;
}
