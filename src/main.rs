//! src/main.rs
use std::fmt::{Debug, Display};
use tokio::task::JoinError;
use zero2prod::configurations::get_configurations;
use zero2prod::issue_delivery_worker::run_worker_until_stopped;
use zero2prod::startup::Application;
use zero2prod::telemetry::{get_subscriber, init_subscriber};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = get_subscriber("zero2prod".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);

    // 如果不能读取配置的话，发生panic
    let configuration = get_configurations().expect("Failed to read configurations.");

    let mut application = Application::build(configuration.clone()).await?;
    let terminate_handle = application.terminate_handle();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut application_task = tokio::spawn(application.run_until_stopped());
    let mut worker_task = tokio::spawn(run_worker_until_stopped(configuration, shutdown_rx));

    tokio::select! {
        o = &mut application_task => report_exit("API", o),
        o = &mut worker_task => report_exit("Background worker", o),
        _ = zero2prod::shutdown::terminate_signal() => {
            tracing::info!("Shutdown signal received, terminating the API gracefully");
        }
    }
    let _ = shutdown_tx.send(true);
    if let Some(handle) = terminate_handle {
        handle.stop(true).await;
    }
    if let Ok(outcome) = application_task.await {
        report_exit("API", Ok(outcome));
    }
    if let Ok(outcome ) = worker_task.await {
        report_exit("Background worker", Ok(outcome));
    }
    Ok(())
}

fn report_exit(task_name: &str, outcome: Result<Result<(), impl Debug + Display>, JoinError>) {
    match outcome {
        Ok(Ok(())) => {
            tracing::info!("{} has exited", task_name)
        }
        Ok(Err(e)) => {
            tracing::error!(
            error.cause_chain = ?e,
            error.message = %e,
            "{} failed",
            task_name
            )
        }
        Err(e) => {
            tracing::error!(
            error.cause_chain = ?e,
            error.message = %e,
            "{}' task failed to complete",
            task_name
            )
        }
    }
}
