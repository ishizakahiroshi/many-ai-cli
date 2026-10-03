use many_ai_cli::{proto::core::*, terminal::events::CoreEventBus};
#[tokio::test]
async fn subscriber_observes_effect_publisher_sequence_and_lag_explicitly() {
    let bus = CoreEventBus::new(2).unwrap();
    let mut sub = bus.subscribe();
    let cancel = HubShutdownCancellation::default();
    assert_eq!(
        bus.publish(CoreEvent::HistoryReset(LiveSessionId(1)))
            .unwrap(),
        EventSequence(1)
    );
    assert_eq!(
        bus.publish(CoreEvent::HistoryReset(LiveSessionId(2)))
            .unwrap(),
        EventSequence(2)
    );
    match sub.next(&cancel).await {
        CoreEventPoll::Event(event) => {
            assert_eq!(event.sequence, EventSequence(1));
            assert!(matches!(
                event.event,
                CoreEvent::HistoryReset(LiveSessionId(1))
            ));
        }
        _ => panic!("missing first event"),
    }
    for id in 3..=5 {
        bus.publish(CoreEvent::HistoryReset(LiveSessionId(id)))
            .unwrap();
    }
    assert!(matches!(
        sub.next(&cancel).await,
        CoreEventPoll::Lagged { missed: 2 }
    ));
    match sub.next(&cancel).await {
        CoreEventPoll::Event(e) => assert_eq!(e.sequence, EventSequence(4)),
        _ => panic!("missing retained event"),
    }
    cancel.cancel();
    assert!(matches!(sub.next(&cancel).await, CoreEventPoll::Cancelled));
}
#[tokio::test]
async fn concurrent_publishers_are_ordered_on_the_single_bus() {
    let bus = CoreEventBus::new(256).unwrap();
    let mut sub = bus.subscribe();
    let mut tasks = Vec::new();
    for id in 0..100 {
        let publisher = bus.clone();
        tasks.push(tokio::spawn(async move {
            publisher
                .publish(CoreEvent::Dismissed(LiveSessionId(id)))
                .unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let cancel = HubShutdownCancellation::default();
    for expected in 1..=100 {
        match sub.next(&cancel).await {
            CoreEventPoll::Event(e) => assert_eq!(e.sequence, EventSequence(expected)),
            _ => panic!("out-of-order bus"),
        }
    }
    drop(bus);
    assert!(matches!(sub.next(&cancel).await, CoreEventPoll::Closed));
}
