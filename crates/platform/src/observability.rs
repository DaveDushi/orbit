use tracing_subscriber::EnvFilter;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TracingFormat {
    Compact,
    Json,
}

/// Installs local logging on stdout without configuring any external telemetry exporter.
pub fn init_tracing(format: TracingFormat) -> Result<(), tracing_subscriber::util::TryInitError> {
    init(format, false)
}

/// [`init_tracing`] on stderr, for commands whose stdout is data for scripts.
pub fn init_tracing_on_stderr(
    format: TracingFormat,
) -> Result<(), tracing_subscriber::util::TryInitError> {
    init(format, true)
}

fn init(format: TracingFormat, stderr: bool) -> Result<(), tracing_subscriber::util::TryInitError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match (format, stderr) {
        (TracingFormat::Compact, false) => builder.compact().finish().try_init(),
        (TracingFormat::Compact, true) => builder
            .compact()
            .with_writer(std::io::stderr)
            .finish()
            .try_init(),
        (TracingFormat::Json, false) => builder.json().finish().try_init(),
        (TracingFormat::Json, true) => builder
            .json()
            .with_writer(std::io::stderr)
            .finish()
            .try_init(),
    }
}
