use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::item::Item;
use crate::request::Request;
use crate::response::Response;
use crate::stats::Snapshot as StatsSnapshot;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    SpiderOpened,
    SpiderClosed,
    RequestScheduled,
    ResponseReceived,
    ItemScraped,
    SpiderError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiderOpened {
    pub spider_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiderClosed {
    pub spider_name: String,
    pub stats: StatsSnapshot,
}

#[derive(Debug, Clone)]
pub struct RequestScheduled {
    pub spider_name: String,
    pub request: Request,
}

#[derive(Debug, Clone)]
pub struct ResponseReceived {
    pub spider_name: String,
    pub response: Response,
}

#[derive(Debug, Clone)]
pub struct ItemScraped {
    pub spider_name: String,
    pub item: Item,
}

#[derive(Debug, Clone)]
pub struct SpiderErrorSignal {
    pub spider_name: String,
    pub request: Request,
    pub response: Option<Response>,
    pub error: SpiderError,
}

#[derive(Debug, Clone)]
pub enum Signal {
    SpiderOpened(SpiderOpened),
    SpiderClosed(SpiderClosed),
    RequestScheduled(RequestScheduled),
    ResponseReceived(ResponseReceived),
    ItemScraped(ItemScraped),
    SpiderError(SpiderErrorSignal),
}

impl Signal {
    pub fn spider_opened(spider_name: impl Into<String>) -> Self {
        Self::SpiderOpened(SpiderOpened {
            spider_name: spider_name.into(),
        })
    }

    pub fn spider_closed(spider_name: impl Into<String>, stats: StatsSnapshot) -> Self {
        Self::SpiderClosed(SpiderClosed {
            spider_name: spider_name.into(),
            stats,
        })
    }

    pub fn request_scheduled(spider_name: impl Into<String>, request: Request) -> Self {
        Self::RequestScheduled(RequestScheduled {
            spider_name: spider_name.into(),
            request,
        })
    }

    pub fn response_received(spider_name: impl Into<String>, response: Response) -> Self {
        Self::ResponseReceived(ResponseReceived {
            spider_name: spider_name.into(),
            response,
        })
    }

    pub fn item_scraped(spider_name: impl Into<String>, item: Item) -> Self {
        Self::ItemScraped(ItemScraped {
            spider_name: spider_name.into(),
            item,
        })
    }

    pub fn spider_error(
        spider_name: impl Into<String>,
        request: Request,
        response: Option<Response>,
        error: SpiderError,
    ) -> Self {
        Self::SpiderError(SpiderErrorSignal {
            spider_name: spider_name.into(),
            request,
            response,
            error,
        })
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::SpiderOpened(_) => Kind::SpiderOpened,
            Self::SpiderClosed(_) => Kind::SpiderClosed,
            Self::RequestScheduled(_) => Kind::RequestScheduled,
            Self::ResponseReceived(_) => Kind::ResponseReceived,
            Self::ItemScraped(_) => Kind::ItemScraped,
            Self::SpiderError(_) => Kind::SpiderError,
        }
    }

    pub fn spider_name(&self) -> &str {
        match self {
            Self::SpiderOpened(signal) => signal.spider_name.as_str(),
            Self::SpiderClosed(signal) => signal.spider_name.as_str(),
            Self::RequestScheduled(signal) => signal.spider_name.as_str(),
            Self::ResponseReceived(signal) => signal.spider_name.as_str(),
            Self::ItemScraped(signal) => signal.spider_name.as_str(),
            Self::SpiderError(signal) => signal.spider_name.as_str(),
        }
    }
}

pub trait Listener: Send + Sync {
    fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()>;
}

#[derive(Default)]
pub(crate) struct Bus {
    listeners: Mutex<Vec<Arc<dyn Listener>>>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Bus")
            .field(
                "listener_count",
                &self.listeners.lock().map(|it| it.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl Bus {
    pub(crate) fn add_listener(&self, listener: Arc<dyn Listener>) {
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(listener);
        }
    }

    pub(crate) async fn emit(&self, signal: Signal) {
        let listeners = match self.listeners.lock() {
            Ok(listeners) => listeners.clone(),
            Err(_) => return,
        };

        for listener in listeners {
            listener.on_signal(&signal).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingListener {
        events: Mutex<Vec<Kind>>,
    }

    impl Listener for RecordingListener {
        fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                self.events.lock().unwrap().push(signal.kind());
            })
        }
    }

    #[tokio::test]
    async fn bus_notifies_registered_listeners() {
        let bus = Bus::default();
        let listener = Arc::new(RecordingListener::default());
        bus.add_listener(listener.clone());

        bus.emit(Signal::spider_opened("example")).await;
        bus.emit(Signal::spider_closed("example", StatsSnapshot::default()))
            .await;

        assert_eq!(
            listener.events.lock().unwrap().clone(),
            vec![Kind::SpiderOpened, Kind::SpiderClosed]
        );
    }
}
