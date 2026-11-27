use super::*;
use crate::settings::RuntimeSettings;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn settings_persistence_validates_whole_batch_before_writing() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    assert!(
        memory
            .commit_runtime_settings(
                settings.clone(),
                vec![
                    ("paid_api_fallback_enabled".into(), Some(json!(true))),
                    ("synthetic_unknown_setting".into(), Some(json!(true))),
                ]
            )
            .await
            .is_err()
    );
    assert!(memory.load_all_settings().await.unwrap().is_empty());
    assert_eq!(settings.get_if_stored("paid_api_fallback_enabled"), None);
    memory
        .commit_runtime_settings(
            settings.clone(),
            vec![("paid_api_fallback_enabled".into(), Some(json!("false")))],
        )
        .await
        .unwrap();
    assert_eq!(
        settings.get("paid_api_fallback_enabled"),
        Some(json!(false))
    );
    assert_eq!(
        memory.load_all_settings().await.unwrap()["paid_api_fallback_enabled"],
        json!(false)
    );
    memory
        .commit_runtime_settings(
            settings.clone(),
            vec![("paid_api_fallback_enabled".into(), None)],
        )
        .await
        .unwrap();
    assert_eq!(settings.get_if_stored("paid_api_fallback_enabled"), None);
    assert!(memory.load_all_settings().await.unwrap().is_empty());
}

#[tokio::test]
async fn settings_persistence_cancelled_waiter_cannot_skip_cache_publication() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let updates = settings
        .prepare_updates(vec![(
            "paid_api_fallback_enabled".into(),
            Some(json!(true)),
        )])
        .unwrap();
    let (reply, receiver) = oneshot::channel();
    // Deterministic cancellation after enqueue, before observing the reply.
    memory
        .tx
        .send(MemoryCommand::CommitRuntimeSettings {
            updates,
            settings: settings.clone(),
            embedding: None,
            write_guard: Some(
                crate::call_validation::SETTINGS_WRITE
                    .clone()
                    .lock_owned()
                    .await,
            ),
            reply,
        })
        .await
        .unwrap();
    drop(receiver);
    // Same actor queue: this read is a barrier after commit and publication.
    assert_eq!(
        memory.load_all_settings().await.unwrap()["paid_api_fallback_enabled"],
        json!(true)
    );
    assert_eq!(
        settings.get_if_stored("paid_api_fallback_enabled"),
        Some(json!(true))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_persistence_concurrent_batches_publish_complete_snapshots() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let observed = settings.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_done = done.clone();
    let reader = std::thread::spawn(move || {
        while !reader_done.load(std::sync::atomic::Ordering::Acquire) {
            let snapshot: std::collections::HashMap<_, _> = observed
                .all()
                .into_iter()
                .map(|s| (s.key, s.value))
                .collect();
            if snapshot["llm_model"]
                .as_str()
                .is_some_and(|s| s.starts_with("batch-"))
            {
                assert_eq!(snapshot["llm_provider"], snapshot["llm_model"]);
            }
            // Exercises the metadata/value lock order concurrently with writes.
            let _ = observed.get_if_stored("llm_model");
            std::thread::yield_now();
        }
    });
    let mut writers = Vec::new();
    for index in 0..32 {
        let memory = memory.clone();
        let settings = settings.clone();
        writers.push(tokio::spawn(async move {
            let value = json!(format!("batch-{index}"));
            memory
                .commit_runtime_settings(
                    settings,
                    vec![
                        ("llm_provider".into(), Some(value.clone())),
                        ("llm_model".into(), Some(value)),
                    ],
                )
                .await
                .unwrap();
        }));
    }
    for writer in writers {
        writer.await.unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::Release);
    reader.join().unwrap();
    let disk = memory.load_all_settings().await.unwrap();
    assert_eq!(disk["llm_provider"], disk["llm_model"]);
    assert_eq!(
        settings.get("llm_provider").as_ref(),
        disk.get("llm_provider")
    );
    assert_eq!(settings.get("llm_model").as_ref(), disk.get("llm_model"));
}

#[tokio::test]
async fn settings_persistence_cancelled_request_retains_validation_order() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let write_guard = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    let (entered, entered_rx) = oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    memory
        .tx
        .send(MemoryCommand::SettingsTestBarrier {
            entered,
            release: release_rx,
        })
        .await
        .unwrap();
    entered_rx.await.unwrap();
    let (reply, receiver) = oneshot::channel();
    memory
        .tx
        .send(MemoryCommand::CommitRuntimeSettings {
            settings: settings.clone(),
            embedding: None,
            updates: settings
                .prepare_updates(vec![
                    ("llm_provider".into(), Some(json!("codex_oauth"))),
                    ("llm_model".into(), Some(json!(""))),
                ])
                .unwrap(),
            write_guard: Some(write_guard),
            reply,
        })
        .await
        .unwrap();
    drop(receiver);
    // Cancelled HTTP A cannot let B validate against the pre-A provider.
    assert!(
        crate::call_validation::SETTINGS_WRITE
            .clone()
            .try_lock_owned()
            .is_err()
    );
    release.send(()).unwrap();
    let next_guard = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    assert_eq!(settings.get("llm_provider"), Some(json!("codex_oauth")));
    memory
        .commit_runtime_settings_locked(
            settings.clone(),
            vec![("llm_model".into(), Some(json!("codex-validated-model")))],
            next_guard,
        )
        .await
        .unwrap();
    let stored = memory.load_all_settings().await.unwrap();
    assert_eq!(stored["llm_provider"], json!("codex_oauth"));
    assert_eq!(stored["llm_model"], json!("codex-validated-model"));
    assert_eq!(settings.get("llm_model"), stored.get("llm_model").cloned());
}

#[tokio::test]
async fn settings_persistence_unrelated_settings_do_not_wait_for_provider_probe() {
    let memory = MemoryHandle::spawn_in_memory().unwrap();
    let settings = Arc::new(RuntimeSettings::new());
    let _probe = crate::call_validation::SETTINGS_WRITE
        .clone()
        .lock_owned()
        .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        memory.commit_runtime_settings(
            settings.clone(),
            vec![("display_language".into(), Some(json!("en")))],
        ),
    )
    .await
    .expect("language save must not wait for a provider probe")
    .unwrap();
    assert_eq!(settings.get("display_language"), Some(json!("en")));
    assert_eq!(
        memory.load_all_settings().await.unwrap()["display_language"],
        json!("en")
    );
}
