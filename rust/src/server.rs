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
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io, net::SocketAddr, time::Duration};
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

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
        let publisher = TcpListener::bind((config.publisher.ip, config.publisher.port)).await?;
        let subscriber = TcpListener::bind((config.subscriber.ip, config.subscriber.port)).await?;
        let publisher_addr = publisher.local_addr()?;
        let subscriber_addr = subscriber.local_addr()?;
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
            .route("/publish", post(publish))
            .layer(DefaultBodyLimit::max(100 * 1024))
            .layer(middleware::from_fn(deadline))
            .with_state(app.clone());
        let subscriber_app = Router::new()
            .route("/ping", get(|| async { "subscriber server up" }))
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
        let mut pub_stop = stop_rx.clone();
        let mut sub_stop = stop_rx;
        let http = tokio::spawn(async move {
            let (a, b) = tokio::join!(
                axum::serve(publisher, publisher_app).with_graceful_shutdown(async move {
                    let _ = pub_stop.changed().await;
                }),
                axum::serve(subscriber, subscriber_app).with_graceful_shutdown(async move {
                    let _ = sub_stop.changed().await;
                })
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
                let result=match command {
                    SubscriptionCommand::Subscribe(resource,version)=>app.tracker.subscribe(connection.id,resource,version).await,
                    SubscriptionCommand::Unsubscribe(resource)=>app.tracker.unsubscribe(connection.id,resource).await,
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
