#![allow(missing_docs)]
use super::{span::MockSpan, field as mock_field};
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tokio_trace::{field, Event, Id, Metadata, Subscriber};

#[derive(Debug, Eq, PartialEq)]
struct ExpectEvent {
    // TODO: implement
}

#[derive(Debug, Eq, PartialEq)]
enum Expect {
    #[allow(dead_code)] // TODO: implement!
    Event(ExpectEvent),
    Enter(MockSpan),
    Exit(MockSpan),
    CloneSpan(MockSpan),
    DropSpan(MockSpan),
    Record(MockSpan, mock_field::Expect),
    Nothing,
}

struct SpanState {
    name: &'static str,
    refs: usize,
}

struct Running<F: Fn(&Metadata) -> bool> {
    spans: Mutex<HashMap<Id, SpanState>>,
    expected: Arc<Mutex<VecDeque<Expect>>>,
    ids: AtomicUsize,
    filter: F,
}

pub struct MockSubscriber<F: Fn(&Metadata) -> bool> {
    expected: VecDeque<Expect>,
    filter: F,
}

pub struct MockHandle(Arc<Mutex<VecDeque<Expect>>>);

pub fn mock() -> MockSubscriber<fn(&Metadata) -> bool> {
    MockSubscriber {
        expected: VecDeque::new(),
        filter: (|_: &Metadata| true) as for<'r, 's> fn(&'r Metadata<'s>) -> _,
    }
}

impl<F: Fn(&Metadata) -> bool> MockSubscriber<F> {
    pub fn enter(mut self, span: MockSpan) -> Self {
        self.expected.push_back(Expect::Enter(span));
        self
    }

    pub fn event(mut self) -> Self {
        // TODO: expect message/fields!
        self.expected.push_back(Expect::Event(ExpectEvent {}));
        self
    }

    pub fn exit(mut self, span: MockSpan) -> Self {
        self.expected.push_back(Expect::Exit(span));
        self
    }

    pub fn clone_span(mut self, span: MockSpan) -> Self {
        self.expected.push_back(Expect::CloneSpan(span));
        self
    }

    pub fn drop_span(mut self, span: MockSpan) -> Self {
        self.expected.push_back(Expect::DropSpan(span));
        self
    }

    pub fn done(mut self) -> Self {
        self.expected.push_back(Expect::Nothing);
        self
    }

    pub fn record<I>(mut self, span: MockSpan, fields: I) -> Self
    where
        I: Into<mock_field::Expect>,
    {
        self.expected.push_back(Expect::Record(span, fields.into()));
        self
    }

    pub fn with_filter<G>(self, filter: G) -> MockSubscriber<G>
    where
        G: Fn(&Metadata) -> bool,
    {
        MockSubscriber {
            filter,
            expected: self.expected,
        }
    }

    pub fn run(self) -> impl Subscriber {
        let (subscriber, _) = self.run_with_handle();
        subscriber
    }

    pub fn run_with_handle(self) -> (impl Subscriber, MockHandle) {
        let expected = Arc::new(Mutex::new(self.expected));
        let handle = MockHandle(expected.clone());
        let subscriber = Running {
            spans: Mutex::new(HashMap::new()),
            expected,
            ids: AtomicUsize::new(0),
            filter: self.filter,
        };
        (subscriber, handle)
    }
}

impl<F: Fn(&Metadata) -> bool> Subscriber for Running<F> {
    fn enabled(&self, meta: &Metadata) -> bool {
        (self.filter)(meta)
    }

    fn record(&self, id: &Id, values: field::ValueSet) {
        let spans = self.spans.lock().unwrap();
        let mut expected = self.expected.lock().unwrap();
        let span = spans
            .get(id)
            .unwrap_or_else(|| panic!("no span for ID {:?}", id));
        println!("record: span={:?}; values={:?};", span.name, values);
        let was_expected = if let Some(Expect::Record(_, _)) = expected.front() {
            true
        } else {
            false
        };
        if was_expected {
            if let Expect::Record(expected_span, mut expected_values) =
                expected.pop_front().unwrap()
            {
                if let Some(name) = expected_span.name {
                    assert_eq!(name, span.name);
                }
                expected_values.check(values, format_args!("Span({})::record: ", span.name));
            }

        }
    }

    fn event(&self, event: Event) {
        match self.expected.lock().unwrap().pop_front() {
            None => {}
            Some(Expect::Event(_)) => {
                // TODO: implement me
            }
            Some(ex) => ex.bad(format_args!("observed event {:?}", event)),
        }
    }

