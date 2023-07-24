use serde_json::{Map, Value};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer;
use tracing_subscriber::layer::Context;
#[allow(unused_imports)]
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug)]
struct CustomFieldStorage(Map<String, serde_json::Value>);

trait JsonOutput<'a> {
    fn write(&self, value: Value);
}

#[derive(Default)]
pub struct JsonStdout {
    pretty: bool,
}

impl<'a> JsonOutput<'a> for JsonStdout {
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

pub struct JsonLayer<O = JsonStdout> {
    output: O,
}

impl Default for JsonLayer {
    fn default() -> Self {
        JsonLayer {
            output: JsonStdout::default(),
        }
    }
}

impl<S, W> layer::Layer<S> for JsonLayer<W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'output> JsonOutput<'output> + 'static,
{
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

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        // Get the span whose data is being recorded
        let span = ctx.span(id).unwrap();

        // Get a mutable reference to the data we created in new_span
        let mut extensions_mut = span.extensions_mut();
        let custom_field_storage: &mut CustomFieldStorage =
            extensions_mut.get_mut::<CustomFieldStorage>().unwrap();
        let json_data: &mut Map<String, serde_json::Value> = &mut custom_field_storage.0;

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
                let field_data: &Map<String, serde_json::Value> = &storage.0;

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
            "level".to_string(),
            format!("{:?}", event.metadata().level()).into(),
        );
        fields.insert(
            "timestamp".to_string(),
            OffsetDateTime::now_utc()
                .format(&Iso8601::DEFAULT)
                .unwrap()
                .into(),
        );

        // And create our output
        let output = fields.into();

        self.output.write(output);
    }
}

struct JsonVisitor<'a>(&'a mut Map<String, serde_json::Value>);

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
    use tracing::field;
    use tracing::subscriber::with_default;
    use tracing_subscriber::Registry;

    /// A helper method for asserting a serde::Value matches expectations
    fn assert_json_timestamp_name(
        expected: Value,
        name_value_prefix: &str,
        before: &OffsetDateTime,
        value: &mut Value,
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
        let parsed = OffsetDateTime::parse(&timestamp, &Iso8601::DEFAULT)
            .expect("timestamp should be parseable");
        assert!(
            parsed.ge(before),
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

    impl<'a> JsonOutput<'a> for TestOutput {
        fn write(&self, value: Value) {
            let mut data = self.data.lock().unwrap();
            (*data).push(value);
        }
    }

    #[test]
    fn one_span_some_fields() {
        tracing_subscriber::fmt().pretty().init();

        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
                "target": "tracing_json::tests",
                "level": "Level(Error)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

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
                "target": "tracing_json::tests",
                "level": "Level(Info)",
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
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

        let subscriber = Registry::default().with(layer);

        let before = OffsetDateTime::now_utc();

        with_default(subscriber, || {
            tracing::info!(event_field = 1.1, "FOOBAR");
        });

        let mut data = data.lock().unwrap();
        let mut iter = (*data).iter_mut();

        assert_json_timestamp_name(
            serde_json::json!({
                "target": "tracing_json::tests",
                "level": "Level(Info)",
                "message": "FOOBAR",
                "event_field": 1.1,
            }),
            "event src/lib.rs:",
            &before,
            iter.next().unwrap(),
        );
        assert_eq!(None, iter.next(), "No more logged events");
    }
}
