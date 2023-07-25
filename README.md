# Structured JSON logging from [tracing](https://docs.rs/tracing/) with fields from spans

Unlike the JSON support in [tracing_subscriber](https://docs.rs/tracing_subscriber/), this
implementation treats spans as a way to provide context and adds all fields from all spans to the logged events.

## Examples
```rust
use tracing::{info, info_span};
use tracing_subscriber::prelude::*;
use tracing_json::JsonLayer;
tracing_subscriber::registry().with(JsonLayer::pretty()).init();
let _span = info_span!("A span", span_field = 42).entered();
info!(logged_message_field = "value", "Logged message");
```
Will produce the following output
```json
{
  "log_level": "INFO",
  "logged_message_field": "value",
  "message": "Logged message",
  "name": "event src/main.rs:123",
  "span_field": 42,
  "target": "tracing_json",
  "timestamp": "2023-07-25T09:53:01.790152227Z"
}
```

### Customising timestamps
```rust
use time::macros::format_description;
use tracing::{error, info_span};
use tracing_subscriber::prelude::*;
use tracing_json::JsonLayer;
let timestamp_format = format_description!("[hour]:[minute]:[second].[subsecond digits:1]");
tracing_subscriber::registry().with(JsonLayer::default().with_timestamp_format(timestamp_format)).init();
let _span = info_span!("A span", span_field = 42).entered();
error!(logged_message_field = "value", "Logged message");
```
Will produce the following output
```json
{"log_level":"ERROR","logged_message_field":"value","message":"Logged message","name":"event src/main.rs:123","span_field":42,"target":"tracing_json","timestamp":"10:02:01.9"}
```

## Thanks
* <https://burgers.io/custom-logging-in-rust-using-tracing>
