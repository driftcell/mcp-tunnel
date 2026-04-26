use std::collections::HashMap;

use anyhow::Context;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use tonic::metadata::{MetadataKey, MetadataValue};

pub struct OtelGuard {
    trace_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    tracer: Option<SdkTracer>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.trace_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.meter_provider.take() {
            let _ = provider.shutdown();
        }
    }
}

impl OtelGuard {
    pub fn tracer(&self) -> Option<SdkTracer> {
        self.tracer.clone()
    }
}

pub fn init_otel(config: &crate::config::OtelConfig) -> anyhow::Result<Option<OtelGuard>> {
    let endpoint = match &config.endpoint {
        Some(ep) => ep.clone(),
        None => return Ok(None),
    };

    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attribute(opentelemetry::KeyValue::new(
            "service.version",
            env!("CARGO_PKG_VERSION"),
        ))
        .build();

    let mut trace_provider = None;
    let mut meter_provider = None;
    let mut tracer = None;

    if config.traces_enabled {
        let provider = if config.protocol == "http/protobuf" || config.protocol.starts_with("http")
        {
            let mut exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&endpoint);

            if let Some(hdrs) = &config.headers {
                let mut map = HashMap::new();
                for h in hdrs {
                    map.insert(h.key.clone(), h.value.clone());
                }
                exporter = exporter.with_headers(map);
            }

            let exporter = exporter
                .build()
                .context("failed to build HTTP trace exporter")?;

            SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource.clone())
                .with_sampler(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(
                    config.sample_rate,
                ))
                .build()
        } else {
            let mut exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&endpoint);

            if let Some(hdrs) = &config.headers {
                let mut metadata = tonic::metadata::MetadataMap::new();
                for h in hdrs {
                    let key: MetadataKey<_> = h
                        .key
                        .parse()
                        .context(format!("invalid metadata key: {}", h.key))?;
                    let value: MetadataValue<_> = h
                        .value
                        .parse()
                        .context(format!("invalid metadata value: {}", h.value))?;
                    metadata.insert(key, value);
                }
                exporter = exporter.with_metadata(metadata);
            }

            let exporter = exporter
                .build()
                .context("failed to build gRPC trace exporter")?;

            SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource.clone())
                .with_sampler(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(
                    config.sample_rate,
                ))
                .build()
        };

        tracer = Some(provider.tracer("mcp-tunnel"));
        global::set_tracer_provider(provider.clone());
        trace_provider = Some(provider);
    }

    if config.metrics_enabled {
        let provider = if config.protocol == "http/protobuf" || config.protocol.starts_with("http")
        {
            let mut exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(&endpoint);

            if let Some(hdrs) = &config.headers {
                let mut map = HashMap::new();
                for h in hdrs {
                    map.insert(h.key.clone(), h.value.clone());
                }
                exporter = exporter.with_headers(map);
            }

            let exporter = exporter
                .build()
                .context("failed to build HTTP metrics exporter")?;

            SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(resource.clone())
                .build()
        } else {
            let mut exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(&endpoint);

            if let Some(hdrs) = &config.headers {
                let mut metadata = tonic::metadata::MetadataMap::new();
                for h in hdrs {
                    let key: MetadataKey<_> = h
                        .key
                        .parse()
                        .context(format!("invalid metadata key: {}", h.key))?;
                    let value: MetadataValue<_> = h
                        .value
                        .parse()
                        .context(format!("invalid metadata value: {}", h.value))?;
                    metadata.insert(key, value);
                }
                exporter = exporter.with_metadata(metadata);
            }

            let exporter = exporter
                .build()
                .context("failed to build gRPC metrics exporter")?;

            SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(resource.clone())
                .build()
        };

        global::set_meter_provider(provider.clone());
        meter_provider = Some(provider);

        super::metrics::init_meter();
    }

    Ok(Some(OtelGuard {
        trace_provider,
        meter_provider,
        tracer,
    }))
}
