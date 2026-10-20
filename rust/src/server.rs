use crate::{
    config::Config,
    model::{Record, Resource, SubscriptionCommand, Version},
    storage::Storage,
    tracker::{self, Connection, Tracker},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    task::{JoinHandle, JoinSet},
};

const MAX_MESSAGE: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Clone)]
struct App {
    tracker: Tracker,
    secret: [u8; 32],
    stop: watch::Receiver<bool>,
    heartbeat: Duration,
}

pub struct Server {
    pub publisher: SocketAddr,
    pub subscriber: SocketAddr,
    stop: watch::Sender<bool>,
    tracker: Tracker,
    http: JoinHandle<io::Result<()>>,
    actor: JoinHandle<()>,
}
impl Server {
    pub async fn start(config: Config) -> io::Result<Self> {
        let storage = Storage::open(config.storage)?;
        // Bind both listeners before spawning any work, so partial startup cleans up.
        let publisher =
            TcpListener::bind((config.publisher.ip.as_str(), config.publisher.port)).await?;
        let subscriber =
            TcpListener::bind((config.subscriber.ip.as_str(), config.subscriber.port)).await?;
        let publisher_addr = publisher.local_addr()?;
        let subscriber_addr = subscriber.local_addr()?;
        // Leave bounded handshake headroom so the tracker can return HTTP 503
        // at the WebSocket limit instead of resetting every excess upgrade.
        let connections = config.limits.connections + 32;
        let (tracker, actor) = Tracker::start(storage, config.cache_items, config.limits);
        let (stop, stop_rx) = watch::channel(false);
        let app = App {
            tracker: tracker.clone(),
            secret: Sha256::digest(config.secret.as_bytes()).into(),
            stop: stop_rx.clone(),
            heartbeat: config.heartbeat,
        };
        let publisher_app = Router::new()
            .route("/ping", get(|| async { "publisher server up" }))
            .route("/ping/", get(|| async { "publisher server up" }))
            .route("/publish", post(publish))
            .route("/publish/", post(publish))
            .layer(DefaultBodyLimit::max(100 * 1024))
            .layer(middleware::from_fn(deadline))
            .with_state(app.clone());
        let publisher_app = Router::new()
            .fallback_service(publisher_app)
            .layer(middleware::from_fn(canonical_paths));
        let subscriber_app = Router::new()
            .route("/ping", get(|| async { "subscriber server up" }))
            .route("/ping/", get(|| async { "subscriber server up" }))
            .route("/subscribe", get(upgrade))
            .route(
                "/recency-tracker.js",
                get(|| async {
                    (
                        [
                            ("content-type", "application/javascript; charset=utf-8"),
                            ("cache-control", "no-cache"),
                        ],
                        include_str!("../../resources/static/recency-tracker.js"),
                    )
                }),
            )
            .fallback(|headers: HeaderMap| async move {
                if headers.contains_key("upgrade") {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::NOT_FOUND
                }
            })
            .layer(middleware::from_fn(deadline))
            .with_state(app);
        let subscriber_app = Router::new()
            .fallback_service(subscriber_app)
            .layer(middleware::from_fn(canonical_paths));
        let http = tokio::spawn(async move {
            let (a, b) = tokio::join!(
                serve(publisher, publisher_app, stop_rx.clone(), connections),
                serve(subscriber, subscriber_app, stop_rx, connections)
            );
            a?;
            b?;
            Ok(())
        });
        Ok(Self {
            publisher: publisher_addr,
            subscriber: subscriber_addr,
            stop,
            tracker,
            http,
            actor,
        })
    }
    pub async fn shutdown(self) -> io::Result<()> {
        let _ = self.stop.send(true);
        // HTTP handlers finish accepted publications before the actor is drained/stopped.
        let result = self.http.await.map_err(io::Error::other)?;
        self.tracker.shutdown().await;
        self.actor.await.map_err(io::Error::other)?;
        result
    }
}
// The permit travels with the I/O through Hyper's WebSocket upgrade, so an
// upgraded socket stays charged against the same pre-header admission budget.
struct AdmittedStream {
    stream: TcpStream,
    _permit: OwnedSemaphorePermit,
}
impl AsyncRead for AdmittedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for AdmittedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}
async fn serve(
    listener: TcpListener,
    app: Router,
    mut stop: watch::Receiver<bool>,
    limit: usize,
) -> io::Result<()> {
    let budget = Arc::new(Semaphore::new(limit));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            Some(_)=tasks.join_next(), if !tasks.is_empty()=>{},
            accepted=listener.accept()=>{
                let (stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        // Descriptor exhaustion and aborted accepts are transient.
                        // Keep both listeners alive without a busy retry loop.
                        eprintln!("listener accept failed: {:?}", error.kind());
                        tokio::select! {
                            _=stop.changed()=>break,
                            _=tokio::time::sleep(Duration::from_millis(250))=>{}
                        }
                        continue;
                    }
                };
                let Ok(permit) = budget.clone().try_acquire_owned() else {continue};
                let app = app.clone();
                let mut stop = stop.clone();
                tasks.spawn(async move {
                    let mut builder = hyper::server::conn::http1::Builder::new();
                    builder.timer(TokioTimer::new()).header_read_timeout(REQUEST_TIMEOUT);
                    let connection = builder.serve_connection(
                        TokioIo::new(AdmittedStream { stream, _permit: permit }),
                        TowerToHyperService::new(app)
                    ).with_upgrades();
                    tokio::pin!(connection);
                    tokio::select! {
                        _=&mut connection=>{},
                        _=stop.changed()=>{
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                    }
                });
            }
        }
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}
// Express accepted case-insensitive dynamic HTTP routes and one trailing slash.
// Normalize only these known routes before the inner router dispatches them.
async fn canonical_paths(mut request: axum::extract::Request, next: Next) -> Response {
    let path = request.uri().path();
    let path = path.strip_suffix('/').unwrap_or(path);
    let canonical = if path.eq_ignore_ascii_case("/publish") {
        Some("/publish")
    } else if path.eq_ignore_ascii_case("/ping") {
        Some("/ping")
    } else {
        None
    };
    if let Some(path) = canonical {
        let mut parts = request.uri().clone().into_parts();
        let target = match request.uri().query() {
            Some(query) => format!("{path}?{query}"),
            None => path.to_owned(),
        };
        parts.path_and_query = Some(target.parse().expect("valid original query"));
        *request.uri_mut() = axum::http::Uri::from_parts(parts).expect("valid original URI");
    }
    next.run(request).await
}
async fn deadline(request: axum::extract::Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"ok":false,"error":"request timeout"})),
        )
            .into_response(),
    }
}
fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"ok":false,"error":message}))).into_response()
}
fn tracker_failure(error: tracker::Error) -> Response {
    if let tracker::Error::Storage(ref e) = error {
        eprintln!("resource storage operation failed: {:?}", e.kind());
    }
    failure(
        match error {
            tracker::Error::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        &error.to_string(),
    )
}
async fn publish(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    if !headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| {
            h.split(';').next().is_some_and(|t| {
                t.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
    {
        return failure(StatusCode::OK, "bad request");
    }
    if headers
        .get("content-encoding")
        .is_some_and(|h| h != "identity")
    {
        return failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported content encoding",
        );
    }
    let mut form = HashMap::new();
    for (count, (key, value)) in form_urlencoded::parse(&body).into_owned().enumerate() {
        if count >= 1000 {
            return failure(StatusCode::PAYLOAD_TOO_LARGE, "too many parameters");
        }
        if form.insert(key.clone(), value).is_some() {
            return failure(
                StatusCode::OK,
                if key == "secret" {
                    "bad secret"
                } else {
                    "bad request"
                },
            );
        }
    }
    let (Some(secret), Some(resource), Some(version)) = (
        form.get("secret"),
        form.get("resource"),
        form.get("version"),
    ) else {
        return failure(StatusCode::OK, "bad request");
    };
    let supplied: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
    if !bool::from(supplied.ct_eq(&app.secret)) {
        return failure(StatusCode::OK, "bad secret");
    }
    let data = match form.get("data") {
        Some(data) => match serde_json::from_str(data) {
            Ok(data) => data,
            Err(_) => return failure(StatusCode::OK, "bad data - invalid json"),
        },
        None => json!({}),
    };
    let Ok(resource) = Resource::try_from(resource.clone()) else {
        return failure(StatusCode::OK, "bad request");
    };
    match app
        .tracker
        .publish(
            resource,
            Record {
                version: Version(version.clone()),
                data,
            },
        )
        .await
    {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(e) => tracker_failure(e),
    }
}
async fn upgrade(State(app): State<App>, ws: WebSocketUpgrade) -> Response {
    if *app.stop.borrow() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "server is stopping");
    }
    let connection = match app.tracker.connect().await {
        Ok(c) => c,
        Err(e) => return tracker_failure(e),
    };
    ws.read_buffer_size(4096)
        .write_buffer_size(0)
        .max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_upgrade(move |socket| socket_loop(socket, app, connection))
        .into_response()
}
async fn send(socket: &mut WebSocket, message: Message) -> bool {
    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, socket.send(message)).await,
        Ok(Ok(()))
    )
}
async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = send(
        socket,
        Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })),
    )
    .await;
}
async fn socket_loop(mut socket: WebSocket, mut app: App, mut connection: Connection) {
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + app.heartbeat, app.heartbeat);
    let mut alive = true;
    loop {
        tokio::select! {
            _=app.stop.changed()=>{close(&mut socket,1001,"Server shutting down").await;break;}
            _=connection.cancelled.changed()=>{close(&mut socket,1013,"Reconnect and resubscribe").await;break;}
            _=heartbeat.tick()=>{
                if !alive {break;}
                alive=false;
                if !send(&mut socket,Message::Ping(Bytes::new())).await{break;}
            }
            update=connection.updates.recv()=>{
                let Some(update)=update else{break};
                if !send(&mut socket,Message::Text(update.text.to_string().into())).await{break;}
            }
            message=socket.recv()=>{
                let text=match message {
                    Some(Ok(Message::Text(text)))=>text,
                    Some(Ok(Message::Pong(_)))=>{alive=true;continue;}
                    Some(Ok(Message::Ping(_)))=>continue,
                    Some(Ok(Message::Binary(_)))=>{close(&mut socket,1003,"Text messages required").await;break;}
                    Some(Ok(Message::Close(_)))|None=>break,
                    Some(Err(error))=>{
                        let code=match error.into_inner().downcast::<tungstenite::Error>().as_deref() {
                            Ok(tungstenite::Error::Capacity(_))=>1009,
                            Ok(tungstenite::Error::Utf8(_))=>1007,
                            Ok(tungstenite::Error::Protocol(_))=>1002,
                            _=>break,
                        };
                        close(&mut socket,code,"Invalid or oversized message").await;break;
                    }
                };
                let value=match serde_json::from_str(&text){Ok(value)=>value,Err(_)=>{close(&mut socket,1007,"Invalid JSON").await;break;}};
                let Some(command)=SubscriptionCommand::parse(value) else{continue};
                // Keep shutdown/cancellation selectable during filesystem work. A
                // stalled actor must not suspend this socket's control loop forever.
                let operation = async {
                    match command {
                        SubscriptionCommand::Subscribe(resource,version)=>app.tracker.subscribe(connection.id,resource,version).await,
                        SubscriptionCommand::Unsubscribe(resource)=>app.tracker.unsubscribe(connection.id,resource).await,
                    }
                };
                let result = tokio::select! {
                    _=app.stop.changed()=>{close(&mut socket,1001,"Server shutting down").await;break;}
                    _=connection.cancelled.changed()=>{close(&mut socket,1013,"Reconnect and resubscribe").await;break;}
                    result=tokio::time::timeout(REQUEST_TIMEOUT,operation)=>match result {
                        Ok(result)=>result,
                        Err(_)=>{close(&mut socket,1011,"Subscription timeout").await;break;}
                    }
                };
                if let Err(e)=result {
                    let code=if matches!(e,tracker::Error::Storage(_)){1011}else{1013};
                    close(&mut socket,code,"Reconnect and resubscribe").await;break;
                }
            }
        }
    }
    // Connection drop sends cleanup. Closed update receivers are swept even if the
    // command queue was full, so disconnects cannot permanently retain subscriptions.
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn incomplete_headers_expire_and_connections_are_bounded_before_upgrade() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(serve(
            listener,
            Router::new().route("/ping", get(|| async { "up" })),
            stopped,
            1,
        ));
        let mut first = TcpStream::connect(address).await.unwrap();
        first
            .write_all(b"GET /ping HTTP/1.1\r\nHost:")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut excess = TcpStream::connect(address).await.unwrap();
        let mut byte = [0];
        let rejected = tokio::time::timeout(Duration::from_secs(1), excess.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(rejected, Ok(0) | Err(_)));
        let mut response = Vec::new();
        tokio::time::timeout(
            REQUEST_TIMEOUT + Duration::from_secs(2),
            first.read_to_end(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        let mut recovered = TcpStream::connect(address).await.unwrap();
        recovered
            .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        recovered.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn upgraded_io_retains_its_admission_permit() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/subscribe",
            get(|ws: WebSocketUpgrade| async {
                ws.on_upgrade(|mut socket| async move { while socket.recv().await.is_some() {} })
            }),
        );
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(serve(listener, app, stopped, 1));
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(b"GET /subscribe HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").await.unwrap();
        let mut response = [0; 1024];
        let read = socket.read(&mut response).await.unwrap();
        assert!(response[..read].starts_with(b"HTTP/1.1 101"));
        let mut excess = TcpStream::connect(address).await.unwrap();
        let rejected = tokio::time::timeout(Duration::from_secs(1), excess.read(&mut response))
            .await
            .unwrap();
        assert!(matches!(rejected, Ok(0) | Err(_)));
        drop(socket);
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
    }
}
