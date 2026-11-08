use std::sync::{Arc, Mutex};

use rune::{Tx, Waker};

type Job<S> = Box<dyn FnOnce(&mut S, &mut Tx) + Send>;

struct Shared<S> {
    jobs: Vec<Job<S>>,
    waker: Option<Waker>,
    /// Workers `spawn` started that have not sent their result yet.
    running: usize,
}

/// Work for one window's state from other threads and other windows. A sender queues a closure
/// and wakes the window; the window's `poll` runs the closures on the UI thread with
/// `&mut S`, in the order they were sent. Clones share the queue, so each window's state keeps
/// one and hands clones to whoever reports back to it.
///
/// Rune's `AppHandle::update` does the same for an `App`'s main window. Sniplet uses this
/// instead so every window, and every headless test, drains its work the same way.
pub struct Inbox<S> {
    shared: Arc<Mutex<Shared<S>>>,
}

impl<S> Clone for Inbox<S> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<S> Default for Inbox<S> {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                jobs: Vec::new(),
                waker: None,
                running: 0,
            })),
        }
    }
}

impl<S: 'static> Inbox<S> {
    /// Called from the window's `AppSpec::setup` with the waker that wakes its event loop.
    pub fn set_waker(&self, waker: Waker) {
        let pending = {
            let mut shared = self.shared.lock().unwrap();
            shared.waker = Some(waker.clone());
            !shared.jobs.is_empty()
        };
        if pending {
            waker();
        }
    }

    /// Run `f` with the window's state on its UI thread, soon.
    pub fn send(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static) {
        self.queue(Box::new(f), false);
    }

    fn queue(&self, job: Job<S>, finished: bool) {
        let waker = {
            let mut shared = self.shared.lock().unwrap();
            shared.jobs.push(job);
            shared.running -= usize::from(finished);
            shared.waker.clone()
        };
        if let Some(waker) = waker {
            waker();
        }
    }

    /// Run `work` on a new thread, then `done` with its result on the UI thread.
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut S, T, &mut Tx) + Send + 'static,
    ) {
        let inbox = self.clone();
        self.shared.lock().unwrap().running += 1;
        std::thread::spawn(move || {
            let result = work();
            inbox.queue(Box::new(move |state, tx| done(state, result, tx)), true);
        });
    }

    /// Run every queued closure. True when any ran, so `poll` can report a change.
    pub fn drain(state: &mut S, inbox: impl Fn(&S) -> &Self, tx: &mut Tx) -> bool {
        let mut ran = false;
        loop {
            let jobs = std::mem::take(&mut inbox(state).shared.lock().unwrap().jobs);
            if jobs.is_empty() {
                return ran;
            }
            for job in jobs {
                job(state, tx);
            }
            ran = true;
        }
    }

    /// Whether nothing is queued and no worker is running. Tests wait for this after a
    /// background job.
    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        let shared = self.shared.lock().unwrap();
        shared.jobs.is_empty() && shared.running == 0
    }
}
