use hyper::body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

type OnEnd = Box<dyn FnOnce(bool) + Send>;

/// Runs `on_end(complete)` when dropped. Completeness is set by `FinishBody`.
pub struct FinishOnDrop {
    on_end: Mutex<Option<OnEnd>>,
    complete: Arc<AtomicBool>,
}

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        if let Some(cb) = self.on_end.lock().unwrap_or_else(|e| e.into_inner()).take() {
            cb(self.complete.load(Ordering::SeqCst));
        }
    }
}

pin_project_lite::pin_project! {
    /// Sets `complete` when the inner stream yields terminal `None`.
    /// The paired [`FinishOnDrop`] fires on drop, including mid-stream aborts.
    pub struct FinishBody<B> {
        #[pin]
        inner: B,
        complete: Arc<AtomicBool>,
        // Field Drop is allowed; pin-project forbids Drop on this struct itself.
        _guard: FinishOnDrop,
    }
}

impl<B> FinishBody<B> {
    pub fn new(inner: B, on_end: impl FnOnce(bool) + Send + 'static) -> Self {
        let complete = Arc::new(AtomicBool::new(false));
        Self {
            inner,
            complete: complete.clone(),
            _guard: FinishOnDrop {
                on_end: Mutex::new(Some(Box::new(on_end))),
                complete,
            },
        }
    }
}

impl<B> Body for FinishBody<B>
where
    B: Body,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match this.inner.poll_frame(cx) {
            Poll::Ready(None) => {
                this.complete.store(true, Ordering::SeqCst);
                Poll::Ready(None)
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
