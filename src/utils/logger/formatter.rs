use time::formatting::Formattable;
use tracing_subscriber::fmt::{
    time::{FormatTime, OffsetTime},
    FormatFields,
};

pub struct CustomFormatter<F> {
    local_time: OffsetTime<F>,
}
impl<F> CustomFormatter<F> {
    pub fn new(local_time: OffsetTime<F>) -> Self {
        Self { local_time }
    }
}

impl<S, N, F> tracing_subscriber::fmt::format::FormatEvent<S, N> for CustomFormatter<F>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::format::FormatFields<'a> + 'static,
    F: Formattable,
{
    fn format_event(
        &self, ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>, event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        // Implement custom formatting logic here
        let metadata = event.metadata();

        // Write timestamp
        // write!(writer, "{} ", fastdate::DateTime::now().format("YYYY-MM-DD hh:mm:ss.000000"))?;
        self.local_time.format_time(&mut writer)?;
        // Write log level
        write!(writer, " {:<5} ", metadata.level())?;

        // Write target (module path)
        write!(writer, "{} ", metadata.target().split("::").next().unwrap_or(""))?;

        // Write the log message
        ctx.format_fields(writer.by_ref(), event)?;

        // Write file and line
        if let Some(file) = metadata.file() {
            if file.starts_with("/") {
                // let relative_path = file.trim_start_matches('src/');
                // write!(writer, "{}:{} ", relative_path, metadata.line().unwrap_or(0))?;
            } else {
                write!(writer, "\n  at {}:{} ", file, metadata.line().unwrap_or(0))?;
            }
        }

        writeln!(writer)
    }
}
