//! Structured JSON logging from [`tracing`] with fields from spans
//!
//! Unlike the JSON support in [`tracing_subscriber`], this
//! implementation treats spans as a way to provide context and adds all fields from all spans to the logged events.
//!
//! ## Examples
//! ```
//! use tracing::{info, info_span};
//! use tracing_subscriber::prelude::*;
//! use tracing_json_span_fields::JsonLayer;
//! tracing_subscriber::registry().with(JsonLayer::pretty()).init();
//! let _span = info_span!("A span", span_field = 42).entered();
//! info!(logged_message_field = "value", "Logged message");
//! ```
//!
//! Will produce the following output
//!
//! ```json
//! {
//!   "log_level": "INFO",
//!   "logged_message_field": "value",
//!   "message": "Logged message",
//!   "name": "event src/main.rs:123",
//!   "span_field": 42,
//!   "target": "tracing_json",
//!   "timestamp": "2023-07-25T09:53:01.790152227Z"
//! }
//! ```
//!
//! ### Customising timestamps
//!
//! ```
//! use time::macros::format_description;
//! use tracing::{error, info_span};
//! use tracing::level_filters::LevelFilter;
//! use tracing_subscriber::prelude::*;
//! use tracing_json_span_fields::JsonLayer;
//! let timestamp_format = format_description!("[hour]:[minute]:[second].[subsecond digits:1]");
//! tracing_subscriber::registry().with(JsonLayer::default().with_timestamp_format(timestamp_format).with_level(LevelFilter::ERROR)).init();
//! let _span = info_span!("A span", span_field = 42).entered();
//! error!(logged_message_field = "value", "Logged message");
//! ```
//!
//! Will produce the following output
//!
//! ```json
//! {"log_level":"ERROR","logged_message_field":"value","message":"Logged message","name":"event src/main.rs:123","target":"tracing_json","timestamp":"10:02:01.9"}
//! ```
//!
//! ## Thanks
//!
//! * <https://burgers.io/custom-logging-in-rust-using-tracing>

use serde_json::{Map, Value};
use time::format_description::well_known::Iso8601;
use time::formatting::Formattable;
use time::OffsetDateTime;
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Metadata, Subscriber};
use tracing_subscriber::layer;
use tracing_subscriber::layer::Context;
#[allow(unused_imports)]
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug)]
struct CustomFieldStorage(Map<String, Value>);

/// Something that can be used to write output from a [`JsonLayer`].
///
/// Primarily intended to allow custom outputs in unit testing.
pub trait JsonOutput {
    fn write(&self, value: Value);
}

/// Default [`JsonOutput`] writing to stdout.
#[derive(Default)]
pub struct JsonStdout {
    pretty: bool,
}

impl JsonOutput for JsonStdout {
    fn write(&self, value: Value) {
        println!(
            "{}",
            if self.pretty {
                serde_json::to_string_pretty(&value).unwrap()
            } else {
                serde_json::to_string(&value).unwrap()
            }
        );
    }
}

/// An implementation of a [`tracing_subscriber::Layer`] that writes events as JSON using a
/// [`JsonOutput`].
pub struct JsonLayer<O = JsonStdout, F = Iso8601> {
    output: O,
    timestamp_format: F,
    max_level: LevelFilter,
}

impl Default for JsonLayer {
    fn default() -> Self {
        JsonLayer {
            output: JsonStdout::default(),
            timestamp_format: Iso8601::DEFAULT,
            max_level: LevelFilter::INFO,
        }
    }
}

impl JsonLayer<JsonStdout, Iso8601> {
    pub fn pretty() -> JsonLayer<JsonStdout, Iso8601> {
        JsonLayer::default().with_output(JsonStdout { pretty: true })
    }
}

impl<O, F> JsonLayer<O, F>
where
    F: Formattable,
    O: JsonOutput,
{
    pub fn with_output<O2>(self, output: O2) -> JsonLayer<O2, F>
    where
        O2: JsonOutput,
    {
        JsonLayer {
            output,
            timestamp_format: self.timestamp_format,
            max_level: self.max_level,
        }
    }

    pub fn with_timestamp_format<F2>(self, timestamp_format: F2) -> JsonLayer<O, F2>
    where
        F2: Formattable,
    {
        JsonLayer {
            output: self.output,
            timestamp_format,
            max_level: self.max_level,
        }
    }

    pub fn with_level(self, max_level: LevelFilter) -> JsonLayer<O, F> {
        JsonLayer {
            output: self.output,
            timestamp_format: self.timestamp_format,
            max_level,
        }
    }
}

