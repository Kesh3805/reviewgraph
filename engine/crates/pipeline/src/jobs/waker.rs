//! LISTEN/NOTIFY wake-up (PIPE-002).
//!
//! One dedicated connection listens on `jobs_<queue>` for every subscribed queue and signals a
//! per-queue [`Notify`]. The wake-up is a hint only: correctness never depends on it, because
//! the claim loop also polls. On reconnect every queue is woken, since a notification may have
//! been lost while disconnected.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::model::Queue;

const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);

/// Owns the listener task. Dropping it stops the task.
#[derive(Debug)]
pub struct QueueWaker {
    notifies: HashMap<Queue, Arc<Notify>>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl QueueWaker {
    /// Starts listening; returns immediately (the connection is established in the background).
    pub fn spawn(pool: PgPool, queues: &[Queue]) -> Self {
        let notifies: HashMap<Queue, Arc<Notify>> = queues
            .iter()
            .map(|q| (*q, Arc::new(Notify::new())))
            .collect();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(listen_task(pool, notifies.clone(), cancel.clone()));
        Self {
            notifies,
            cancel,
            task: Some(task),
        }
    }

    /// The signal for `queue` (a fresh, never-signalled one for an unsubscribed queue).
    pub fn notify_for(&self, queue: Queue) -> Arc<Notify> {
        self.notifies
            .get(&queue)
            .cloned()
            .unwrap_or_else(|| Arc::new(Notify::new()))
    }

    /// Stops the listener task and waits for it.
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for QueueWaker {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn wake_all(notifies: &HashMap<Queue, Arc<Notify>>) {
    for n in notifies.values() {
        n.notify_one();
    }
}

/// Sleeps `d`; returns `true` when cancelled first.
async fn sleep_or_cancel(cancel: &CancellationToken, d: Duration) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = tokio::time::sleep(d) => false,
    }
}

async fn listen_task(
    pool: PgPool,
    notifies: HashMap<Queue, Arc<Notify>>,
    cancel: CancellationToken,
) {
    let channels: Vec<(String, Queue)> = notifies.keys().map(|q| (q.channel(), *q)).collect();
    let mut backoff = Duration::from_secs(1);
    'connect: loop {
        let mut listener = match PgListener::connect_with(&pool).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "jobs_listener_connect_failed");
                if sleep_or_cancel(&cancel, backoff).await {
                    return;
                }
                backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                continue 'connect;
            }
        };
        if let Err(e) = listener
            .listen_all(channels.iter().map(|(c, _)| c.as_str()))
            .await
        {
            tracing::warn!(error = %e, "jobs_listener_listen_failed");
            if sleep_or_cancel(&cancel, backoff).await {
                return;
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
            continue 'connect;
        }
        backoff = Duration::from_secs(1);
        // A notification may have been lost before the subscription existed.
        wake_all(&notifies);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                received = listener.try_recv() => match received {
                    Ok(Some(n)) => {
                        if let Some((_, q)) = channels.iter().find(|(c, _)| c == n.channel()) {
                            if let Some(notify) = notifies.get(q) {
                                notify.notify_one();
                            }
                        }
                    }
                    // Connection lost: the next `try_recv` reconnects and re-subscribes.
                    Ok(None) => wake_all(&notifies),
                    Err(e) => {
                        tracing::warn!(error = %e, "jobs_listener_failed");
                        wake_all(&notifies);
                        if sleep_or_cancel(&cancel, backoff).await {
                            return;
                        }
                        backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                        continue 'connect;
                    }
                },
            }
        }
    }
}
