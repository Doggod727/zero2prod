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

    // 三个长期任务：API、发信 worker、幂等键清扫器。
    //
    // 为什么清扫器也做成独立任务，而不是塞进 worker 的循环里：
    //   两者的节奏完全不同 —— worker 是"队列里有活就立刻干"，空队列才睡 10 秒；
    //   清扫器是固定 15 分钟一轮。塞在一起会让任一边的节奏被另一边拖着走，
    //   而且失败后的退避策略也不一样（worker 出错等 1 秒，清扫器出错等一整轮）。
    //
    // 为什么是三个具名变量而不是一个 Vec：
    //   它们的错误类型不同（application 返回 std::io::Error，另两个返回 anyhow::Error），
    //   而 select! 的每个分支要求能各自包进 report_exit。
    //   要统一成 Vec 就得把它们都擦成 Box<dyn Error>，纯粹的复杂度，
    //   换来的只是"少写两个 if"。等任务多到四五个再考虑。
    let mut application_task = tokio::spawn(application.run_until_stopped());
    let mut worker_task = tokio::spawn(run_worker_until_stopped(
        configuration.clone(),
        shutdown_rx.clone(),
    ));
    let mut sweeper_task = tokio::spawn(zero2prod::idempotency::run_sweeper_until_stopped(
        zero2prod::startup::get_connection_pool(&configuration.database),
        shutdown_rx,
    ));

    tokio::select! {
        // 任何一个任务自己退出了（正常结束或 panic），都说明进程该收摊了。
        o = &mut application_task => report_exit("API", o),
        o = &mut worker_task => report_exit("Background worker", o),
        o = &mut sweeper_task => report_exit("Idempotency sweeper", o),
        _ = zero2prod::shutdown::terminate_signal() => {
            tracing::info!("Shutdown signal received, terminating the API gracefully");
        }
    }

    // 收尾顺序：先发停止信号（让 worker / sweeper 在各自的下一个检查点退出），
    // 再停 API —— terminate_handle.stop(true) 会等正在处理的请求做完，这是
    // "优雅"两个字的实际含义；最后逐一 await 三个任务，确保它们的收尾都跑完了
    // 才让 main 返回、进程退出。
    let _ = shutdown_tx.send(true);
    if let Some(handle) = terminate_handle {
        handle.stop(true).await;
    }
    // JoinHandle 完成后可以重复 await。上面 select! 已经报过的那一个，
    // 这里会再拿到一次同样的结果 —— 用 `if let Ok` 过滤掉 Err，
    // 免得同一个 panic 被报两遍。
    if let Ok(outcome) = application_task.await {
        report_exit("API", Ok(outcome));
    }
    if let Ok(outcome) = worker_task.await {
        report_exit("Background worker", Ok(outcome));
    }
    if let Ok(outcome) = sweeper_task.await {
        report_exit("Idempotency sweeper", Ok(outcome));
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
                "{}'s task failed to complete",
                task_name
            )
        }
    }
}
