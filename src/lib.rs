use serde_json::Value;
use std::collections::BTreeMap;
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug)]
struct CustomFieldStorage(BTreeMap<String, serde_json::Value>);

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
        let mut fields = BTreeMap::new();
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
        let json_data: &mut BTreeMap<String, serde_json::Value> = &mut custom_field_storage.0;

        // And add to using our old friend the visitor!
        let mut visitor = JsonVisitor(json_data);
        values.record(&mut visitor);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // All of the span context
        let scope = ctx.event_scope(event).unwrap();
        let mut spans = vec![];
        for span in scope.from_root() {
            let extensions = span.extensions();
            let storage = extensions.get::<CustomFieldStorage>().unwrap();
            let field_data: &BTreeMap<String, serde_json::Value> = &storage.0;
            spans.push(serde_json::json!({
                "target": span.metadata().target(),
                "name": span.name(),
                "level": format!("{:?}", span.metadata().level()),
                "fields": field_data,
            }));
        }

        // The fields of the event
        let mut fields = BTreeMap::new();
        let mut visitor = JsonVisitor(&mut fields);
        event.record(&mut visitor);

        // And create our output
        let output = serde_json::json!({
            "target": event.metadata().target(),
            "name": event.metadata().name(),
            "level": format!("{:?}", event.metadata().level()),
            "fields": fields,
            "spans": spans,
        });

        self.output.write(output);
    }
}

struct JsonVisitor<'a>(&'a mut BTreeMap<String, serde_json::Value>);

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

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::subscriber::with_default;
    use tracing_subscriber::Registry;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
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
    fn foo() {
        tracing_subscriber::fmt().pretty().init();

        let data = Arc::new(Mutex::new(vec![]));
        let layer = JsonLayer {
            output: TestOutput { data: data.clone() },
        };

        let subscriber = Registry::default().with(layer);

        tracing::info!("BEFORE");

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_top = 0).entered();
            tracing::info!("FOOBAR");
            tracing::error!("BAZ");
        });

        tracing::info!("AFTER");

        let data = data.lock().unwrap();
        for d in (*data).iter() {
            dbg!(d);
        }
    }

    #[test]
    fn without_any_spans() {
        let subscriber = Registry::default().with(JsonLayer::default());

        with_default(subscriber, || {
            // TODO: Should not fail in on_event()
            tracing::info!("FOOBAR");
        })
    }
}
