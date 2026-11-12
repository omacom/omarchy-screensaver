use omarchy_launch_screensaver::{
    engine::Animation,
    observer::{INTERFACE, PATH, Publisher, SERVICE, Snapshot},
    options::Options,
};
use std::{
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use zbus::blocking::{Connection, Proxy};

const TIMEOUT: Duration = Duration::from_secs(5);

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for bus state");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn observer_lifecycle() {
    // Re-run this test on a private bus without changing the test runner's
    // environment or requiring a desktop session. Missing D-Bus is a failure.
    if std::env::var_os("SCREENSAVER_OBSERVER_TEST_BUS").is_none() {
        let mut child = Command::new("dbus-run-session")
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "observer_lifecycle", "--nocapture"])
            .env("SCREENSAVER_OBSERVER_TEST_BUS", "1")
            .spawn()
            .expect("observer tests require dbus-run-session");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "private-bus test failed: {status}");
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("private-bus test timed out");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    let connection = Connection::session().unwrap();
    let bus = Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .unwrap();
    let has_owner = || bus.call::<_, _, bool>("NameHasOwner", &(SERVICE,)).unwrap();
    assert!(!has_owner());

    let publisher = Publisher::start();
    wait_until(has_owner);
    let owner: String = bus.call("GetNameOwner", &(SERVICE,)).unwrap();
    let proxy = Proxy::new(&connection, owner.as_str(), PATH, INTERFACE).unwrap();
    let signals = proxy.receive_signal("StateChanged").unwrap();
    let (sender, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        for message in signals {
            let (snapshot,): (Snapshot,) = message.body().deserialize().unwrap();
            let source = message.header().sender().unwrap().to_string();
            if sender.send((source, snapshot)).is_err() {
                break;
            }
        }
    });
    let initial: Snapshot = proxy.call("GetState", &()).unwrap();
    assert_eq!(initial, Snapshot::default());

    for (index, effect) in ["rain", "matrix"].into_iter().enumerate() {
        let options = Options {
            effect: Some(effect.into()),
            seed: Some(42),
            ..Options::default()
        };
        let mut animation = Animation::new("OMARCHY".into(), options, 80, 24).unwrap();
        let mut expected = animation.take_started_effect().unwrap();
        publisher.publish(expected.clone());
        expected.revision = index as u64 + 1;
        let (source, signalled) = received.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(source, owner);
        assert_eq!(signalled, expected);
        let current: Snapshot = proxy.call("GetState", &()).unwrap();
        assert_eq!(current, expected);
    }

    // A late subscriber can obtain the latest effect without waiting for a
    // transition, even when it connects through a separate bus connection.
    let late_connection = Connection::session().unwrap();
    let late = Proxy::new(&late_connection, owner.as_str(), PATH, INTERFACE).unwrap();
    let current: Snapshot = late.call("GetState", &()).unwrap();
    assert_eq!(current.effect, "matrix");
    assert_eq!(current.revision, 2);

    // Dropping the publisher releases its name while this process stays alive.
    drop(publisher);
    wait_until(|| !has_owner());
    let replacement = Publisher::start();
    wait_until(has_owner);
    let new_owner: String = bus.call("GetNameOwner", &(SERVICE,)).unwrap();
    assert_ne!(new_owner, owner);
    let restarted = Proxy::new(&connection, new_owner.as_str(), PATH, INTERFACE).unwrap();
    let reset: Snapshot = restarted.call("GetState", &()).unwrap();
    assert_eq!(reset, Snapshot::default());
    replacement.publish(current);
    wait_until(|| {
        let state: Snapshot = restarted.call("GetState", &()).unwrap();
        state.revision == 1 && state.effect == "matrix" && state.active
    });
    drop(replacement);
    wait_until(|| !has_owner());
    connection.close().unwrap();
    reader.join().unwrap();
}
