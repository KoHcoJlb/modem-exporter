use std::fmt::Error;
use std::net::SocketAddr;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use prometheus_client::collector::Collector;
use prometheus_client::encoding::{DescriptorEncoder, EncodeLabelSet, EncodeLabelValue};
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::MetricType;
use prometheus_client::registry::{Registry, Unit};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use warp::Filter;

#[derive(Debug, Serialize)]
#[serde(rename = "request")]
struct ModemRequest<T>(T);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ModemResponse<T = ()> {
    Response(T),
    Error {
        code: i32,
        message: String,
    },
}

impl<T> ModemResponse<T> {
    fn ok(self) -> Result<T> {
        match self {
            ModemResponse::Response(val) => Ok(val),
            ModemResponse::Error { code, message } =>
                Err(anyhow!("api error: code={code} message={message}"))
        }
    }
}

#[derive(Debug, Deserialize)]
struct SessionResponse {
    #[serde(rename = "SesInfo")]
    session: String,
    #[serde(rename = "TokInfo")]
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TrafficStatistics {
    current_upload: u64,
    current_download: u64,
    current_connect_time: u64,
    total_upload: u64,
    total_download: u64,
    total_connect_time: u64,
}

impl Collector for TrafficStatistics {
    #[allow(non_camel_case_types)]
    fn encode(&self, mut encoder: DescriptorEncoder) -> std::result::Result<(), Error> {
        #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
        enum period {
            session,
            total,
        }
        use period::*;

        {
            #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
            enum direction {
                upload,
                download,
            }
            use direction::*;

            #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
            struct labels {
                period: period,
                direction: direction,
            }

            let mut transferred = encoder.encode_descriptor(
                "modem_transferred", "Transferred bytes",
                Some(&Unit::Bytes), MetricType::Gauge,
            )?;

            transferred.encode_family(&labels {
                period: session,
                direction: upload,
            })?.encode_counter::<(), _, u64>(&self.current_upload, None)?;
            transferred.encode_family(&labels {
                period: session,
                direction: download,
            })?.encode_counter::<(), _, u64>(&self.current_download, None)?;

            transferred.encode_family(&labels {
                period: total,
                direction: upload,
            })?.encode_counter::<(), _, u64>(&self.total_upload, None)?;
            transferred.encode_family(&labels {
                period: total,
                direction: download,
            })?.encode_counter::<(), _, u64>(&self.total_download, None)?;
        }

        {
            #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
            struct labels {
                period: period,
            }

            let mut duration = encoder.encode_descriptor(
                "modem_connect_duration", "Connected duration",
                Some(&Unit::Seconds), MetricType::Counter,
            )?;

            duration.encode_family(&labels { period: session })?
                .encode_counter::<(), _, u64>(&self.current_connect_time, None)?;
            duration.encode_family(&labels { period: total })?
                .encode_counter::<(), _, u64>(&self.total_connect_time, None)?;
        }

        Ok(())
    }
}

pub struct Modem {
    client: Client,
    session: Option<SessionResponse>,
}

impl Modem {
    pub fn new() -> Result<Modem> {
        Ok(Self {
            client: Client::new(),
            session: None,
        })
    }

    async fn get<Resp: DeserializeOwned>(&self, path: &str) -> Result<Resp> {
        let mut builder = self.client.get(format!("http://192.168.8.1{path}"));
        if let Some(session) = &self.session {
            builder = builder.header("Cookie", &session.session)
                .header("__RequestVerificationToken", &session.token);
        }
        let resp = builder.send().await?.error_for_status()?;
        let data = resp.bytes().await?;
        Ok(quick_xml::de::from_reader(data.as_ref())?)
    }

    async fn post<Req: Serialize, Resp: DeserializeOwned>(&self, path: &str, req: Req) -> Result<Resp> {
        let mut builder = self.client.post(format!("http://192.168.8.1{path}"));
        if let Some(session) = &self.session {
            builder = builder.header("Cookie", &session.session)
                .header("__RequestVerificationToken", &session.token);
        }
        let resp = builder.body(quick_xml::se::to_string(&req).context("serialize body")?)
            .send().await?
            .error_for_status()?;
        let data = resp.text().await?;
        Ok(quick_xml::de::from_reader(data.as_bytes()).context("deserialize response")?)
    }

    async fn gather_statistics(&mut self) -> Result<TrafficStatistics> {
        self.session = self.get("/api/webserver/SesTokInfo").await.context("get session")?;

        let traffic_stats = self.get::<ModemResponse<TrafficStatistics>>("/api/monitoring/traffic-statistics")
            .await?
            .ok()?;
        Ok(traffic_stats)
    }
}

async fn gather_metrics() -> Result<String> {
    let mut modem = Modem::new()?;
    let stats = modem.gather_statistics().await?;

    let mut registry = Registry::default();
    registry.register_collector(Box::new(stats));

    let mut data = String::new();
    encode(&mut data, &registry).context("failed to encode")?;
    Ok(data)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let metrics_route = warp::path!("metrics")
        .then(move || {
            async {
                gather_metrics().await.unwrap_or_else(|err| format!("{err:?}"))
            }
        });

    warp::serve(metrics_route)
        .run(SocketAddr::from_str("0.0.0.0:9091").unwrap())
        .await;

    Ok(())
}
