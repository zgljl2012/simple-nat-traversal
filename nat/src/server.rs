use std::sync::Arc;

use log::{debug, info, error};
use tokio::{sync::{RwLock, mpsc::{UnboundedSender, UnboundedReceiver, self}}, select};

use crate::{NatStream, Message};


pub struct NatServer {
    stream: Option<NatStream>,
    sender: Arc<RwLock<UnboundedSender<Message>>>,
    receiver: UnboundedReceiver<Message>,
    exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
    exteral_receiver: UnboundedReceiver<Message>,
}

impl NatServer {
    pub fn new(
        exteral_sender: Arc<RwLock<UnboundedSender<Message>>>,
        exteral_receiver: UnboundedReceiver<Message>,
    ) -> NatServer {
        let (sender, receiver) = mpsc::unbounded_channel::<Message>();
        Self {
            stream: None,
            receiver: receiver,
            sender: Arc::new(RwLock::new(sender)),
            exteral_receiver,
            exteral_sender,
        }
    }

    pub fn init(&mut self, stream: NatStream) {
        self.stream = Some(stream);
    }

    async fn send_text(&mut self, msg: &str) -> std::io::Result<()> {
        if self.stream.is_none() {
            return Ok(());
        }
        let binding = self.stream.as_ref().unwrap();
        let stream = binding.write().await;
        stream.writable().await?;
        match stream.try_write(msg.as_bytes()) {
            Ok(n) => {
                debug!("Send text {:?}({:?} bytes) successfully", msg, n);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                return Err(e.into());
            }
        };
        Ok(())
    }

    pub async fn ok(&mut self) -> std::io::Result<()> {
        info!("Reply OK to client");
        self.send_text("OK").await
    }

    pub async fn run_forever(&mut self) {
        // Wait for NAT server
        let stream = self.stream.as_ref().unwrap().write().await;
        let sender = &mut self.sender;
        let receiver = &mut self.receiver;
        let exteral_receiver = &mut self.exteral_receiver;
        let exteral_sender = &mut self.exteral_sender;
        loop {
            select! {
                _ = stream.readable() => {
                    match Message::from_stream(&stream).await {
                        Ok(msg) => match msg {
                            Some(msg) => {
                                debug!("Received {:?}", msg.protocol);
                                if msg.is_ping() {
                                    debug!("You received PING from NAT client");
                                    sender.write().await.send(Message::pong()).unwrap();
                                }
                                if msg.is_http() {
                                    info!("Received HTTP Response from client");
                                    exteral_sender.write().await.send(msg).unwrap();
                                }
                            },
                            None => {}
                        },
                        Err(err) => {
                            error!("{:?}", err);
                            break;
                        },
                    }
                },
                exteral_msg = exteral_receiver.recv() => match exteral_msg {
                    Some(msg) => {
                        sender.write().await.send(msg).unwrap();
                    },
                    None => todo!()
                },
                msg = receiver.recv() => match msg {
                    Some(msg) => {
                        msg.write_to(&stream).await;
                    },
                    None => todo!(),
                }
            }
        }
    }
}

