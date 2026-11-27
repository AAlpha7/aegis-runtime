//! Actual admin/HITL futures paused after enqueue: synthetic SQLite and clients.
use super::*;
use crate::embedding::EmbeddingClient;
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

async fn fixture() -> (tempfile::TempDir, Arc<AdminState>, EmbeddingClient) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("embedding.db");
    let memory = MemoryHandle::spawn(&path).unwrap();
    let mut state =
        webapp_test_state_with_memory_and_db(memory.clone(), Some(path.to_string_lossy().into()));
    let orch = Arc::new(crate::test_tool_helpers::test_orchestrator(
        memory,
        state.settings.clone(),
    ));
    let client = orch.embedding_client_for_settings();
    Arc::get_mut(&mut state).unwrap().orchestrator = Some(orch);
    assert_eq!(
        save(
            state.clone(),
            "PUT",
            "embedding_primary_model",
            json!("before")
        )
        .await
        .status(),
        StatusCode::OK
    );
    (dir, state, client)
}

fn request(method: &str, key: &str, value: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("/settings/{key}"))
        .header("Authorization", "Bearer test-secret-token")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({"value":value}).to_string()))
        .unwrap()
}

async fn save(
    state: Arc<AdminState>,
    method: &str,
    key: &str,
    value: Value,
) -> axum::response::Response {
    admin_router(state)
        .oneshot(request(method, key, value))
        .await
        .unwrap()
}

