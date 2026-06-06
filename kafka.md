# Kafka integration — core_api

`core_api` acts as the **weather processor** in the Kafka pipeline. It consumes weather
requests, fetches live data from wttr.in, and publishes results back.

---

## Message flow

```
weather.requests  ──►  core_api  ──►  wttr.in API
                                           │
weather.results   ◄────────────────────────┘
```

## Topics

| Topic              | Role     | Action                                      |
|--------------------|----------|---------------------------------------------|
| `weather.requests` | Consumer | Receives `{action:"get_weather", client_id}` |
| `weather.results`  | Producer | Sends `{action:"weather_result", client_id, payload:{...}}` |

## Envelope

```json
{
  "action":    "weather_result",
  "client_id": "550e8400-e29b-41d4-a716-446655440000",
  "payload":   { ...wttr.in JSON response... }
}
```

`client_id` is passed through unchanged from the request envelope so `interaction_api`
can route the result to the correct WebSocket connection.

## Consumer group

The consumer uses group id `core-api-weather`. Kafka assigns each partition to one
consumer in the group — with 2 `core_api` pods running, only one will process each
message. This is correct: you don't want both pods fetching weather for the same request.

Reference: [Consumer groups](https://www.confluent.io/blog/kafka-consumer-group-protocol/)

## Configuration

`KAFKA_BROKERS` environment variable controls the broker address. Set in the k8s
`Deployment` manifest; defaults to `localhost:9092` for local development.

## Library: rdkafka

[rdkafka](https://docs.rs/rdkafka) is the standard Rust Kafka client. It wraps
[librdkafka](https://github.com/confluentinc/librdkafka) (the C library from Confluent)
which is the most battle-tested Kafka client implementation across all languages.

The `cmake-build` feature compiles librdkafka from source during `cargo build` — no
system library installation required, at the cost of a longer first build.

Reference: [rdkafka docs](https://docs.rs/rdkafka/latest/rdkafka/)
