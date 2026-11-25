//! Concurrent writers on one physical database, with identical message ids
//! under every account. Correctness fixture only: the timeouts are liveness
//! guards, not latency or memory assertions.

mod common;

use common::*;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Barrier;
use whatsapp_rust_sqlite_storage::{SqliteDatabase, SqliteDatabaseConfig};

const PEER_JID: &str = "stresspeer@lid";
const MESSAGE_COUNT: usize = 160;
const FIRST_TS: i64 = 1_700_000_000;
const REDELIVERED_INDEX: usize = 0;
const SENTINEL_ID: &str = "STRESS-SENTINEL";
const LIVENESS: Duration = Duration::from_secs(20);

/// One real SQLite file per case, cleaned up even when a case panics.
struct TempDb {
    path: std::path::PathBuf,
}

impl TempDb {
    fn new(accounts: usize) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "oxidezap_writer_stress_{}_{}_{}.db",
            std::process::id(),
            accounts,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self { path }
    }

    fn url(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    fn remove(&self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.path.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.remove();
    }
}

fn stress_id(index: usize) -> String {
    format!("STRESS-{index:03}")
}

fn payload(account: usize, index: usize) -> String {
    format!("synthetic account {account} payload {index}")
}

fn redelivered_payload(account: usize) -> String {
    format!("synthetic account {account} redelivered payload")
}

fn sentinel_payload(account: usize) -> String {
    format!("synthetic account {account} sentinel payload")
}

fn expected_payloads(account: usize) -> BTreeMap<String, String> {
    let mut expected = BTreeMap::new();
    for index in 0..MESSAGE_COUNT {
        expected.insert(stress_id(index), payload(account, index));
    }
    expected.insert(stress_id(REDELIVERED_INDEX), redelivered_payload(account));
    expected.insert(SENTINEL_ID.to_owned(), sentinel_payload(account));
    expected
}

fn text_event(index: usize, text: String) -> Event {
    message_event(
        wa::Message::text(text),
        incoming_info(
            PEER_JID,
            PEER_JID,
            &stress_id(index),
            FIRST_TS + index as i64,
        ),
    )
}

fn sentinel_message(account: usize) -> InboundMessage {
    InboundMessage::builder()
        .message(Arc::new(wa::Message::text(sentinel_payload(account))))
        .info(Arc::new(incoming_info(
            PEER_JID,
            PEER_JID,
            SENTINEL_ID,
            FIRST_TS + MESSAGE_COUNT as i64,
        )))
        .build()
}

async fn write_account(index: usize, chat_store: Arc<ChatStore>, barrier: Arc<Barrier>) {
    barrier.wait().await;
    let handler = chat_store.handler();
    for message in 0..MESSAGE_COUNT {
        handler.handle_event(Arc::new(text_event(message, payload(index, message))));
    }
    handler.handle_event(Arc::new(text_event(
        REDELIVERED_INDEX,
        redelivered_payload(index),
    )));
    chat_store.flush().await.expect("handler path barrier");
    chat_store
        .commit_inbound_batch(std::slice::from_ref(&sentinel_message(index)))
        .await
        .expect("sentinel commit");
}

async fn verify_account(index: usize, chat_store: &ChatStore, expected: &BTreeMap<String, String>) {
    let peer = jid(PEER_JID);
    let rows = chat_store
        .messages(&peer, None, (MESSAGE_COUNT + 4) as i64)
        .await
        .expect("account messages");
    let seen: BTreeMap<String, String> = rows
        .iter()
        .map(|row| {
            (
                row.id.clone(),
                row.text.clone().expect("stored text payload"),
            )
        })
        .collect();
    assert_eq!(
        seen.len(),
        rows.len(),
        "no duplicate ids for account {index}"
    );
    assert_eq!(&seen, expected, "account {index} message set and text");

    for (id, text) in expected {
        let stored = chat_store
            .message(&peer, id)
            .await
            .expect("message lookup")
            .expect("message id present");
        assert_eq!(
            stored.text.as_deref(),
            Some(text.as_str()),
            "account {index} id {id}"
        );
    }
    assert_eq!(
        chat_store
            .message(&peer, &stress_id(REDELIVERED_INDEX))
            .await
            .expect("redelivery lookup")
            .expect("redelivered id present")
            .text
            .as_deref(),
        Some(redelivered_payload(index).as_str()),
        "redelivery wins for account {index}"
    );
    assert_eq!(
        chat_store
            .message(&peer, SENTINEL_ID)
            .await
            .expect("sentinel lookup")
            .expect("sentinel present")
            .text
            .as_deref(),
        Some(sentinel_payload(index).as_str()),
        "sentinel is durable for account {index}"
    );

    let coverage = chat_store
        .message_coverage(Some(&peer))
        .await
        .expect("coverage");
    assert_eq!(
        coverage.stored_count,
        expected.len() as u64,
        "account {index} stored count"
    );

    let chats = chat_store.chats(false, 10).await.expect("chat list");
    assert_eq!(chats.len(), 1, "account {index} has exactly one chat");
    assert_eq!(chats[0].jid, peer, "account {index} chat peer");
}

