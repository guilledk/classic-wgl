//! A small `Send` thread pool.
//!
//! The generic machinery underpinning Tiers 1–3 of the multithreading plan:
//! pathfinding, terrain kernels, and arbitrary guest background work all submit
//! fire-and-forget jobs to a pool.  Native-only for now (`std::thread` +
//! `mpsc`); the web backend uses a `Worker` and does not link this module.

use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

enum Message {
    Job(Job),
    Shutdown,
}

/// A fixed-size pool of worker threads pulling jobs from a shared queue.
///
/// Workers share a single `mpsc::Receiver` behind a mutex: a worker blocks on
/// `recv` while holding the lock, so only one worker waits for the next job at
/// a time.  This serializes wake-ups but is correct and simple; the engine
/// submits at most a handful of jobs per frame, so it is never a hot path.
pub struct ThreadPool {
    sender: mpsc::Sender<Message>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ThreadPool {
    /// Spawn `threads` workers (clamped to at least one).
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        let (sender, receiver) = mpsc::channel::<Message>();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(threads);
        for _ in 0..threads {
            let rx = Arc::clone(&receiver);
            workers.push(thread::spawn(move || loop {
                let message = rx.lock().expect("worker pool mutex poisoned").recv();
                match message {
                    Ok(Message::Job(job)) => job(),
                    Ok(Message::Shutdown) | Err(_) => break,
                }
            }));
        }

        Self { sender, workers }
    }

    /// Queue a fire-and-forget job.  Returns immediately; the job runs on some
    /// worker thread.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let _ = self.sender.send(Message::Job(Box::new(f)));
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        for _ in &self.workers {
            let _ = self.sender.send(Message::Shutdown);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn runs_jobs_on_worker_threads() {
        let pool = ThreadPool::new(4);
        let (tx, rx) = mpsc::channel();
        for i in 0..10 {
            let tx = tx.clone();
            pool.spawn(move || tx.send(i).unwrap());
        }
        drop(tx);

        let mut got: Vec<i32> = rx.iter().collect();
        got.sort_unstable();
        assert_eq!(got, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn shutdown_joins_workers() {
        let pool = ThreadPool::new(2);
        let (tx, rx) = mpsc::channel();
        pool.spawn(move || tx.send(42).unwrap());
        assert_eq!(rx.recv().unwrap(), 42);
        drop(pool);
    }
}
