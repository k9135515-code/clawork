use anyhow::Context;
use chrono::Utc;
use clawork_core::TaskInfo;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::error;

#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Heartbeat {
        at: chrono::DateTime<chrono::Utc>,
    },
    Suggestion {
        at: chrono::DateTime<chrono::Utc>,
        text: String,
    },
    DailyBriefingTick {
        at: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DaemonSnapshot {
    pub running: bool,
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone)]
pub struct DaemonService {
    snapshot: Arc<RwLock<DaemonSnapshot>>,
    tx: broadcast::Sender<DaemonEvent>,
    tasks: Arc<RwLock<Vec<TaskInfo>>>,
    heartbeat_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    scheduler: Arc<RwLock<Option<JobScheduler>>>,
}

impl Default for DaemonService {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            snapshot: Arc::new(RwLock::new(DaemonSnapshot::default())),
            tx,
            tasks: Arc::new(RwLock::new(Vec::new())),
            heartbeat_handle: Arc::new(RwLock::new(None)),
            scheduler: Arc::new(RwLock::new(None)),
        }
    }
}

impl DaemonService {
    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> DaemonSnapshot {
        self.snapshot.read().clone()
    }

    pub fn list_tasks(&self) -> Vec<TaskInfo> {
        self.tasks.read().clone()
    }

    pub async fn start(&self, heartbeat_interval: Duration) -> anyhow::Result<()> {
        {
            let mut snap = self.snapshot.write();
            if snap.running {
                return Ok(());
            }
            snap.running = true;
        }

        let tx = self.tx.clone();
        let snap = Arc::clone(&self.snapshot);
        let handle = tokio::spawn(async move {
            loop {
                let at = Utc::now();
                {
                    let mut state = snap.write();
                    if !state.running {
                        break;
                    }
                    state.last_heartbeat = Some(at);
                }

                let _ = tx.send(DaemonEvent::Heartbeat { at });
                let _ = tx.send(DaemonEvent::Suggestion {
                    at,
                    text: "Review open tasks and grant least privilege only when needed."
                        .to_string(),
                });

                sleep(heartbeat_interval).await;
            }
        });
        *self.heartbeat_handle.write() = Some(handle);

        let scheduler_result = async {
            let scheduler = JobScheduler::new().await.context("create scheduler")?;
            let tx_for_job = self.tx.clone();
            let job = Job::new_async("0 */30 * * * *", move |_id, _lock| {
                let tx = tx_for_job.clone();
                Box::pin(async move {
                    let _ = tx.send(DaemonEvent::Suggestion {
                        at: Utc::now(),
                        text: "Daily briefing data refresh checkpoint reached.".into(),
                    });
                })
            })?;
            scheduler.add(job).await?;
            let tx_for_briefing = self.tx.clone();
            let daily_job = Job::new_async("0 0 8 * * *", move |_id, _lock| {
                let tx = tx_for_briefing.clone();
                Box::pin(async move {
                    let _ = tx.send(DaemonEvent::DailyBriefingTick { at: Utc::now() });
                })
            })?;
            scheduler.add(daily_job).await?;
            scheduler.start().await?;
            *self.scheduler.write() = Some(scheduler);
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = scheduler_result {
            self.stop().await;
            return Err(err);
        }

        let mut tasks = self.tasks.write();
        tasks.clear();
        tasks.extend([
            TaskInfo {
                id: "heartbeat".into(),
                label: "Heartbeat monitor".into(),
                next_run: Some(
                    Utc::now() + chrono::Duration::seconds(heartbeat_interval.as_secs() as i64),
                ),
            },
            TaskInfo {
                id: "cron:briefing-refresh".into(),
                label: "Refresh briefing cache every 30 minutes".into(),
                next_run: None,
            },
            TaskInfo {
                id: "cron:daily-briefing".into(),
                label: "Emit daily briefing tick at 08:00".into(),
                next_run: None,
            },
        ]);

        Ok(())
    }

    pub async fn stop(&self) {
        {
            let mut snap = self.snapshot.write();
            snap.running = false;
        }
        let handle = { self.heartbeat_handle.write().take() };
        if let Some(handle) = handle {
            handle.abort();
            if let Err(err) = handle.await {
                error!("failed to abort heartbeat task: {err}");
            }
        }

        let scheduler = { self.scheduler.write().take() };
        if let Some(mut scheduler) = scheduler {
            if let Err(err) = scheduler.shutdown().await {
                error!("failed to shutdown cron scheduler: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DaemonService;
    use tokio::time::Duration;

    #[tokio::test]
    async fn daemon_can_restart_after_stop() {
        let daemon = DaemonService::default();
        daemon
            .start(Duration::from_millis(10))
            .await
            .expect("first start");
        daemon.stop().await;
        daemon
            .start(Duration::from_millis(10))
            .await
            .expect("second start");
        daemon.stop().await;
    }
}
