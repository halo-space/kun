#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineAction {
    Ack,
    Nack,
    Retry,
}
