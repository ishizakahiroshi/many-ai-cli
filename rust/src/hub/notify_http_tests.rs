use super::{ServiceRouter, http::Request, task_owner::HubTaskOwner};
use crate::{
    config::{Config, ConfigStore, RuntimePaths},
    notify::Manager,
    proto::{
        core::{HttpWaitCancellation, TaskCancellation},
        time::Timestamp,
    },
};
use std::{sync::Arc, time::Duration};
#[tokio::test]
async fn notification_test_cancels_on_http_waiter_loss_without_waiting_for_backend_timeout() {
    let started = Arc::new(tokio::sync::Notify::new());
    let entered = started.clone();
    let backend = axum::Router::new().route(
        "/test",
        axum::routing::post(move || {
            let entered = entered.clone();
            async move {
                entered.notify_one();
                std::future::pending::<()>().await;
                "unreachable"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}/test", listener.local_addr().unwrap());
    let backend_task = tokio::spawn(async move {
        axum::serve(listener, backend).await.unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 48888, installed.path()).unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let manager = Manager::new(
        Default::default(),
        tasks.handle(),
        Arc::new(|_, _| panic!("unexpected background delivery")),
    )
    .unwrap();
    let router = Arc::new(
        ServiceRouter::isolated_at_port(config, paths, 48888)
            .unwrap()
            .with_notifications(manager)
            .with_task_owner(tasks.handle()),
    );
    let request = Request {
        method: "POST".into(),
        path: "/api/notify-test".into(),
        host: "127.0.0.1:48888".into(),
        remote_addr: "127.0.0.1:10000".into(),
        query: "token=synthetic".into(),
        body: serde_json::to_vec(&serde_json::json!({"backend":{"type":"webhook","url":url}}))
            .unwrap(),
        ..Default::default()
    };
    let waiter = HttpWaitCancellation::default();
    let lost = waiter.clone();
    let operation = tokio::spawn(async move {
        router
            .handle_owned_async_at(
                &request,
                Timestamp::now(),
                &TaskCancellation::default(),
                &waiter,
            )
            .await
            .response
    });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    lost.cancel();
    let response = tokio::time::timeout(Duration::from_secs(1), operation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status, 503);
    assert_eq!(tasks.snapshot().effects.running, 0);
    backend_task.abort();
    assert!(backend_task.await.unwrap_err().is_cancelled());
}
