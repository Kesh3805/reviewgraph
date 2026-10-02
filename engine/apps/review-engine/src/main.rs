fn main() -> anyhow::Result<()> {
    let _telemetry = telemetry::init(telemetry::TelemetryConfig::from_env("review-engine")?)?;
    Ok(())
}
