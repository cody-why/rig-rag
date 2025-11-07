use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, time::OffsetTime},
    util::SubscriberInitExt,
};

use super::{LogConfig, formatter::CustomFormatter};

/// 初始化日志
pub fn init_logger() -> Result<(), Box<dyn std::error::Error>> {
    let config = LogConfig::from_file()?;

    let local_time = OffsetTime::new(
        ::time::UtcOffset::from_hms(8, 0, 0).unwrap(),
        ::time::format_description::parse(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]",
        )
        .unwrap(),
    );

    let env_filter = EnvFilter::new(config.level);
    let to_file = config.to_file;
    let to_stdout = config.to_stdout;
    // let to_opentelemetry = config.to_opentelemetry;

    let stdout_layer = to_stdout.then(|| {
        fmt::layer()
            .with_timer(local_time.clone())
            .pretty()
            .with_writer(std::io::stdout)
    });

    let file_layer = to_file.then(|| {
        let file_appender = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            // .filename_prefix("app")
            .filename_suffix(&config.file_name)
            .max_log_files(180)
            .build(&config.file_path)
            .expect("Init file appender failed");

        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        Box::leak(Box::new(_guard));
        fmt::layer()
            // .with_timer(local_time)
            .event_format(CustomFormatter::new(local_time))
            // .with_ansi(false)
            .with_writer(non_blocking)
    });

    // let otel_logs_layer = to_opentelemetry.then(init_otel_logs_layer);
    // let otel_trace_layer = to_opentelemetry.then(init_otel_traces_layer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        // .with(otel_logs_layer)
        // .with(otel_trace_layer)
        .init();

    Ok(())
}

/// 用于test输出
pub fn init_test_logger() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .pretty()
        .init();
}
