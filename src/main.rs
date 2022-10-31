//! Simple echo websocket server.
//!
//! Open `http://localhost:8080/` in browser to test.

use actix_web::{middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;

mod server;
use self::server::MyWebSocket;

async fn index() -> impl Responder {
    format!("")
}

/// WebSocket handshake and start `MyWebSocket` actor.
async fn echo_ws(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    ws::start(MyWebSocket::new(), &req, stream)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!("starting HTTP server at http://localhost:8080");

    HttpServer::new(|| {
        App::new()
            // WebSocket UI HTML file
            .service(web::resource("/").to(index))
            // websocket route
            .service(web::resource("/ws").route(web::get().to(echo_ws)))
            // enable logger
            .wrap(middleware::Logger::default())
    })
    .workers(2)
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
