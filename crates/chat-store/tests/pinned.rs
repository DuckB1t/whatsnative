//! The durable local half of a pin: a row is unpinned until the session says
//! the network accepted a pin, and the bounded account-wide listing follows
//! that flag. All addresses and ids here are synthetic.

// Tests exercise the raw buffa API.
#![allow(clippy::disallowed_methods)]

mod common;

use common::*;

#[tokio::test]
async fn stored_messages_start_unpinned_and_follow_the_set_flag() {
    let (_store, chat_store) = test_store().await;
    let chat = jid(PEER);

    feed(
        &chat_store,
        [
            message_event(
                wa::Message::text("synthetic first"),
                incoming_info(PEER, PEER, "PIN-1", 1_700_000_000),
            ),
            message_event(
                wa::Message::text("synthetic second"),
                incoming_info(PEER, PEER, "PIN-2", 1_700_000_100),
            ),
        ],
    )
    .await;

    // The migration's default is the honest starting state: nothing is pinned
    // until a confirmed network mutation sets the flag.
    assert!(chat_store.pinned_messages(10).await.unwrap().is_empty());

    chat_store
        .set_pinned(&chat, "PIN-1", true)
        .await
        .expect("pin the first row");
    let pinned = chat_store.pinned_messages(10).await.unwrap();
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].id, "PIN-1");
    assert!(pinned[0].pinned);

    // The row's own read path sees the same flag, not just the listing.
    let row = chat_store
        .message(&chat, "PIN-1")
        .await
        .unwrap()
        .expect("the pinned row is still in the timeline");
    assert!(row.pinned);

    chat_store
        .set_pinned(&chat, "PIN-1", false)
        .await
        .expect("unpin the first row");
    assert!(chat_store.pinned_messages(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn pinning_an_unknown_message_is_a_no_op() {
    let (_store, chat_store) = test_store().await;
    chat_store
        .set_pinned(&jid(PEER), "NOT-STORED", true)
        .await
        .expect("an absent row is not an error");
    assert!(chat_store.pinned_messages(10).await.unwrap().is_empty());
}