fn enqueue(future: Pin<&mut impl Future>) {
    assert!(matches!(
        future.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
}

async fn assert_model(state: &AdminState, client: &EmbeddingClient, model: &str) {
    assert_eq!(
        state.settings.get("embedding_primary_model"),
        Some(json!(model))
    );
    let disk = state.memory.load_all_settings().await.unwrap();
    let effective = disk
        .get("embedding_primary_model")
        .cloned()
        .or_else(|| RuntimeSettings::new().get("embedding_primary_model"));
    assert_eq!(effective, Some(json!(model)));
    assert_eq!(client.primary_model(), model);
}

async fn reply_value(response: axum::response::Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["value"].clone()
}

#[tokio::test]
async fn embedding_persistence_put_cannot_reorder_or_skip_client_publication() {
    for cancel in [false, true] {
        let (_dir, state, client) = fixture().await;
        let release = state.memory.test_settings_barrier().await;
        let mut first = Box::pin(admin_router(state.clone()).oneshot(request(
            "PUT",
            "embedding_primary_model",
            json!("writer-a"),
        )));
        enqueue(first.as_mut());
        release.send(()).unwrap();
        // Actor read is a barrier; A's HTTP future has not resumed.
        assert_eq!(
            state.memory.load_all_settings().await.unwrap()["embedding_primary_model"],
            json!("writer-a")
        );
        assert_model(&state, &client, "writer-a").await;
        // Unrelated writes do not wait for the paused request or a client lock.
        assert_eq!(
            save(state.clone(), "PUT", "display_language", json!("en"))
                .await
                .status(),
            StatusCode::OK
        );
        if cancel {
            drop(first);
        } else {
            assert_eq!(
                save(
                    state.clone(),
                    "PUT",
                    "embedding_primary_model",
                    json!("writer-b")
                )
                .await
                .status(),
                StatusCode::OK
            );
            assert_eq!(reply_value(first.await.unwrap()).await, json!("writer-a"));
        }
        assert_model(
            &state,
            &client,
            if cancel { "writer-a" } else { "writer-b" },
        )
        .await;
        state.memory.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn embedding_persistence_reset_cannot_reorder_or_skip_client_publication() {
    for cancel in [false, true] {
        let (_dir, state, client) = fixture().await;
        let default = RuntimeSettings::new()
            .get("embedding_primary_model")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let release = state.memory.test_settings_barrier().await;
        let mut first = Box::pin(admin_router(state.clone()).oneshot(request(
            "DELETE",
            "embedding_primary_model",
            Value::Null,
        )));
        enqueue(first.as_mut());
        release.send(()).unwrap();
        assert!(
            !state
                .memory
                .load_all_settings()
                .await
                .unwrap()
                .contains_key("embedding_primary_model")
        );
        assert_model(&state, &client, &default).await;
        if cancel {
            drop(first);
        } else {
            assert_eq!(
                save(
                    state.clone(),
                    "PUT",
                    "embedding_primary_model",
                    json!("after-reset")
                )
                .await
                .status(),
                StatusCode::OK
            );
            assert_eq!(reply_value(first.await.unwrap()).await, json!(default));
        }
        assert_model(
            &state,
            &client,
            if cancel { &default } else { "after-reset" },
        )
        .await;
        state.memory.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn embedding_persistence_hitl_shares_admin_commit_order_and_survives_cancel() {
    for cancel in [false, true] {
        let (_dir, state, client) = fixture().await;
        let orch = state.orchestrator.as_ref().unwrap();
        let value = json!("approved-a");
        let release = state.memory.test_settings_barrier().await;
        let mut first = Box::pin(orch.apply_approved_config(
            "synthetic-embedding",
            "embedding_primary_model",
            &value,
        ));
        enqueue(first.as_mut());
        release.send(()).unwrap();
        assert_eq!(
            state.memory.load_all_settings().await.unwrap()["embedding_primary_model"],
            value
        );
        assert_model(&state, &client, "approved-a").await;
        if cancel {
            drop(first);
        } else {
            assert_eq!(
                save(
                    state.clone(),
                    "PUT",
                    "embedding_primary_model",
                    json!("admin-b")
                )
                .await
                .status(),
                StatusCode::OK
            );
            assert!(matches!(
                first.await,
                crate::hitl::ConfigApplyResult::ChangedFromTo { .. }
            ));
        }
        assert_model(
            &state,
            &client,
            if cancel { "approved-a" } else { "admin-b" },
        )
        .await;
        state.memory.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn embedding_persistence_plan_failure_preserves_db_cache_and_client() {
    let (_dir, state, client) = fixture().await;
    let before = state.memory.load_all_settings().await.unwrap();
    for (key, value) in [
        ("embedding_primary_model", json!("")),
        ("embedding_dimensions", json!(0)),
        ("embedding_backup_model", json!({})),
    ] {
        assert_eq!(
            save(state.clone(), "PUT", key, value).await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(state.memory.load_all_settings().await.unwrap(), before);
        assert_model(&state, &client, "before").await;
    }
    // A valid early candidate must not publish when a later plan fails to build.
    assert!(
        state
            .memory
            .commit_runtime_settings_with_embedding(
                state.settings.clone(),
                vec![
                    ("embedding_primary_model".into(), Some(json!("candidate"))),
                    ("embedding_dimensions".into(), Some(json!(0))),
                ],
                Some(client.clone())
            )
            .await
            .is_err()
    );
    assert_eq!(state.memory.load_all_settings().await.unwrap(), before);
    assert_model(&state, &client, "before").await;
    assert!(matches!(
        state
            .orchestrator
            .as_ref()
            .unwrap()
            .apply_approved_config("synthetic-invalid", "embedding_primary_model", &json!(""))
            .await,
        crate::hitl::ConfigApplyResult::ApplyFailed { .. }
    ));
    assert_model(&state, &client, "before").await;
    assert_eq!(
        save(
            state.clone(),
            "PUT",
            "embedding_primary_model",
            json!("  retry  ")
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_model(&state, &client, "retry").await;
    state.memory.shutdown().await.unwrap();
}

#[tokio::test]
async fn embedding_persistence_sql_failures_never_publish_and_allow_retry() {
    for failure in ["statement", "commit", "delete"] {
        let (_dir, state, client) = fixture().await;
        let conn = rusqlite::Connection::open(state.memory.db_path()).unwrap();
        let sql = match failure {
            "commit" => {
                "CREATE TABLE fault_parent (id INTEGER PRIMARY KEY); CREATE TABLE fault_child (id INTEGER REFERENCES fault_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER fault AFTER INSERT ON user_settings BEGIN INSERT INTO fault_child VALUES (1); END;"
            }
            "delete" => {
                "CREATE TRIGGER fault BEFORE DELETE ON user_settings BEGIN SELECT RAISE(ABORT, 'synthetic'); END;"
            }
            _ => {
                "CREATE TRIGGER fault BEFORE INSERT ON user_settings BEGIN SELECT RAISE(ABORT, 'synthetic'); END;"
            }
        };
        conn.execute_batch(sql).unwrap();
        assert_eq!(
            save(
                state.clone(),
                if failure == "delete" { "DELETE" } else { "PUT" },
                "embedding_primary_model",
                json!("rejected")
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_model(&state, &client, "before").await;
        conn.execute_batch("DROP TRIGGER fault").unwrap();
        assert_eq!(
            save(
                state.clone(),
                "PUT",
                "embedding_primary_model",
                json!("retry")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_model(&state, &client, "retry").await;
        state.memory.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn embedding_persistence_client_reads_do_not_wait_for_sqlite_io() {
    let (_dir, state, client) = fixture().await;
    let conn = rusqlite::Connection::open(state.memory.db_path()).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut write = Box::pin(admin_router(state.clone()).oneshot(request(
        "PUT",
        "embedding_primary_model",
        json!("after-lock"),
    )));
    enqueue(write.as_mut());
    let reader = client.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || send.send(reader.primary_model()).unwrap());
    assert_eq!(
        receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        "before"
    );
    thread.join().unwrap();
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(write.await.unwrap().status(), StatusCode::OK);
    assert_model(&state, &client, "after-lock").await;
    state.memory.shutdown().await.unwrap();
}

#[tokio::test]
async fn embedding_persistence_fast_writes_and_unrelated_settings_finish_consistently() {
    let (_dir, state, client) = fixture().await;
    let mut writes = Vec::new();
    for i in 0..16 {
        let state = state.clone();
        writes.push(tokio::spawn(async move {
            assert_eq!(
                save(
                    state.clone(),
                    "PUT",
                    "embedding_primary_model",
                    json!(format!("model-{i}"))
                )
                .await
                .status(),
                StatusCode::OK
            );
            assert_eq!(
                save(state, "PUT", "display_language", json!("en"))
                    .await
                    .status(),
                StatusCode::OK
            );
        }));
    }
    for write in writes {
        write.await.unwrap();
    }
    let disk = state.memory.load_all_settings().await.unwrap();
    assert_model(
        &state,
        &client,
        disk["embedding_primary_model"].as_str().unwrap(),
    )
    .await;
    assert_eq!(state.settings.get("display_language"), Some(json!("en")));
    state.memory.shutdown().await.unwrap();
}
