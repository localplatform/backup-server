use crate::models::backup_job;
use crate::services::agent_orchestrator;
use crate::state::AppState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

pub struct BackupScheduler {
    scheduler: Mutex<JobScheduler>,
    state: Arc<AppState>,
}

impl BackupScheduler {
    pub async fn new(state: Arc<AppState>) -> anyhow::Result<Self> {
        let scheduler = JobScheduler::new().await?;
        Ok(Self {
            scheduler: Mutex::new(scheduler),
            state,
        })
    }

    pub async fn schedule_job(&self, job_id: &str, cron_expression: &str) -> anyhow::Result<()> {
        let state = self.state.clone();
        let jid = job_id.to_string();

        let job = Job::new_async(cron_expression, move |_uuid, _lock| {
            let state = state.clone();
            let jid = jid.clone();
            Box::pin(async move {
                let db = state.db.clone();
                let jid2 = jid.clone();
                let job_data = tokio::task::spawn_blocking(move || {
                    let conn = db.get()?;
                    backup_job::find_by_id(&conn, &jid2)
                })
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten();

                let Some(job_data) = job_data else { return };
                if job_data.enabled == 0 {
                    return;
                }

                {
                    let running = state.running_jobs.lock().await;
                    if running.contains(&jid) {
                        tracing::warn!(job_id = %jid, "Skipping scheduled run: job already running");
                        return;
                    }
                }

                tracing::info!(job_id = %jid, name = %job_data.name, "Starting scheduled backup");
                if let Err(e) = agent_orchestrator::run_backup_job(state, jid.clone()).await {
                    tracing::error!(job_id = %jid, error = %e, "Scheduled backup failed");
                }
            })
        })?;

        self.scheduler.lock().await.add(job).await?;
        tracing::info!(job_id = %job_id, cron = %cron_expression, "Job scheduled");
        Ok(())
    }

    pub async fn init_schedules(&self) -> anyhow::Result<()> {
        let db = self.state.db.clone();
        let jobs = tokio::task::spawn_blocking(move || {
            let conn = db.get()?;
            backup_job::find_all(&conn)
        })
        .await??;

        let mut count = 0;
        for job in jobs {
            if let Some(cron) = &job.cron_schedule {
                if job.enabled != 0 && !cron.is_empty() {
                    if let Err(e) = self.schedule_job(&job.id, cron).await {
                        tracing::error!(job_id = %job.id, cron = %cron, error = %e, "Failed to schedule job");
                    } else {
                        count += 1;
                    }
                }
            }
        }

        tracing::info!(count, "Cron schedules initialized");
        Ok(())
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        self.scheduler.lock().await.start().await?;
        Ok(())
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.scheduler.lock().await.shutdown().await?;
        Ok(())
    }
}
