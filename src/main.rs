use axum::{routing::get, Json, Router};
use futures::StreamExt;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const TOPIC_REQUESTS: &str = "weather.requests";
const TOPIC_RESULTS: &str = "weather.results";

#[derive(Serialize, Deserialize)]
struct Envelope {
    action: String,
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
}

async fn get_version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn fetch_weather() -> Option<serde_json::Value> {
    reqwest::get("https://wttr.in/Limassol?format=j1")
        .await
        .ok()?
        .json()
        .await
        .ok()
}

fn start_weather_processor(producer: FutureProducer, brokers: String) {
    tokio::spawn(async move {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &brokers)
            .set("group.id", "core-api-weather")
            .set("auto.offset.reset", "latest")
            .create()
            .expect("failed to create kafka consumer");

        consumer
            .subscribe(&[TOPIC_REQUESTS])
            .expect("failed to subscribe");

        let mut stream = consumer.stream();
        while let Some(Ok(msg)) = stream.next().await {
            let Some(Ok(raw)) = msg.payload_view::<str>() else {
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Envelope>(raw) else {
                continue;
            };
            if envelope.action != "get_weather" {
                continue;
            }

            let weather = fetch_weather().await;
            let result = Envelope {
                action: "weather_result".into(),
                client_id: envelope.client_id.clone(),
                payload: weather,
            };
            if let Ok(json) = serde_json::to_string(&result) {
                let _ = producer
                    .send(
                        FutureRecord::to(TOPIC_RESULTS)
                            .payload(&json)
                            .key(&envelope.client_id),
                        Duration::from_secs(5),
                    )
                    .await;
            }
        }
    });
}

#[tokio::main]
async fn main() {
    let brokers = std::env::var("KAFKA_BROKERS")
        .unwrap_or_else(|_| "localhost:9092".into());

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .create()
        .expect("failed to create kafka producer");

    start_weather_processor(producer, brokers);

    let app = Router::new().route("/version", get(get_version));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