impl<S, O, F> layer::Layer<S> for JsonLayer<O, F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    O: JsonOutput + 'static,
    F: Formattable + 'static,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        metadata.level() <= &self.max_level
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        // Build our json object from the field values like we have been
        let mut fields = Map::new();
        let mut visitor = JsonVisitor(&mut fields);
        attrs.record(&mut visitor);

        // And stuff it in our newtype.
        let storage = CustomFieldStorage(fields);

        // Get a reference to the internal span data
        let span = ctx.span(id).unwrap();
        // Get the special place where tracing stores custom data
        let mut extensions = span.extensions_mut();
        // And store our data
        extensions.insert::<CustomFieldStorage>(storage);
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(self.max_level)
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        // Get the span whose data is being recorded
        let span = ctx.span(id).unwrap();

        // Get a mutable reference to the data we created in new_span
        let mut extensions_mut = span.extensions_mut();
        let custom_field_storage: &mut CustomFieldStorage =
            extensions_mut.get_mut::<CustomFieldStorage>().unwrap();
        let json_data: &mut Map<String, Value> = &mut custom_field_storage.0;

        // And add to using our old friend the visitor!
        let mut visitor = JsonVisitor(json_data);
        values.record(&mut visitor);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Map::new();

        // The fields of the spans
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                let extensions = span.extensions();
                let storage = extensions.get::<CustomFieldStorage>().unwrap();
                let field_data: &Map<String, Value> = &storage.0;

                for (key, value) in field_data {
                    fields.insert(key.clone(), value.clone());
                }
            }
        }

        // The fields of the event
        let mut visitor = JsonVisitor(&mut fields);
        event.record(&mut visitor);

        // Add default fields
        fields.insert("target".to_string(), event.metadata().target().into());
        fields.insert("name".to_string(), event.metadata().name().into());
        fields.insert(
            "log_level".to_string(),
            event.metadata().level().as_str().into(),
        );
        fields.insert(
            "timestamp".to_string(),
            OffsetDateTime::now_utc()
                .format(&self.timestamp_format)
                .unwrap()
                .into(),
        );

        // And create our output
        let output = fields.into();

        self.output.write(output);
    }
}

struct JsonVisitor<'a>(&'a mut Map<String, Value>);

impl<'a> tracing::field::Visit for JsonVisitor<'a> {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.0.insert(
            field.name().to_string(),
            serde_json::json!(value.to_string()),
        );
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(
            field.name().to_string(),
            serde_json::json!(format!("{:?}", value)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use time::macros::format_description;
    use time::parsing::Parsable;
    use time::PrimitiveDateTime;
    use tracing::field;
    use tracing::subscriber::with_default;
    use tracing_subscriber::Registry;

    /// A helper function for asserting a serde::Value matches expectations
    fn assert_json_timestamp_name(
        expected: Value,
        name_value_prefix: &str,
        before: &OffsetDateTime,
        value: &mut Value,
    ) {
        assert_json_timestamp_name_with_format(
            expected,
            name_value_prefix,
            before,
            value,
            &Iso8601::DEFAULT,
        )
    }

    /// A helper function for asserting a serde::Value matches expectations
    fn assert_json_timestamp_name_with_format(
        expected: Value,
        name_value_prefix: &str,
        before: &OffsetDateTime,
        value: &mut Value,
        timestamp_format: &(impl Parsable + ?Sized),
    ) {
        let map = value.as_object_mut().unwrap();
        assert!(map.contains_key("name"));
        assert!(map
            .remove("name")
            .expect("should contain field 'name'")
            .as_str()
            .expect("field 'name' should be a string")
            .starts_with(name_value_prefix));

        assert!(map.contains_key("timestamp"));
        let timestamp = map
            .remove("timestamp")
            .expect("should contain field 'timestamp'")
            .as_str()
            .expect("field 'timestamp' should be a string")
            .to_string();
        let parsed = PrimitiveDateTime::parse(&timestamp, timestamp_format)
            .expect("timestamp should be parseable")
            .assume_utc();
        assert!(
            (parsed + Duration::from_millis(1)).ge(before),
            "timestamp ({}) must not be before {}",
            timestamp,
            before
        );
        let now = &OffsetDateTime::now_utc();
        assert!(
            parsed.le(now),
            "timestamp ({}) must not be after {}",
            timestamp,
            now
        );

        assert_eq!(expected, *value)
    }

    struct TestOutput {
        data: Arc<Mutex<Vec<Value>>>,
    }

    impl JsonOutput for TestOutput {
        fn write(&self, value: Value) {
            let mut data = self.data.lock().unwrap();
            (*data).push(value);
        }
    }

    #[test]
    fn one_span_some_fields() {
        tracing_subscriber::fmt().pretty().init();

        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        tracing::info!("BEFORE");

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_top = 0).entered();
            tracing::info!(field_event = "from event", "FOOBAR");
            tracing::error!("BAZ");
        });

