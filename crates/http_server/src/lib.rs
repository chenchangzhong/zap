use std::net::SocketAddr;

use tower_http::trace::TraceLayer;
use warpui::{Entity, ModelContext, SingletonEntity};

// Spells "Zap" - should hopefully not conflict with other ports.
// Does not conflict with known ports on https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers
const PORT: u16 = 9277;

/// A singleton model for the small HTTP server that is run by the Zap client.
pub struct HttpServer {
    /// The tokio runtime that the HTTP server runs on.
    ///
    /// We use a private runtime only because we don't currently have a shared
    /// tokio runtime.
    ///
    /// TODO(vorporeal): Remove this when we have a shared tokio runtime.
    _runtime: Option<tokio::runtime::Runtime>,
}

impl HttpServer {
    pub fn new(
        routers: impl IntoIterator<Item = axum::Router>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        let runtime = Self::spawn_server(routers)
            .inspect_err(|err| {
                log::warn!("Failed to start local HTTP server: {err:#}");
            })
            .ok();

        Self { _runtime: runtime }
    }

    fn spawn_server(
        routers: impl IntoIterator<Item = axum::Router>,
    ) -> Result<tokio::runtime::Runtime, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()?;

        let mut root = axum::Router::new();
        for router in routers {
            root = root.merge(router);
        }

        runtime.spawn(async move {
            let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
            // bind 失败原本只让这个 async 任务返回 Err(静默无监听,调用方以为成功);
            // 显式记录,避免"端口被别的实例占用"这类问题被完全掩盖。
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => listener,
                Err(err) => {
                    log::error!("Failed to bind local HTTP server on {addr}: {err:#}");
                    return Err(err);
                }
            };

            axum::serve(listener, root.layer(TraceLayer::new_for_http())).await
        });

        Ok(runtime)
    }
}

impl Entity for HttpServer {
    type Event = ();
}

impl SingletonEntity for HttpServer {}
