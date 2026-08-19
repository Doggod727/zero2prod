use tracing::Subscriber;
use tracing::subscriber::set_global_default;
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, Registry};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::fmt::MakeWriter;
/// 将多个层次组合为 ‘tracing'的订阅器
///
/// # 注意
///
/// 将 'impl Subscriber' 作为返回值的类型，以避免写出真实的繁琐的类型
/// 我们需要显示的将类型标记为'Send' 和 'Sync'，以便后续可以将其传递给'init_subscriber'
pub fn get_subscriber<Sink>(name: String, env_filter: String, sink: Sink) -> impl Subscriber + Send + Sync
where
    // 这个奇怪的语法是高阶约束
    // 含义是针对任意的生命周期'a，会实现MakeWriter trait
    Sink: for<'a> MakeWriter<'a> + Send + Sync + 'static
{
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(env_filter));
    let formatting_layer = BunyanFormattingLayer::new(
        name,
        sink,
    );
    Registry::default()
        .with(env_filter)
        .with(JsonStorageLayer)
        .with(formatting_layer)
}

/// 将一个订阅器设置为全局默认值，用于处理所有的跨度数据
///
/// 这个函数只能调用一次
pub fn init_subscriber(subscriber: impl Subscriber + Send + Sync) {
    LogTracer::init().expect("Failed to set logger");
    set_global_default(subscriber).expect("Failed to set subscriber");
}