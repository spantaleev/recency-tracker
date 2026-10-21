//! HTTP/1 WebSocket upgrade glue using the protocol library's public typed API.
use axum::{body::Body, extract::Request, http::StatusCode, response::Response};
use hyper_util::rt::TokioIo;
use std::future::Future;
pub use tokio_tungstenite::tungstenite::{Error, Message, protocol::CloseFrame};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        handshake::server::create_response_with_body,
        protocol::{Role, WebSocketConfig},
    },
};

pub type WebSocket = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;
pub struct Upgrade {
    response: Response,
    upgraded: hyper::upgrade::OnUpgrade,
}
impl Upgrade {
    pub fn new(mut request: Request) -> Result<Self, StatusCode> {
        // Let the maintained protocol library validate methods, headers and keys.
        let response = create_response_with_body(&request, Body::empty)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok(Self {
            response,
            upgraded: hyper::upgrade::on(&mut request),
        })
    }
    pub fn on_upgrade<F, Fut>(self, callback: F) -> Response
    where
        F: FnOnce(WebSocket) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(async move {
            let Ok(stream) = self.upgraded.await else {
                return;
            };
            let config = WebSocketConfig::default()
                .read_buffer_size(4096)
                .write_buffer_size(0)
                .max_message_size(Some(64 * 1024))
                .max_frame_size(Some(64 * 1024));
            let socket =
                WebSocketStream::from_raw_socket(TokioIo::new(stream), Role::Server, Some(config))
                    .await;
            callback(socket).await;
        });
        self.response
    }
}
