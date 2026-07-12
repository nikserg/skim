use crate::db::models::{Address, NewMessage};
use crate::db::queries::insert_message;
use crate::db::Db;
use rusqlite::params;

fn setup() -> Db {
    let db = Db::open_in_memory().unwrap();
    db.with(|conn| {
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
             VALUES ('acc1', 'me@example.com', 'custom', 'imap.example.com', 'smtp.example.com', 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO folders (id, account_id, imap_name, role, display_name)
             VALUES (1, 'acc1', 'INBOX', 'inbox', 'Inbox')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    db
}

fn msg(uid: u32, message_id: &str, refs: &[&str], subject: &str, date: i64) -> NewMessage {
    NewMessage {
        account_id: "acc1".into(),
        folder_id: 1,
        uid,
        message_id: Some(message_id.into()),
        in_reply_to: refs.last().map(|s| s.to_string()),
        references: refs.iter().map(|s| s.to_string()).collect(),
        subject: Some(subject.into()),
        from_name: Some("Sender".into()),
        from_addr: Some("sender@example.com".into()),
        to_addrs: vec![Address {
            name: None,
            addr: "me@example.com".into(),
        }],
        date,
        ..Default::default()
    }
}

fn thread_of(db: &Db, uid: u32) -> i64 {
    db.with(|conn| {
        conn.query_row(
            "SELECT thread_id FROM messages WHERE uid = ?1",
            params![uid],
            |r| r.get(0),
        )
    })
    .unwrap()
}

fn thread_count(db: &Db) -> i64 {
    db.with(|conn| conn.query_row("SELECT count(*) FROM threads", [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn references_chain_threads_in_order() {
    let db = setup();
    db.with(|c| insert_message(c, &msg(1, "<a@x>", &[], "Hello", 1000)).map(|_| ()))
        .unwrap();
    db.with(|c| insert_message(c, &msg(2, "<b@x>", &["<a@x>"], "Re: Hello", 2000)).map(|_| ()))
        .unwrap();
    db.with(|c| {
        insert_message(c, &msg(3, "<c@x>", &["<a@x>", "<b@x>"], "Re: Hello", 3000)).map(|_| ())
    })
    .unwrap();

    assert_eq!(thread_of(&db, 1), thread_of(&db, 2));
    assert_eq!(thread_of(&db, 2), thread_of(&db, 3));
    assert_eq!(thread_count(&db), 1);
}

#[test]
fn reverse_lookup_when_parent_arrives_after_child() {
    let db = setup();
    // Newest-first sync: the reply arrives before the message it references.
    db.with(|c| insert_message(c, &msg(1, "<b@x>", &["<a@x>"], "Re: Hello", 2000)).map(|_| ()))
        .unwrap();
    db.with(|c| insert_message(c, &msg(2, "<a@x>", &[], "Hello", 1000)).map(|_| ()))
        .unwrap();

    assert_eq!(thread_of(&db, 1), thread_of(&db, 2));
    assert_eq!(thread_count(&db), 1);
}

#[test]
fn merge_when_message_bridges_two_threads() {
    let db = setup();
    // Two replies to the same unseen parent arrive first, but reference
    // disjoint ids — they form separate threads.
    db.with(|c| insert_message(c, &msg(1, "<b@x>", &["<a@x>"], "Re: Topic", 2000)).map(|_| ()))
        .unwrap();
    db.with(|c| insert_message(c, &msg(2, "<c@x>", &["<z@x>"], "Re: Topic", 3000)).map(|_| ()))
        .unwrap();
    assert_ne!(thread_of(&db, 1), thread_of(&db, 2));

    // A message referencing both bridges them into one thread.
    db.with(|c| {
        insert_message(c, &msg(3, "<d@x>", &["<a@x>", "<z@x>"], "Re: Topic", 4000)).map(|_| ())
    })
    .unwrap();
    assert_eq!(thread_of(&db, 1), thread_of(&db, 2));
    assert_eq!(thread_of(&db, 2), thread_of(&db, 3));
    assert_eq!(thread_count(&db), 1);
}

#[test]
fn subject_fallback_requires_shared_participant() {
    let db = setup();
    // No references on either message; same normalized subject, shared sender.
    let mut a = msg(1, "<a@x>", &[], "Weekly report", 1000);
    a.in_reply_to = None;
    let mut b = msg(2, "<b@x>", &[], "Re: Weekly report", 2000);
    b.in_reply_to = None;
    db.with(|c| insert_message(c, &a).map(|_| ())).unwrap();
    db.with(|c| insert_message(c, &b).map(|_| ())).unwrap();
    assert_eq!(thread_of(&db, 1), thread_of(&db, 2));

    // Same subject but completely different participants → separate thread.
    let mut stranger = msg(3, "<c@y>", &[], "Weekly report", 2500);
    stranger.in_reply_to = None;
    stranger.from_addr = Some("noreply@newsletter.example".into());
    stranger.to_addrs = vec![Address {
        name: None,
        addr: "subscriber@elsewhere.example".into(),
    }];
    db.with(|c| insert_message(c, &stranger).map(|_| ()))
        .unwrap();
    assert_ne!(thread_of(&db, 3), thread_of(&db, 1));
    assert_eq!(thread_count(&db), 2);
}

#[test]
fn subject_fallback_respects_time_window() {
    let db = setup();
    let mut a = msg(1, "<a@x>", &[], "Standup", 1000);
    a.in_reply_to = None;
    // 60 days later — outside the 30-day fallback window.
    let mut b = msg(2, "<b@x>", &[], "Re: Standup", 1000 + 60 * 24 * 3600);
    b.in_reply_to = None;
    db.with(|c| insert_message(c, &a).map(|_| ())).unwrap();
    db.with(|c| insert_message(c, &b).map(|_| ())).unwrap();
    assert_ne!(thread_of(&db, 1), thread_of(&db, 2));
}

#[test]
fn duplicate_uid_is_skipped() {
    let db = setup();
    let m = msg(1, "<a@x>", &[], "Hello", 1000);
    let first = db.with(|c| insert_message(c, &m)).unwrap();
    let second = db.with(|c| insert_message(c, &m)).unwrap();
    assert!(first.is_some());
    assert!(second.is_none());
}

#[test]
fn thread_aggregates_and_unread_counts() {
    let db = setup();
    let mut a = msg(1, "<a@x>", &[], "Hello", 1000);
    a.is_read = true;
    db.with(|c| insert_message(c, &a).map(|_| ())).unwrap();
    db.with(|c| insert_message(c, &msg(2, "<b@x>", &["<a@x>"], "Re: Hello", 2000)).map(|_| ()))
        .unwrap();

    let (count, unread, last_date): (i64, i64, i64) = db
        .with(|conn| {
            conn.query_row(
                "SELECT message_count, unread_count, last_date FROM threads LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(unread, 1);
    assert_eq!(last_date, 2000);

    let folder_unread: i64 = db
        .with(|conn| {
            conn.query_row("SELECT unread_count FROM folders WHERE id = 1", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(folder_unread, 1);
}

#[test]
fn fts_finds_headers_and_bodies() {
    let db = setup();
    db.with(|c| {
        insert_message(
            c,
            &msg(1, "<a@x>", &[], "Quarterly invoice from Acme", 1000),
        )
        .map(|_| ())
    })
    .unwrap();
    db.with(|conn| {
        crate::db::queries::fts_index_body(conn, 1, "The total amount due is $4,200 by July 3rd.")
    })
    .unwrap();

    let by_subject: i64 = db
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'invoice'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    let by_body: i64 = db
        .with(|conn| {
            conn.query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'amount'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(by_subject, 1);
    assert_eq!(by_body, 1);
}

#[test]
fn normalize_subject_strips_prefixes() {
    use crate::mail::threading::normalize_subject;
    assert_eq!(normalize_subject("Re: Hello"), Some("hello".into()));
    assert_eq!(
        normalize_subject("RE: FWD: Hello  world"),
        Some("hello world".into())
    );
    assert_eq!(normalize_subject("Re[2]: Hello"), Some("hello".into()));
    assert_eq!(normalize_subject("回复: 你好"), Some("你好".into()));
    assert_eq!(normalize_subject("  "), None);
    assert_eq!(normalize_subject("Re:"), None);
}
