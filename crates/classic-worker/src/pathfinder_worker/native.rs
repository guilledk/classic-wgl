//! Host-owned A* pathfinding worker (native backend).
//!
//! Owns an `Arc<NavSnapshot>` and runs `classic_core::pathfinder::find_path`
//! on a dedicated `std::thread`, so the render thread can submit a request and
//! poll for the result without blocking mid-frame.  A synchronous fallback
//! ([`PathfinderWorker::find_path_sync`]) runs the same search inline against
//! the latest snapshot, for the deterministic test harness.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use classic_core::pathfinder::{GridCell, NavSnapshot, PathPoll};

use super::PathId;

enum Command {
    Find { id: PathId, from: GridCell, to: GridCell },
    SetSnapshot(Arc<NavSnapshot>),
    Flush(mpsc::Sender<()>),
    Shutdown,
}

/// Native pathfinding worker: a dedicated thread + `mpsc` request/result pair.
pub struct PathfinderWorker {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<(PathId, Option<Vec<GridCell>>)>,
    results: HashMap<PathId, PathPoll>,
    snapshot: Arc<NavSnapshot>,
}

impl PathfinderWorker {
    /// Spawn the worker thread over `snapshot`.
    pub fn new(snapshot: Arc<NavSnapshot>) -> Self {
        let (tx, worker_rx) = mpsc::channel::<Command>();
        let (worker_tx, rx) = mpsc::channel::<(PathId, Option<Vec<GridCell>>)>();
        let worker_snapshot = Arc::clone(&snapshot);

        thread::spawn(move || {
            let mut snapshot = worker_snapshot;
            while let Ok(command) = worker_rx.recv() {
                match command {
                    Command::Find { id, from, to } => {
                        let result = snapshot.find_path(from, to);
                        let _ = worker_tx.send((id, result));
                    }
                    Command::SetSnapshot(next) => snapshot = next,
                    Command::Flush(ack) => {
                        let _ = ack.send(());
                    }
                    Command::Shutdown => break,
                }
            }
        });

        Self { tx, rx, results: HashMap::new(), snapshot }
    }

    /// Replace the nav snapshot both workers and the sync fallback search
    /// against.  In-flight requests keep searching their original snapshot.
    pub fn set_snapshot(&mut self, snapshot: Arc<NavSnapshot>) {
        self.snapshot = Arc::clone(&snapshot);
        let _ = self.tx.send(Command::SetSnapshot(snapshot));
    }

    /// The latest snapshot shared with the worker.
    pub fn snapshot(&self) -> &Arc<NavSnapshot> {
        &self.snapshot
    }

    /// Submit a path request under a caller-chosen `id`.  Non-blocking.
    pub fn request_path(&mut self, id: PathId, from: GridCell, to: GridCell) {
        let _ = self.tx.send(Command::Find { id, from, to });
    }

    /// Poll a previously submitted request.  Non-blocking; returns
    /// [`PathPoll::Pending`] until the worker has delivered a result.
    pub fn poll_path(&mut self, id: PathId) -> PathPoll {
        self.drain_results();
        self.results.remove(&id).unwrap_or(PathPoll::Pending)
    }

    /// Synchronous fallback: run A* inline against the latest snapshot.
    pub fn find_path_sync(&self, from: GridCell, to: GridCell) -> Option<Vec<GridCell>> {
        self.snapshot.find_path(from, to)
    }

    /// Block until every previously submitted request has been processed.
    ///
    /// The flush ack rides the same FIFO `mpsc` channel as `Find` requests, so
    /// receiving it guarantees all earlier searches have completed.  Used as
    /// the determinism barrier at frame boundaries.
    pub fn join(&self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        let _ = self.tx.send(Command::Flush(ack_tx));
        let _ = ack_rx.recv();
    }

    fn drain_results(&mut self) {
        while let Ok((id, result)) = self.rx.try_recv() {
            let poll = match result {
                Some(path) => PathPoll::Path(path),
                None => PathPoll::NoPath,
            };
            self.results.insert(id, poll);
        }
    }
}

impl Drop for PathfinderWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn open_snapshot(w: i32, h: i32) -> Arc<NavSnapshot> {
        Arc::new(NavSnapshot::new(w, h, vec![1; (w * h) as usize]))
    }

    fn poll_until(worker: &mut PathfinderWorker, id: PathId) -> PathPoll {
        for _ in 0..1000 {
            match worker.poll_path(id) {
                PathPoll::Pending => thread::sleep(Duration::from_millis(1)),
                result => return result,
            }
        }
        PathPoll::Pending
    }

    #[test]
    fn finds_a_path_async() {
        let mut worker = PathfinderWorker::new(open_snapshot(5, 5));
        worker.request_path(0, (0, 0), (4, 4));
        match poll_until(&mut worker, 0) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(4, 4)));
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn reports_no_path() {
        let mut grid = vec![1_i32; 9];
        grid[1] = 0;
        grid[4] = 0;
        grid[7] = 0;
        let mut worker = PathfinderWorker::new(Arc::new(NavSnapshot::new(3, 3, grid)));
        worker.request_path(7, (0, 0), (2, 0));
        assert_eq!(poll_until(&mut worker, 7), PathPoll::NoPath);
    }

    #[test]
    fn sync_fallback_matches_async() {
        let mut worker = PathfinderWorker::new(open_snapshot(8, 8));
        worker.request_path(3, (1, 1), (6, 5));
        let sync = worker.find_path_sync((1, 1), (6, 5)).unwrap();
        match poll_until(&mut worker, 3) {
            PathPoll::Path(path) => assert_eq!(path, sync),
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_update_repins_search() {
        // A blocked centre cell forces the route around it.
        let mut grid = vec![1_i32; 9];
        grid[4] = 0;
        let mut worker = PathfinderWorker::new(Arc::new(NavSnapshot::new(3, 3, grid)));
        worker.request_path(1, (0, 0), (2, 2));
        match poll_until(&mut worker, 1) {
            PathPoll::Path(path) => {
                assert_eq!(path.first(), Some(&(0, 0)));
                assert_eq!(path.last(), Some(&(2, 2)));
                assert!(!path.contains(&(1, 1)), "must route around the blocked centre");
            }
            other => panic!("expected a path around the wall, got {other:?}"),
        }

        // Clear the wall; the same query now takes the straight diagonal.
        worker.set_snapshot(open_snapshot(3, 3));
        worker.request_path(2, (0, 0), (2, 2));
        assert_eq!(poll_until(&mut worker, 2), PathPoll::Path(vec![(0, 0), (1, 1), (2, 2)]));
    }

    #[test]
    fn join_barrier_waits_for_inflight() {
        let mut worker = PathfinderWorker::new(open_snapshot(64, 64));
        worker.request_path(0, (0, 0), (63, 63));
        worker.join();
        // After the barrier, the result must already be buffered (no Pending).
        assert_ne!(worker.poll_path(0), PathPoll::Pending);
    }
}
