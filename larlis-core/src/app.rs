pub trait App {
    fn execute(&mut self, op_num: u32, op: &[u8]) -> Vec<u8>;
}

// TODO common App -> actor::State adapter
