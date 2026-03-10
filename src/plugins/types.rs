#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Middleware,
    Rules,
    Provider,
    Storage,
}