    fn add_follows_from(&self, _span: &Id, _follows: Id) {
        // TODO: it should be possible to expect spans to follow from other spans
    }

    fn new_span(&self, meta: &Metadata) -> Id {
        let id = self.ids.fetch_add(1, Ordering::SeqCst);
        let id = Id::from_u64(id as u64);
        println!(
            "new_span: name={:?}; target={:?}; id={:?};",
            meta.name(),
            meta.target(),
            id
        );
        self.spans.lock().unwrap().insert(
            id.clone(),
            SpanState {
                name: meta.name(),
                refs: 1,
            },
        );
        id
    }

    fn enter(&self, id: &Id) {
        let spans = self.spans.lock().unwrap();
        if let Some(span) = spans.get(id) {
            println!("enter: {}; id={:?};", span.name, id);
            match self.expected.lock().unwrap().pop_front() {
                None => {}
                Some(Expect::Enter(ref expected_span)) => {
                    if let Some(name) = expected_span.name {
                        assert_eq!(name, span.name);
                    }
                }
                Some(ex) => ex.bad(format_args!("entered span {:?}", span.name)),
            }
        };
    }

    fn exit(&self, id: &Id) {
        let spans = self.spans.lock().unwrap();
        let span = spans
            .get(id)
            .unwrap_or_else(|| panic!("no span for ID {:?}", id));
        println!("exit: {}; id={:?};", span.name, id);
        match self.expected.lock().unwrap().pop_front() {
            None => {}
            Some(Expect::Exit(ref expected_span)) => {
                if let Some(name) = expected_span.name {
                    assert_eq!(name, span.name);
                }
            }
            Some(ex) => ex.bad(format_args!("exited span {:?}", span.name)),
        };
    }

    fn clone_span(&self, id: &Id) -> Id {
        let name = self.spans.lock().unwrap().get_mut(id).map(|span| {
            let name = span.name;
            println!("clone_span: {}; id={:?}; refs={:?};", name, id, span.refs);
            span.refs += 1;
            name
        });
        if name.is_none() {
            println!("clone_span: id={:?};", id);
        }
        let mut expected = self.expected.lock().unwrap();
        let was_expected = if let Some(Expect::CloneSpan(ref span)) = expected.front() {
            assert_eq!(name, span.name);
            true
        } else {
            false
        };
        if was_expected {
            expected.pop_front();
        }
        id.clone()
    }

    fn drop_span(&self, id: Id) {
        let mut is_event = false;
        let name = if let Ok(mut spans) = self.spans.try_lock() {
            spans.get_mut(&id).map(|span| {
                let name = span.name;
                if name.contains("event") {
                    is_event = true;
                }
                println!("drop_span: {}; id={:?}; refs={:?};", name, id, span.refs);
                span.refs -= 1;
                name
            })
        } else {
            None
        };
        if name.is_none() {
            println!("drop_span: id={:?}", id);
        }
        if let Ok(mut expected) = self.expected.try_lock() {
            let was_expected = match expected.front() {
                Some(Expect::DropSpan(ref span)) => {
                    // Don't assert if this function was called while panicking,
                    // as failing the assertion can cause a double panic.
                    if !::std::thread::panicking() {
                        assert_eq!(name, span.name);
                    }
                    true
                }
                Some(Expect::Event(_)) => {
                    if !::std::thread::panicking() {
                        assert!(is_event);
                    }
                    true
                }
                _ => false,
            };
            if was_expected {
                expected.pop_front();
            }
        }
    }
}

impl MockHandle {
    pub fn assert_finished(&self) {
        if let Ok(ref expected) = self.0.lock() {
            assert!(
                !expected.iter().any(|thing| thing != &Expect::Nothing),
                "more notifications expected: {:?}",
                **expected
            );
        }
    }
}

impl Expect {
    fn bad<'a>(&self, what: fmt::Arguments<'a>) {
        match self {
            Expect::Event(e) => panic!(
                "expected event, but {} instead", what,
            ),
            Expect::Enter(e) => panic!(
                "expected to enter {} but {} instead", e, what,
            ),
            Expect::Exit(e) => panic!(
                "expected to exit {} but {} instead", e, what,
            ),
            Expect::CloneSpan(e) => panic!(
                "expected to clone {} but {} instead", e, what,
            ),
            Expect::DropSpan(e) => panic!(
                "expected to drop {} but {} instead", e, what,
            ),
            Expect::Record(e, fields) => panic!(
                "expected {} to record {} but {} instead", e, fields, what,
            ),
            Expect::Nothing => panic!(
                "expected nothing else to happen, but {} instead", what,
            ),
        }
    }
}
