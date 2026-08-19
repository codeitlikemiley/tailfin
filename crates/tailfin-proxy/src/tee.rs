use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

pin_project_lite::pin_project! {
    /// Streams `inner` to the client while cloning each data frame to `tx`.
    ///
    /// `try_send` is load-bearing: a stalled or dead meter must never
    /// backpressure the relay. A full channel drops the clone; a closed
    /// channel disables further teeing.
    pub struct TeeBody<B> {
        #[pin]
        inner: B,
        tx: Option<mpsc::Sender<Bytes>>,
    }
}

impl<B> TeeBody<B> {
    pub fn new(inner: B, tx: mpsc::Sender<Bytes>) -> Self {
        Self {
            inner,
            tx: Some(tx),
        }
    }
}

impl<B> Body for TeeBody<B>
where
    B: Body<Data = Bytes>,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match this.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    if let Some(tx) = this.tx.as_ref() {
                        match tx.try_send(data.clone()) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Closed(_)) => {
                                *this.tx = None;
                            }
                        }
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};

    #[tokio::test]
    async fn tee_clones_frames_without_consuming_them() {
        let (tx, mut rx) = mpsc::channel(4);
        let teed = TeeBody::new(Full::new(Bytes::from_static(b"hello")), tx);
        let out = teed.collect().await.unwrap().to_bytes();
        assert_eq!(out.as_ref(), b"hello");
        assert_eq!(rx.recv().await.unwrap().as_ref(), b"hello");
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn tee_keeps_yielding_after_meter_is_dropped() {
        let (tx, rx) = mpsc::channel(4);
        drop(rx);
        let teed = TeeBody::new(Full::new(Bytes::from_static(b"still here")), tx);
        let out = teed.collect().await.unwrap().to_bytes();
        assert_eq!(out.as_ref(), b"still here");
    }
}
