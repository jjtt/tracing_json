use std::collections::BTreeMap;
use std::io;
use tracing::span::{Attributes, Record};
use tracing_subscriber::fmt::{FormatEvent, FormatFields, MakeWriter};
use tracing_subscriber::layer;
use tracing_subscriber::layer::{Context, Layered};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::prelude::*;
use tracing::{Event, Id, Subscriber};

#[derive(Debug)]
struct CustomFieldStorage(BTreeMap<String, serde_json::Value>);

pub struct JsonLayer;

impl<S> layer::Layer<S> for JsonLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        // N: for<'writer> FormatFields<'writer> + 'static,
        // E: FormatEvent<S, N> + 'static,
        // W: for<'writer> MakeWriter<'writer> + 'static,
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

    // fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
    //     if self.fmt_span.trace_enter() || self.fmt_span.trace_close() && self.fmt_span.fmt_timing {
    //         let span = ctx.span(id).expect("Span not found, this is a bug");
    //         let mut extensions = span.extensions_mut();
    //         if let Some(timings) = extensions.get_mut::<Timings>() {
    //             let now = Instant::now();
    //             timings.idle += (now - timings.last).as_nanos() as u64;
    //             timings.last = now;
    //         }
    //
    //         if self.fmt_span.trace_enter() {
    //             with_event_from_span!(id, span, "message" = "enter", |event| {
    //                 drop(extensions);
    //                 drop(span);
    //                 self.on_event(&event, ctx);
    //             });
    //         }
    //     }
    // }

    // fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
    //     if self.fmt_span.trace_exit() || self.fmt_span.trace_close() && self.fmt_span.fmt_timing {
    //         let span = ctx.span(id).expect("Span not found, this is a bug");
    //         let mut extensions = span.extensions_mut();
    //         if let Some(timings) = extensions.get_mut::<Timings>() {
    //             let now = Instant::now();
    //             timings.busy += (now - timings.last).as_nanos() as u64;
    //             timings.last = now;
    //         }
    //
    //         if self.fmt_span.trace_exit() {
    //             with_event_from_span!(id, span, "message" = "exit", |event| {
    //                 drop(extensions);
    //                 drop(span);
    //                 self.on_event(&event, ctx);
    //             });
    //         }
    //     }
    // }

    // fn on_close(&self, id: Id, ctx: Context<'_, S>) {
    //     if self.fmt_span.trace_close() {
    //         let span = ctx.span(&id).expect("Span not found, this is a bug");
    //         let extensions = span.extensions();
    //         if let Some(timing) = extensions.get::<Timings>() {
    //             let Timings {
    //                 busy,
    //                 mut idle,
    //                 last,
    //             } = *timing;
    //             idle += (Instant::now() - last).as_nanos() as u64;
    //
    //             let t_idle = field::display(TimingDisplay(idle));
    //             let t_busy = field::display(TimingDisplay(busy));
    //
    //             with_event_from_span!(
    //                 id,
    //                 span,
    //                 "message" = "close",
    //                 "time.busy" = t_busy,
    //                 "time.idle" = t_idle,
    //                 |event| {
    //                     drop(extensions);
    //                     drop(span);
    //                     self.on_event(&event, ctx);
    //                 }
    //             );
    //         } else {
    //             with_event_from_span!(id, span, "message" = "close", |event| {
    //                 drop(extensions);
    //                 drop(span);
    //                 self.on_event(&event, ctx);
    //             });
    //         }
    //     }
    // }

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
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        // thread_local! {
        //     static BUF: RefCell<String> = RefCell::new(String::new());
        // }
        //
        // BUF.with(|buf| {
        //     let borrow = buf.try_borrow_mut();
        //     let mut a;
        //     let mut b;
        //     let mut buf = match borrow {
        //         Ok(buf) => {
        //             a = buf;
        //             &mut *a
        //         }
        //         _ => {
        //             b = String::new();
        //             &mut b
        //         }
        //     };
        //
        //     let ctx = self.make_ctx(ctx, event);
        //     if self
        //         .fmt_event
        //         .format_event(
        //             &ctx,
        //             format::Writer::new(&mut buf).with_ansi(self.is_ansi),
        //             event,
        //         )
        //         .is_ok()
        //     {
        //         let mut writer = self.make_writer.make_writer_for(event.metadata());
        //         let res = io::Write::write_all(&mut writer, buf.as_bytes());
        //         if self.log_internal_errors {
        //             if let Err(e) = res {
        //                 eprintln!("[tracing-subscriber] Unable to write an event to the Writer for this Subscriber! Error: {}\n", e);
        //             }
        //         }
        //     } else if self.log_internal_errors {
        //         let err_msg = format!("Unable to format the following event. Name: {}; Fields: {:?}\n",
        //                               event.metadata().name(), event.fields());
        //         let mut writer = self.make_writer.make_writer_for(event.metadata());
        //         let res = io::Write::write_all(&mut writer, err_msg.as_bytes());
        //         if let Err(e) = res {
        //             eprintln!("[tracing-subscriber] Unable to write an \"event formatting error\" to the Writer for this Subscriber! Error: {}\n", e);
        //         }
        //     }
        //
        //     buf.clear();
        // });
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
    use tracing::subscriber;
    use tracing::subscriber::with_default;
    use tracing_subscriber::Registry;
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }

    #[test]
    fn foo() {
        let subscriber = Registry::default().with(JsonLayer);

        with_default(subscriber, || {
            let _span1 = tracing::info_span!("Top level", field_top = 0).entered();
            tracing::info!("FOOBAR");
        })
    }

    #[test]
    fn without_any_spans() {
        let subscriber = Registry::default().with(JsonLayer);

        with_default(subscriber, || {
            // TODO: Should not fail in on_event()
            tracing::info!("FOOBAR");
        })
    }
}
