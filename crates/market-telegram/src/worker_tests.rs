use super::super::{
    tests::{credentials, message},
    transport::{Delivery, Message, TelegramActions, UpdateCheckpoint},
};
use super::*;
use crate::Reply;
use crate::tests::{TestHandler, TestRequest, request};
use grammers_session::updates::{MessageBox, State};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
struct Fake {
    deliveries: VecDeque<Delivery>,
    shared: FakeActions,
}
#[derive(Clone)]
struct FakeActions {
    replies: Arc<Mutex<VecDeque<Result<(), BotError>>>>,
    events: Arc<Mutex<Vec<String>>>,
    sent_text: Arc<Mutex<Vec<String>>>,
    changed: Arc<tokio::sync::Notify>,
    reads: Arc<std::sync::atomic::AtomicUsize>,
}
impl std::ops::Deref for Fake {
    type Target = FakeActions;
    fn deref(&self) -> &FakeActions {
        &self.shared
    }
}
impl Fake {
    fn new(messages: Vec<Option<Message>>, replies: Vec<Result<(), BotError>>) -> Self {
        Self {
            deliveries: messages
                .into_iter()
                .enumerate()
                .map(|(index, message)| Delivery {
                    state: State {
                        date: 10,
                        seq: 10,
                        message_box: Some(MessageBox::Common {
                            pts: index as i32 + 1,
                        }),
                    },
                    message,
                })
                .collect(),
            shared: FakeActions {
                replies: Arc::new(Mutex::new(replies.into())),
                events: Arc::default(),
                sent_text: Arc::default(),
                changed: Arc::default(),
                reads: Arc::default(),
            },
        }
    }
}
impl TelegramActions for FakeActions {
    async fn identity(&self) -> Result<BotUser, BotError> {
        Ok(BotUser {
            id: 42,
            username: "market_bot".into(),
            active_usernames: vec!["market_bot".into(), "market_alias".into()],
            first_name: "Market".into(),
            has_main_web_app: true,
        })
    }
    async fn configure(
        &self,
        _: &[crate::MenuCommand],
        _: Option<&LaunchButton>,
    ) -> Result<(), BotError> {
        self.events.lock().unwrap().push("configure".into());
        Ok(())
    }
    async fn send(
        &self,
        message: &Message,
        reply: &Reply,
        _: Option<&LaunchButton>,
    ) -> Result<(), BotError> {
        self.sent_text.lock().unwrap().push(reply.text.clone());
        self.events
            .lock()
            .unwrap()
            .push(format!("send:{}", message.id));
        self.changed.notify_waiters();
        self.replies.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
}
impl Transport for Fake {
    type Actions = FakeActions;
    fn actions(&self) -> FakeActions {
        self.shared.clone()
    }
    async fn next(&mut self) -> Result<Delivery, BotError> {
        match self.deliveries.pop_front() {
            Some(delivery) => {
                self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(delivery)
            }
            None => std::future::pending().await,
        }
    }
}
impl UpdateCheckpoint for FakeActions {
    async fn acknowledge(&self, state: State) -> Result<(), BotError> {
        let Some(MessageBox::Common { pts }) = state.message_box else {
            panic!("missing progress")
        };
        self.events.lock().unwrap().push(format!("ack:{pts}"));
        self.changed.notify_waiters();
        Ok(())
    }
}

fn worker() -> TelegramWorker {
    TelegramWorker::new(BotConfig {
        credentials: credentials(),
        public_url: Some("https://market.example".into()),
        data_dir: PathBuf::from("unused"),
    })
    .unwrap()
}
#[tokio::test]
async fn authenticated_identity_is_published_before_command_configuration() {
    let mut worker = worker();
    let mut fake = Fake::new(vec![], vec![]);
    let events = fake.events.clone();
    worker.on_authenticated(move |user| {
        assert_eq!(user.id, 42);
        assert_eq!(user.username, "market_bot");
        assert!(user.active_usernames.contains(&"market_alias".to_owned()));
        events.lock().unwrap().push("identity".into());
    });
    assert!(fake.events.lock().unwrap().is_empty());
    worker.serve(&mut fake, false, &TestHandler).await.unwrap();
    // Reconnects republish the current Telegram profile.
    worker.serve(&mut fake, false, &TestHandler).await.unwrap();
    assert_eq!(
        *fake.events.lock().unwrap(),
        ["identity", "configure", "identity", "configure"]
    );
}
#[tokio::test]
async fn generic_handler_receives_peer_identity_and_failure_keeps_pending_update_unacknowledged() {
    use crate::tests::TestRequest;
    struct Handler(Mutex<Vec<TestRequest>>);
    impl MessageHandler for Handler {
        type Request = TestRequest;
        fn parse(
            &self,
            message: crate::IncomingMessage<'_>,
            usernames: &[String],
        ) -> Option<TestRequest> {
            request(message, usernames)
        }
        async fn handle(&self, request: TestRequest) -> Result<Reply, BotError> {
            self.0.lock().unwrap().push(request.clone());
            match request.command.as_str() {
                "status" => Ok(Reply {
                    text: "custom status".into(),
                }),
                _ => Err(BotError::Configuration("custom handler rejected command")),
            }
        }
    }
    let handler = Handler(Mutex::default());
    let mut fake = Fake::new(
        vec![
            Some(message(1, "/status@MARKET_ALIAS user_id=999")),
            Some(message(2, "/help")),
        ],
        vec![],
    );
    assert!(matches!(
        worker().serve(&mut fake, true, &handler).await,
        Err(BotError::Configuration(_))
    ));
    assert_eq!(
        *handler.0.lock().unwrap(),
        [
            TestRequest {
                command: "status".into(),
                user_id: 7,
                message_id: 1,
            },
            TestRequest {
                command: "help".into(),
                user_id: 7,
                message_id: 2,
            },
        ]
    );
    assert_eq!(
        *fake.events.lock().unwrap(),
        ["configure", "send:1", "ack:1"]
    );
    assert_eq!(*fake.sent_text.lock().unwrap(), ["custom status"]);
}
#[tokio::test]
async fn worker_retries_same_message_before_advancing_and_skips_irrelevant_updates() {
    let mut outgoing = message(2, "/start");
    outgoing.outgoing = true;
    let mut group = message(3, "/start");
    group.peer.id = grammers_session::types::PeerId::chat(8).unwrap();
    let mut fake = Fake::new(
        vec![
            Some(message(1, "/start")),
            Some(outgoing),
            Some(group),
            None,
            Some(message(5, "/help")),
        ],
        vec![
            Err(BotError::RetryAfter(1)),
            Ok(()),
            Err(BotError::PeerUnavailable),
        ],
    );
    let worker = worker();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(1200),
            worker.serve(&mut fake, true, &TestHandler)
        )
        .await
        .is_err()
    );
    assert_eq!(
        *fake.events.lock().unwrap(),
        [
            "configure",
            "send:1",
            "send:1",
            "ack:1",
            "ack:2",
            "ack:3",
            "ack:4",
            "send:5",
            "ack:5"
        ]
    );
}
#[tokio::test]
async fn authorization_failure_never_acknowledges_pending_message() {
    let mut fake = Fake::new(
        vec![Some(message(1, "/start"))],
        vec![Err(BotError::Authorization)],
    );
    assert!(matches!(
        worker().serve(&mut fake, true, &TestHandler).await,
        Err(BotError::Authorization)
    ));
    assert_eq!(*fake.events.lock().unwrap(), ["configure", "send:1"]);
}
#[tokio::test]
async fn cancellation_during_flood_wait_leaves_message_unacknowledged() {
    let mut fake = Fake::new(
        vec![Some(message(1, "/start"))],
        vec![Err(BotError::RetryAfter(7200))],
    );
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            worker().serve(&mut fake, true, &TestHandler)
        )
        .await
        .is_err()
    );
    assert_eq!(*fake.events.lock().unwrap(), ["configure", "send:1"]);
}
#[tokio::test]
async fn pre_cancelled_worker_opens_no_session_or_connection() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = worker();
    worker.data_dir = directory.path().join("not-created");
    let stop = CancellationToken::new();
    stop.cancel();
    let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
    worker
        .execute(stop, true, &TestHandler, invocations.handle())
        .await
        .unwrap();
    invocations.shutdown().await;
    assert!(!directory.path().join("not-created").exists());
}
struct BlockingHandler {
    release: Arc<tokio::sync::Notify>,
}
impl MessageHandler for BlockingHandler {
    type Request = TestRequest;
    fn parse(
        &self,
        message: crate::IncomingMessage<'_>,
        usernames: &[String],
    ) -> Option<TestRequest> {
        request(message, usernames)
    }
    async fn handle(&self, request: TestRequest) -> Result<Reply, BotError> {
        if request.message_id == 1 {
            self.release.notified().await;
        }
        Ok(Reply {
            text: format!("reply:{}", request.message_id),
        })
    }
}
fn peer_message(id: i32, peer: i64) -> Message {
    let mut message = message(id, "/status");
    message.peer.id = grammers_session::types::PeerId::user(peer).unwrap();
    message
}
async fn event(actions: &FakeActions, name: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let changed = actions.changed.notified();
            if actions
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == name)
            {
                return;
            }
            changed.await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("missing event {name}: {:?}", actions.events.lock().unwrap()));
}
fn start<H: MessageHandler + 'static>(
    mut fake: Fake,
    handler: H,
    limits: Limits,
) -> (
    FakeActions,
    CancellationToken,
    tokio::task::JoinHandle<Fake>,
) {
    let actions = fake.shared.clone();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let job = tokio::spawn(async move {
        let aliases = ["market_bot".into()];
        tokio::select! {
            _=cancel.cancelled()=>{},
            result=dispatcher::run(&mut fake,&handler,None,&aliases,limits)=>panic!("dispatcher unexpectedly exited: {result:?}"),
        }
        fake
    });
    (actions, stop, job)
}
#[tokio::test]
async fn independent_peers_progress_concurrently_but_peer_order_and_checkpoints_are_preserved() {
    let release = Arc::new(tokio::sync::Notify::new());
    let fake = Fake::new(
        vec![
            Some(peer_message(1, 7)),
            Some(peer_message(2, 8)),
            Some(peer_message(3, 7)),
        ],
        vec![],
    );
    let (actions, stop, job) = start(
        fake,
        BlockingHandler {
            release: release.clone(),
        },
        Limits::default(),
    );
    event(&actions, "send:2").await;
    assert!(
        !actions
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.starts_with("ack:") || event == "send:3")
    );
    release.notify_one();
    event(&actions, "ack:3").await;
    let events = actions.events.lock().unwrap().clone();
    let index = |name| events.iter().position(|event| event == name).unwrap();
    assert!(index("send:1") < index("send:3"));
    assert!(index("ack:1") < index("ack:2"));
    assert!(index("ack:2") < index("ack:3"));
    stop.cancel();
    job.await.unwrap();
}
#[tokio::test]
async fn pending_delivery_limit_applies_backpressure_until_the_oldest_command_finishes() {
    let release = Arc::new(tokio::sync::Notify::new());
    let fake = Fake::new(
        (1..=12)
            .map(|id| Some(peer_message(id, id as i64 + 10)))
            .collect(),
        vec![],
    );
    let (actions, stop, job) = start(
        fake,
        BlockingHandler {
            release: release.clone(),
        },
        Limits {
            pending: 4,
            parallelism: 2,
            ..Limits::default()
        },
    );
    event(&actions, "send:4").await;
    assert_eq!(actions.reads.load(std::sync::atomic::Ordering::SeqCst), 4);
    release.notify_one();
    event(&actions, "ack:12").await;
    stop.cancel();
    job.await.unwrap();
}
#[tokio::test]
async fn rejected_reply_is_completed_without_stopping_following_commands() {
    let fake = Fake::new(
        vec![Some(message(1, "/help")), Some(message(2, "/help"))],
        vec![Err(BotError::Rpc(400)), Ok(())],
    );
    let (actions, stop, job) = start(fake, TestHandler, Limits::default());
    event(&actions, "ack:2").await;
    assert!(actions.events.lock().unwrap().contains(&"send:2".into()));
    stop.cancel();
    job.await.unwrap();
}
#[tokio::test]
async fn a_slow_handler_times_out_without_blocking_another_peer() {
    let fake = Fake::new(
        vec![Some(peer_message(1, 7)), Some(peer_message(2, 8))],
        vec![],
    );
    let (actions, stop, job) = start(
        fake,
        BlockingHandler {
            release: Arc::default(),
        },
        Limits {
            handler_timeout: std::time::Duration::from_millis(20),
            ..Limits::default()
        },
    );
    event(&actions, "ack:2").await;
    let text = actions.sent_text.lock().unwrap().clone();
    assert!(text.iter().any(|text| text.contains("took too long")));
    assert!(text.contains(&"reply:2".into()));
    stop.cancel();
    job.await.unwrap();
}
#[tokio::test]
async fn handler_panics_and_exhausted_transient_errors_are_isolated() {
    struct Handler;
    impl MessageHandler for Handler {
        type Request = TestRequest;
        fn parse(
            &self,
            message: crate::IncomingMessage<'_>,
            usernames: &[String],
        ) -> Option<TestRequest> {
            request(message, usernames)
        }
        async fn handle(&self, request: TestRequest) -> Result<Reply, BotError> {
            match request.message_id {
                1 => panic!("simulated command failure"),
                2 => Err(BotError::Rpc(500)),
                _ => Ok(Reply {
                    text: "still running".into(),
                }),
            }
        }
    }
    let fake = Fake::new(
        vec![
            Some(message(1, "/help")),
            Some(message(2, "/help")),
            Some(message(3, "/help")),
        ],
        vec![],
    );
    let (actions, stop, job) = start(
        fake,
        Handler,
        Limits {
            handler_attempts: 1,
            ..Limits::default()
        },
    );
    event(&actions, "ack:3").await;
    assert!(
        actions
            .sent_text
            .lock()
            .unwrap()
            .contains(&"still running".into())
    );
    stop.cancel();
    job.await.unwrap();
}
#[tokio::test]
async fn receiving_a_delivery_is_not_cancelled_when_another_command_completes() {
    struct Resolving {
        inner: Fake,
        popped: Arc<tokio::sync::Notify>,
        resume: Arc<tokio::sync::Notify>,
    }
    impl Transport for Resolving {
        type Actions = FakeActions;
        fn actions(&self) -> FakeActions {
            self.inner.shared.clone()
        }
        async fn next(&mut self) -> Result<Delivery, BotError> {
            let delivery = self.inner.next().await?;
            if delivery
                .message
                .as_ref()
                .is_some_and(|message| message.id == 2)
            {
                self.popped.notify_one();
                self.resume.notified().await;
            }
            Ok(delivery)
        }
    }
    let fake = Fake::new(
        vec![Some(message(1, "/help")), Some(message(2, "/help"))],
        vec![],
    );
    let actions = fake.shared.clone();
    let popped = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let handler = BlockingHandler {
        release: release.clone(),
    };
    let mut transport = Resolving {
        inner: fake,
        popped: popped.clone(),
        resume: resume.clone(),
    };
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let job = tokio::spawn(async move {
        let aliases = ["market_bot".into()];
        tokio::select! {
            _=cancel.cancelled()=>{},
            result=dispatcher::run(&mut transport,&handler,None,&aliases,Limits::default())=>panic!("{result:?}"),
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), popped.notified())
        .await
        .unwrap();
    release.notify_one();
    event(&actions, "ack:1").await;
    resume.notify_one();
    event(&actions, "ack:2").await;
    stop.cancel();
    job.await.unwrap();
}

#[tokio::test]
async fn receiving_backoff_does_not_suspend_running_commands() {
    struct FailingReceiver {
        inner: Fake,
        failed: Arc<tokio::sync::Notify>,
    }
    impl Transport for FailingReceiver {
        type Actions = FakeActions;
        fn actions(&self) -> FakeActions {
            self.inner.shared.clone()
        }
        async fn next(&mut self) -> Result<Delivery, BotError> {
            if !self.inner.deliveries.is_empty() {
                self.inner.next().await
            } else {
                self.failed.notify_one();
                Err(BotError::Transport)
            }
        }
    }
    let fake = Fake::new(vec![Some(message(1, "/help"))], vec![]);
    let actions = fake.shared.clone();
    let failed = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let handler = BlockingHandler {
        release: release.clone(),
    };
    let mut receiver = FailingReceiver {
        inner: fake,
        failed: failed.clone(),
    };
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let job = tokio::spawn(async move {
        let aliases = ["market_bot".into()];
        tokio::select! {
            _=cancel.cancelled()=>{},
            result=dispatcher::run(&mut receiver,&handler,None,&aliases,Limits::default())=>panic!("{result:?}"),
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), failed.notified())
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        event(&actions, "ack:1"),
    )
    .await
    .unwrap();
    stop.cancel();
    job.await.unwrap();
}

#[tokio::test]
async fn request_fields_cannot_override_transport_peer_scheduling() {
    struct Handler(BlockingHandler);
    impl MessageHandler for Handler {
        type Request = TestRequest;
        fn parse(
            &self,
            message: crate::IncomingMessage<'_>,
            usernames: &[String],
        ) -> Option<TestRequest> {
            let mut request = request(message, usernames)?;
            request.user_id = 0;
            Some(request)
        }
        async fn handle(&self, request: TestRequest) -> Result<Reply, BotError> {
            self.0.handle(request).await
        }
    }
    let release = Arc::new(tokio::sync::Notify::new());
    let fake = Fake::new(
        vec![Some(peer_message(1, 7)), Some(peer_message(2, 8))],
        vec![],
    );
    let (actions, stop, job) = start(
        fake,
        Handler(BlockingHandler {
            release: release.clone(),
        }),
        Limits::default(),
    );
    event(&actions, "send:2").await;
    assert!(
        !actions
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.starts_with("ack:"))
    );
    release.notify_one();
    event(&actions, "ack:2").await;
    stop.cancel();
    job.await.unwrap();
}

#[tokio::test]
async fn closed_invocation_runtime_does_not_open_or_reconnect_a_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = worker();
    worker.data_dir = directory.path().join("not-created");
    let runtime = market_invocations::Runtime::start(Default::default()).unwrap();
    let handle = runtime.handle();
    runtime.shutdown().await;
    let result = worker
        .run_with_invocations(&TestHandler, CancellationToken::new(), handle)
        .await;
    assert!(matches!(result, Err(BotError::Invocations)));
    assert!(!worker.data_dir.exists());
}
