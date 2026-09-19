//! Bounded in-process socket used by the same realtime state machines as WebSocket.
use axum::extract::ws::Message;
use futures::{Sink, Stream, channel::mpsc};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pub struct Socket {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
}
pub fn pair() -> (Socket, Socket) {
    let (atx, arx) = mpsc::channel(16);
    let (btx, brx) = mpsc::channel(16);
    (Socket { tx: atx, rx: brx }, Socket { tx: btx, rx: arx })
}
impl Stream for Socket {
    type Item = Result<Message, axum::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.rx)
            .poll_next(cx)
            .map(|value| value.map(Ok))
    }
}
impl Sink<Message> for Socket {
    type Error = axum::Error;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx)
            .poll_ready(cx)
            .map_err(axum::Error::new)
    }
    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        Pin::new(&mut self.tx)
            .start_send(item)
            .map_err(axum::Error::new)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx)
            .poll_flush(cx)
            .map_err(axum::Error::new)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.tx)
            .poll_close(cx)
            .map_err(axum::Error::new)
    }
}
