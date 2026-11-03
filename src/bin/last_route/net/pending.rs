//! Background requests for the macroquad frame loop. Godot's `HTTPRequest` node runs its request
//! off the main loop and reports through `request_completed`; here each request runs on a worker
//! thread and the frame loop polls a [`Pending`] once per frame (`poll()` never blocks).

use super::NetError;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// One request (or chain of requests) running on a worker thread.
pub struct Pending<T> {
    rx: Option<Receiver<Result<T, NetError>>>,
    ready: Option<Result<T, NetError>>,
}

impl<T: Send + 'static> Pending<T> {
    /// Runs `work` on a new thread.
    pub fn spawn<F>(work: F) -> Pending<T>
    where
        F: FnOnce() -> Result<T, NetError> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("lr-net".into()).spawn(move || {
            let _ = tx.send(work());
        });
        match spawned {
            Ok(_) => Pending { rx: Some(rx), ready: None },
            Err(e) => Pending::done(Err(NetError::Transport(format!("COULD NOT START REQUEST ({e})")))),
        }
    }
}

impl<T> Pending<T> {
    /// An already-finished result (offline mode, a local refusal): it is delivered on the next poll,
    /// like the original's `call_deferred` emits.
    pub fn done(result: Result<T, NetError>) -> Pending<T> {
        Pending { rx: None, ready: Some(result) }
    }

    /// `Some(result)` exactly once, when the work has finished; `None` while it is still running
    /// and after the result was taken.
    pub fn poll(&mut self) -> Option<Result<T, NetError>> {
        if let Some(r) = self.ready.take() {
            return Some(r);
        }
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(r) => {
                self.rx = None;
                Some(r)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                Some(Err(NetError::Transport("REQUEST THREAD ENDED".into())))
            }
        }
    }

    /// True once the result was taken by `poll`/`wait`.
    pub fn finished(&self) -> bool {
        self.rx.is_none() && self.ready.is_none()
    }

    /// Blocks until the result is in (tests, the quit path).
    pub fn wait(mut self) -> Result<T, NetError> {
        if let Some(r) = self.ready.take() {
            return r;
        }
        match self.rx.take() {
            Some(rx) => rx.recv().unwrap_or_else(|_| Err(NetError::Transport("REQUEST THREAD ENDED".into()))),
            None => Err(NetError::Protocol("RESULT ALREADY TAKEN".into())),
        }
    }

    /// Blocks up to `limit` (the quit path's `wait_for_room_close(1.5)`); `None` on timeout.
    pub fn wait_timeout(&mut self, limit: std::time::Duration) -> Option<Result<T, NetError>> {
        if let Some(r) = self.ready.take() {
            return Some(r);
        }
        let rx = self.rx.as_ref()?;
        match rx.recv_timeout(limit) {
            Ok(r) => {
                self.rx = None;
                Some(r)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.rx = None;
                Some(Err(NetError::Transport("REQUEST THREAD ENDED".into())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn poll_delivers_once() {
        let mut p = Pending::spawn(|| {
            std::thread::sleep(Duration::from_millis(30));
            Ok(7)
        });
        assert!(p.poll().is_none());
        let mut got = None;
        for _ in 0..200 {
            if let Some(r) = p.poll() {
                got = Some(r);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got.unwrap().unwrap(), 7);
        assert!(p.poll().is_none());
        assert!(p.finished());
    }

    #[test]
    fn done_and_wait() {
        let mut p: Pending<i32> = Pending::done(Err(NetError::Offline));
        assert!(matches!(p.poll(), Some(Err(NetError::Offline))));
        assert_eq!(Pending::spawn(|| Ok("x")).wait().unwrap(), "x");
    }
}