        tracing::info!("AFTER");

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "field_top": 0,
                "field_event": "from event"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "ERROR",
                "message": "BAZ",
                "field_top": 0,
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn two_spans_different_fields() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_top = 0).entered();
            let _span2 = tracing::info_span!("Second level", field_second = 1).entered();
            tracing::info!(field_event = "from event", "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "field_top": 0,
                "field_second": 1,
                "field_event": "from event"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn two_spans_same_fields() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_overwrite = 0).entered();
            let _span2 = tracing::info_span!("Second level", field_overwrite = 1).entered();
            tracing::info!(field_event = "from event", "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "field_overwrite": 1,
                "field_event": "from event"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn two_spans_same_fields_including_event() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_overwrite = 0).entered();
            let _span2 = tracing::info_span!("Second level", field_overwrite = 1).entered();
            tracing::info!(field_overwrite = "from event", "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "field_overwrite": "from event"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn two_events_from_two_spans() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_top = 0).entered();
            tracing::info!(field_event = "from event one", "ONE");
            let _span2 = tracing::info_span!("Second level", field_second = 1).entered();
            tracing::info!(field_event = "from event two", "TWO");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "ONE",
                "field_top": 0,
                "field_event": "from event one"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "TWO",
                "field_top": 0,
                "field_second": 1,
                "field_event": "from event two"
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn one_span_recorded_field() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let span = tracing::info_span!("A span", span_field = 0, recorded_field = field::Empty);
            span.record("recorded_field", "foo");
            let _span = span.entered();
            tracing::info!(event_field = 1.1, "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "event_field": 1.1,
                "recorded_field": "foo",
                "span_field": 0,
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn one_span_recorded_field_overwriting_initial_field() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            let span = tracing::info_span!("A span", span_field = 0);
            span.record("span_field", "foo");
            let _span = span.entered();
            tracing::info!(event_field = 1.1, "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "event_field": 1.1,
                "span_field": "foo",
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn without_any_spans() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            tracing::info!(event_field = 1.1, "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "event_field": 1.1,
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn custom_timestamp() {
        let timestamp_format = format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        );
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default()
            .with_output(TestOutput { data: data.clone() })
            .with_timestamp_format(timestamp_format);

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            tracing::info!(event_field = 1.1, "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name_with_format(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "FOOBAR",
                "event_field": 1.1,
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
            &timestamp_format,
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn logging_levels() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default().with_output(TestOutput { data: data.clone() });

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            tracing::info!("INFO");
            tracing::trace!("TRACE");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "INFO",
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn logging_levels_trace() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default()
            .with_output(TestOutput { data: data.clone() })
            .with_level(LevelFilter::TRACE);

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            tracing::info!("INFO");
            tracing::trace!("TRACE");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "INFO",
                "message": "INFO",
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json_span_fields::tests",
                "log_level": "TRACE",
                "message": "TRACE",
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }

    #[test]
    fn logging_levels_off() {
        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer::default()
            .with_output(TestOutput { data: data.clone() })
            .with_level(LevelFilter::OFF);

        let subscriber = Registry::default().with(layer);

        with_default(subscriber, || {
            tracing::info!("INFO");
            tracing::trace!("TRACE");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_eq!(None, iter.next(), "No logged events");
    }
}
