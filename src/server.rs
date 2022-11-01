use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use actix::prelude::*;
use actix_web::{
    middleware,
    web::{self, Data},
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_actors::ws;
use futures::{channel::mpsc, select, StreamExt};
use log::{error, info};
use tokio::runtime::Runtime;

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

pub type Sender<T> = mpsc::UnboundedSender<T>;
pub type Receiver<T> = mpsc::UnboundedReceiver<T>;

/// websocket connection is long running connection, it easier
/// to handle with an actor
#[derive(Debug, Clone)]
pub struct MyWebSocket {
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    hb: Instant,
    sender: Sender<String>,
}

impl MyWebSocket {
    pub fn new(sender: Sender<String>) -> Self {
        Self {
            hb: Instant::now(),
            sender,
        }
    }

    pub fn send(&mut self, msg: String) {
        match self.sender.start_send(msg.clone()) {
            Ok(_) => {}
            Err(e) => error!("Send message failed: {:?}", e),
        };
    }

    /// helper method that sends ping to client every 5 seconds (HEARTBEAT_INTERVAL).
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                // heartbeat timed out
                println!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");
        });
    }
}

impl Actor for MyWebSocket {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
    }
}

/// Handler for `ws::Message`
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        // process websocket messages
        println!("WS: {msg:?}");
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

struct AppState {
    toClient: Arc<RwLock<Sender<String>>>,
    fromServer: Arc<RwLock<Receiver<String>>>,
    ws: Arc<RwLock<MyWebSocket>>,
}

async fn index(state: Data<AppState>) -> impl Responder {
    state.ws.write().unwrap().send("Hello".to_string());
    format!("")
}

/// WebSocket handshake and start `MyWebSocket` actor.
async fn echo_ws(
    state: Data<AppState>,
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    ws::start(state.ws.read().unwrap().clone(), &req, stream)
}

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

pub async fn start_server(config: &ServerConfig) -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!(
        "starting HTTP server at http://{}:{:?}",
        config.host,
        config.port
    );
    let (toClient, fromServer) = mpsc::unbounded::<String>();
    let (mut sender, mut receiver) = mpsc::unbounded::<String>();

    // State
    let state = Data::new(AppState {
        toClient: Arc::new(RwLock::new(toClient)),
        fromServer: Arc::new(RwLock::new(fromServer)),
        ws: Arc::new(RwLock::new(MyWebSocket::new(sender.clone()))),
    });

    async fn start_server(state: Data<AppState>, config: &ServerConfig) -> std::io::Result<()> {
        HttpServer::new(move || {
            App::new()
                .app_data(state.clone())
                // WebSocket UI HTML file
                .service(web::resource("/").to(index))
                // websocket route
                .service(web::resource("/ws").route(web::get().to(echo_ws)))
                // enable logger
                .wrap(middleware::Logger::default())
        })
        .workers(2)
        .bind((config.host.as_str(), config.port))?
        .run()
        .await
    }

    async fn listen_receiver(receiver: &mut Receiver<String>) {
        loop {
            select! {
                msg = receiver.next() => match msg {
                    Some(msg) => {
                        info!("----->>>>>> {}", msg);
                    },
                    None => {}
                },
            }
        }
    }

    let _ = futures::join!(start_server(state, config), listen_receiver(&mut receiver));
    Ok(())
}