#[derive(diesel::QueryableByName)]
struct StressRow {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    device_id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    msg_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    text_content: String,
}

async fn verify_physical(
    db: &SqliteDatabase,
    expected_by_device: &BTreeMap<i32, BTreeMap<String, String>>,
) {
    let rows: Vec<StressRow> = db
        .shared()
        .read(|conn| {
            diesel::sql_query(
                "SELECT device_id, msg_id, text_content FROM messages \
                 WHERE msg_id LIKE 'STRESS-%'",
            )
            .load::<StressRow>(conn)
            .map_err(db_err)
        })
        .await
        .expect("physical stress rows");
    assert_eq!(
        rows.len(),
        expected_by_device
            .values()
            .map(BTreeMap::len)
            .sum::<usize>(),
        "no foreign physical rows"
    );

    let mut grouped: BTreeMap<i32, BTreeMap<String, String>> = BTreeMap::new();
    for row in rows {
        assert!(
            grouped
                .entry(row.device_id)
                .or_default()
                .insert(row.msg_id, row.text_content)
                .is_none(),
            "no duplicate physical row"
        );
    }
    assert_eq!(&grouped, expected_by_device, "physical rows per device");
}

async fn run_case(accounts: usize) {
    let temp = TempDb::new(accounts);
    let db = SqliteDatabase::open(&temp.url(), SqliteDatabaseConfig::default())
        .await
        .expect("open real database file");

    let mut stores = Vec::with_capacity(accounts);
    for _ in 0..accounts {
        stores.push(db.create_device().await.expect("allocate account"));
    }
    ChatStore::prepare(&stores[0])
        .await
        .expect("prepare physical schema once");

    let mut chat_stores = Vec::with_capacity(accounts);
    for store in &stores {
        chat_stores.push(
            ChatStore::new_prepared(store)
                .await
                .expect("open prepared store"),
        );
    }

    let barrier = Arc::new(Barrier::new(accounts));
    let tasks: Vec<_> = chat_stores
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, chat_store)| {
            let barrier = Arc::clone(&barrier);
            tokio::spawn(write_account(index, chat_store, barrier))
        })
        .collect();
    for task in tasks {
        tokio::time::timeout(LIVENESS, task)
            .await
            .expect("account task liveness")
            .expect("account task did not panic");
    }

    let mut expected_by_device = BTreeMap::new();
    for (index, (store, chat_store)) in stores.iter().zip(&chat_stores).enumerate() {
        let expected = expected_payloads(index);
        verify_account(index, chat_store, &expected).await;
        expected_by_device.insert(store.device_id(), expected);
    }
    assert_eq!(
        expected_by_device.len(),
        accounts,
        "one distinct device per account"
    );
    verify_physical(&db, &expected_by_device).await;

    for chat_store in &chat_stores {
        tokio::time::timeout(LIVENESS, chat_store.close())
            .await
            .expect("close liveness")
            .expect("close succeeds");
    }

    drop(chat_stores);
    drop(stores);
    drop(db);
    temp.remove();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_keep_identical_ids_scoped_to_each_account() {
    for accounts in [1, 2, 4, 8] {
        tokio::time::timeout(LIVENESS, run_case(accounts))
            .await
            .expect("case liveness");
    }
}
