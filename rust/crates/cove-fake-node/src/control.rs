use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::watch;

/// Which wire protocol a request arrived on
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Protocol {
    Esplora,
    Electrum,
}

/// One request a fake server received
///
/// For Esplora `name` is the request path (`/blocks/tip/height`) and `detail`
/// is empty. For Electrum `name` is the JSON-RPC method and `detail` is the
/// serialized params
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LoggedRequest {
    pub protocol: Protocol,
    pub name: String,
    pub detail: String,
}

/// A latch that holds matching requests until the test opens it
///
/// Cloning shares the latch. A held request has been received and logged but
/// is not answered, which lets a test observe the state of the app while a
/// scan is mid-flight
#[derive(Debug, Clone)]
pub struct Gate {
    open: Arc<watch::Sender<bool>>,
    waiting: Arc<AtomicUsize>,
}

impl Gate {
    fn new() -> Self {
        Self { open: Arc::new(watch::channel(false).0), waiting: Arc::new(AtomicUsize::new(0)) }
    }

    /// Releases every held request and lets later ones through unimpeded
    pub fn open(&self) {
        self.open.send_replace(true);
    }

    /// How many requests are currently held at the gate
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    async fn pass(&self) {
        self.waiting.fetch_add(1, Ordering::SeqCst);

        // decrement on drop too, since stopping the server cancels a held request
        let _held = Held(&self.waiting);
        let mut open = self.open.subscribe();
        // the sender lives as long as this gate, so the wait only ends when opened
        let _ = open.wait_for(|is_open| *is_open).await;
    }
}

/// Removes a request from the gate's count when it stops waiting
struct Held<'a>(&'a AtomicUsize);

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What a scripted rule does with one request it matched
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Scripted {
    /// Answers it as the server would without the rule
    Serve,
    /// Processes it, so a broadcast is accepted, then never answers, as a request whose
    /// client died mid-flight is left
    ServeThenStall,
    /// Neither processes nor answers it
    Stall,
    /// Answers with an error, as [`RuleBuilder::fail`] does
    Fail { status: u16, message: String },
}

/// Decides each matched request in turn, see [`RuleBuilder::script`]
#[derive(Clone)]
struct Script(Arc<dyn Fn(&LoggedRequest) -> Scripted + Send + Sync>);

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Script")
    }
}

#[derive(Debug, Clone)]
enum Behavior {
    Fail { status: u16, message: String },
    Stall,
    Delay(Duration),
    Hold(Gate),
    Script(Script),
}

#[derive(Debug)]
struct Rule {
    pattern: String,
    behavior: Behavior,
    hits: Arc<AtomicUsize>,
}

/// Counts how many requests a rule has intercepted
#[derive(Debug, Clone)]
pub struct RuleHandle {
    hits: Arc<AtomicUsize>,
}

impl RuleHandle {
    /// How many requests the rule has intercepted so far
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// What a server must do with a request after the scripted rules ran
#[derive(Debug)]
pub(crate) enum Verdict {
    Serve,
    /// Process the request, then leave it unanswered
    ServeThenStall,
    Fail {
        status: u16,
        message: String,
    },
}

/// Request log and scripted behaviour shared by the fake Esplora and Electrum servers
#[derive(Debug, Default)]
pub(crate) struct Control {
    requests: Mutex<Vec<LoggedRequest>>,
    rules: Mutex<Vec<Rule>>,
}

impl Control {
    pub(crate) fn requests(&self) -> Vec<LoggedRequest> {
        self.requests.lock().clone()
    }

    pub(crate) fn clear_requests(&self) {
        self.requests.lock().clear();
    }

    pub(crate) fn clear_rules(&self) {
        self.rules.lock().clear();
    }

    fn add_rule(&self, pattern: &str, behavior: Behavior) -> RuleHandle {
        let hits = Arc::new(AtomicUsize::new(0));
        self.rules.lock().push(Rule { pattern: pattern.to_string(), behavior, hits: hits.clone() });

        RuleHandle { hits }
    }

    /// Logs the request, applies the first rule that matches and reports what
    /// the server should answer
    ///
    /// A stall never returns, so the connection stays open and silent
    pub(crate) async fn intercept(&self, protocol: Protocol, name: &str, detail: &str) -> Verdict {
        let request =
            LoggedRequest { protocol, name: name.to_string(), detail: detail.to_string() };
        self.requests.lock().push(request.clone());

        let behavior = {
            let rules = self.rules.lock();
            let rule = rules.iter().find(|rule| name.contains(&rule.pattern));

            rule.map(|rule| {
                rule.hits.fetch_add(1, Ordering::SeqCst);
                rule.behavior.clone()
            })
        };

        match behavior {
            None => Verdict::Serve,
            Some(Behavior::Fail { status, message }) => Verdict::Fail { status, message },
            Some(Behavior::Delay(delay)) => {
                tokio::time::sleep(delay).await;
                Verdict::Serve
            }
            Some(Behavior::Hold(gate)) => {
                gate.pass().await;
                Verdict::Serve
            }
            Some(Behavior::Stall) => std::future::pending().await,
            Some(Behavior::Script(Script(decide))) => match decide(&request) {
                Scripted::Serve => Verdict::Serve,
                Scripted::ServeThenStall => Verdict::ServeThenStall,
                Scripted::Stall => std::future::pending().await,
                Scripted::Fail { status, message } => Verdict::Fail { status, message },
            },
        }
    }
}

/// Scripts how the fake servers treat requests whose name contains a pattern
///
/// The first rule that matches wins. An empty pattern matches every request.
/// Esplora names are request paths and Electrum names are method names
pub struct RuleBuilder<'a> {
    pub(crate) control: &'a Control,
    pub(crate) pattern: String,
}

impl RuleBuilder<'_> {
    /// Answers with an error: the HTTP status for Esplora, a JSON-RPC error
    /// carrying `message` for Electrum
    pub fn fail(self, status: u16, message: &str) -> RuleHandle {
        self.control.add_rule(&self.pattern, Behavior::Fail { status, message: message.into() })
    }

    /// Accepts the request and never answers it
    pub fn stall(self) -> RuleHandle {
        self.control.add_rule(&self.pattern, Behavior::Stall)
    }

    /// Answers normally after waiting `delay`
    pub fn delay(self, delay: Duration) -> RuleHandle {
        self.control.add_rule(&self.pattern, Behavior::Delay(delay))
    }

    /// Lets `decide` choose what happens to each matching request, in the order they arrive
    ///
    /// `decide` runs while the request is being received, so it must not block
    pub fn script(
        self,
        decide: impl Fn(&LoggedRequest) -> Scripted + Send + Sync + 'static,
    ) -> RuleHandle {
        self.control.add_rule(&self.pattern, Behavior::Script(Script(Arc::new(decide))))
    }

    /// Holds the request until the returned gate opens
    pub fn hold(self) -> (Gate, RuleHandle) {
        let gate = Gate::new();
        let handle = self.control.add_rule(&self.pattern, Behavior::Hold(gate.clone()));

        (gate, handle)
    }
}
