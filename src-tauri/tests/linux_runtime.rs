#![cfg(target_os = "linux")]

use image::{ImageFormat, Rgba, RgbaImage};
use pc_remote_desktop_lib::platform::{self, PlatformError};
use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("pc-remote-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).expect("create isolated temporary directory");
    path
}

fn require_isolated_validation_container() {
    assert!(
        Path::new("/.dockerenv").exists(),
        "runtime fixtures require Docker isolation"
    );
    assert_eq!(
        std::env::var("PC_REMOTE_LINUX_VALIDATION").as_deref(),
        Ok("1"),
        "runtime tests run only through the isolated validation container"
    );
    assert_eq!(
        std::env::var("XDG_RUNTIME_DIR").as_deref(),
        Ok("/validation/runtime"),
        "the test must use the private container runtime directory"
    );
    assert!(
        std::env::var_os("DISPLAY").is_some(),
        "the test requires its private Xvfb display"
    );
    assert!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "the test requires its private session bus"
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").as_deref(),
        Ok("unix:path=/run/dbus/system_bus_socket"),
        "system D-Bus must resolve only to the absent container socket"
    );
    assert!(
        !Path::new("/run/dbus/system_bus_socket").exists(),
        "the test container must not expose a host system bus"
    );
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RestoreEnv {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl RestoreEnv {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        if let Some(value) = &self.previous {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn xdg_autostart_writes_and_removes_only_under_the_test_config_root() {
    require_isolated_validation_container();
    let root = temp_dir("xdg");
    let _cleanup = RemoveOnDrop(root.clone());
    let _restore_config = RestoreEnv::set("XDG_CONFIG_HOME", &root);

    let desktop_file = root.join("autostart/pc-remote-desktop.desktop");
    platform::autostart()
        .set_enabled(true)
        .expect("enable autostart");
    let entry = fs::read_to_string(&desktop_file).expect("read generated desktop entry");
    assert!(entry.contains("[Desktop Entry]\n"));
    assert!(entry.contains("Exec=\""));
    assert!(entry.contains("X-GNOME-Autostart-enabled=true\n"));
    platform::autostart()
        .set_enabled(false)
        .expect("disable autostart");
    assert!(
        !desktop_file.exists(),
        "disable removes the generated entry"
    );
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn xtest_backend_delivers_key_pointer_and_button_events_to_a_real_x_window() {
    require_isolated_validation_container();
    use x11rb::{
        connection::Connection,
        protocol::xproto::{ConnectionExt, CreateWindowAux, EventMask, InputFocus, WindowClass},
        protocol::xtest::ConnectionExt as _,
        CURRENT_TIME,
    };

    let (conn, screen_num) = x11rb::connect(None).expect("connect to Xvfb");
    let screen = &conn.setup().roots[screen_num];
    let window = conn.generate_id().expect("allocate X window id");
    conn.create_window(
        screen.root_depth,
        window,
        screen.root,
        10,
        10,
        300,
        200,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new().event_mask(
            EventMask::KEY_PRESS
                | EventMask::KEY_RELEASE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION
                | EventMask::FOCUS_CHANGE,
        ),
    )
    .expect("create X window")
    .check()
    .expect("create X window checked");
    conn.map_window(window)
        .expect("map X window")
        .check()
        .expect("map X window checked");
    conn.set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)
        .expect("focus X window")
        .check()
        .expect("focus X window checked");
    // Initialize the XTEST slave pointer too. Core WarpPointer alone leaves the
    // headless XTEST device at (0, 0) until its first synthetic pointer event.
    conn.xtest_fake_input(
        x11rb::protocol::xproto::MOTION_NOTIFY_EVENT,
        0,
        CURRENT_TIME,
        screen.root,
        100,
        100,
        0,
    )
    .expect("place XTEST pointer")
    .check()
    .expect("place XTEST pointer checked");
    conn.flush().expect("flush X setup");
    thread::sleep(Duration::from_millis(100));
    while conn
        .poll_for_event()
        .expect("drain X setup events")
        .is_some()
    {}

    let input = platform::input();
    let initial_pointer = conn
        .query_pointer(window)
        .expect("query initial pointer")
        .reply()
        .expect("initial pointer reply");
    assert_eq!((initial_pointer.root_x, initial_pointer.root_y), (100, 100));
    eprintln!(
        "XTEST fixture initial pointer: ({}, {})",
        initial_pointer.root_x, initial_pointer.root_y
    );
    input
        .press_key(serde_json::from_str("\"enter\"").expect("deserialize Enter key"))
        .expect("XTEST Enter");
    input
        .click_pointer(
            serde_json::from_str("\"left\"").expect("deserialize left pointer button"),
            1,
        )
        .expect("XTEST left click");
    let after_click = conn.query_pointer(window).unwrap().reply().unwrap();
    eprintln!(
        "XTEST pointer after click: ({}, {})",
        after_click.root_x, after_click.root_y
    );
    input.move_pointer(24, 27).expect("XTEST pointer move");
    let after_move = conn.query_pointer(window).unwrap().reply().unwrap();
    eprintln!(
        "XTEST pointer after move: ({}, {})",
        after_move.root_x, after_move.root_y
    );
    input.scroll_pointer(1).expect("XTEST wheel scroll");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut key_press = false;
    let mut key_release = false;
    let mut button_press = false;
    let mut button_release = false;
    let mut wheel_press = false;
    let mut wheel_release = false;
    let mut pointer_motion_at = None;
    while Instant::now() < deadline
        && !(key_press
            && key_release
            && button_press
            && button_release
            && wheel_press
            && wheel_release
            && pointer_motion_at.is_some())
    {
        if let Some(event) = conn.poll_for_event().expect("poll X events") {
            match event {
                x11rb::protocol::Event::KeyPress(event) if event.event == window => {
                    key_press = true;
                }
                x11rb::protocol::Event::KeyRelease(event) if event.event == window => {
                    key_release = true;
                }
                x11rb::protocol::Event::ButtonPress(event)
                    if event.event == window && event.detail == 1 =>
                {
                    button_press = true;
                }
                x11rb::protocol::Event::ButtonRelease(event)
                    if event.event == window && event.detail == 1 =>
                {
                    button_release = true;
                }
                x11rb::protocol::Event::ButtonPress(event)
                    if event.event == window && event.detail == 4 =>
                {
                    wheel_press = true;
                }
                x11rb::protocol::Event::ButtonRelease(event)
                    if event.event == window && event.detail == 4 =>
                {
                    wheel_release = true;
                }
                x11rb::protocol::Event::MotionNotify(event)
                    if event.event == window && event.event_x == 114 && event.event_y == 117 =>
                {
                    pointer_motion_at = Some((event.event_x, event.event_y));
                }
                _ => {}
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }
    conn.destroy_window(window).ok();
    assert!(key_press, "the target X window received a key press");
    assert!(key_release, "the target X window received a key release");
    assert!(button_press, "the target X window received a button press");
    assert!(
        button_release,
        "the target X window received a button release"
    );
    assert!(wheel_press, "the target X window received a wheel press");
    assert!(
        wheel_release,
        "the target X window received a wheel release"
    );
    assert_eq!(
        pointer_motion_at,
        Some((114, 117)),
        "the backend moved the pointer by (24, 27) from root (100, 100)"
    );
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn clipboard_roundtrips_text_and_image_without_spurious_image_change_events() {
    require_isolated_validation_container();
    let cb = platform::clipboard();
    let text = format!("runtime clipboard check {}", std::process::id());
    let text_sequence = cb.write_text(&text, None).expect("write clipboard text");
    assert_eq!(
        cb.read_text().expect("read clipboard text").as_deref(),
        Some(text.as_str())
    );
    assert!(matches!(
        cb.write_text("stale write", Some(text_sequence.wrapping_sub(1))),
        Err(PlatformError::Conflict)
    ));

    let mut png = Vec::new();
    RgbaImage::from_pixel(3, 2, Rgba([17, 93, 201, 255]))
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .expect("encode clipboard fixture image");
    let (events_tx, events_rx) = std::sync::mpsc::channel();
    cb.start_listener(events_tx);
    let written_seq = cb
        .write_image(&png, &[], Some(text_sequence))
        .expect("write clipboard image");
    assert!(cb.read_image().expect("read clipboard image").is_some());

    let mut independent_reader =
        arboard::Clipboard::new().expect("open independent clipboard client");
    let image = independent_reader
        .get_image()
        .expect("independent client reads image");
    assert_eq!((image.width, image.height), (3, 2));
    assert_eq!(&image.bytes.as_ref()[..4], &[17, 93, 201, 255]);
    assert!(matches!(
        cb.write_image(&png, &[], Some(text_sequence)),
        Err(PlatformError::Conflict)
    ));

    thread::sleep(Duration::from_millis(1200));
    let observed: Vec<u32> = events_rx.try_iter().collect();
    assert_eq!(
        cb.sequence(),
        written_seq,
        "reading the same clipboard image must leave its sequence stable; listener notifications: {observed:?}"
    );
    assert!(
        observed.is_empty(),
        "unchanged image emitted sequence notifications: {observed:?}"
    );

    // Identical RGBA bytes with different dimensions are a real external change.
    independent_reader
        .set_image(arboard::ImageData {
            width: 2,
            height: 3,
            bytes: image.bytes,
        })
        .expect("external client reshapes the image");
    let changed_seq = events_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("listener detects reshaped image");
    assert_ne!(changed_seq, written_seq);
    assert_eq!(changed_seq, cb.sequence());
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn pulse_null_sink_supports_real_volume_and_mute_updates() {
    require_isolated_validation_container();
    let audio = platform::audio();
    let initial = audio.read();
    assert!(
        initial.available,
        "default PulseAudio null sink should be available: {initial:?}"
    );

    let volume = audio
        .set(Some(37), None, None)
        .expect("set null sink volume");
    assert!(volume.available);
    assert!(
        (36..=38).contains(&volume.percent),
        "volume was {}%",
        volume.percent
    );

    let muted = audio.set(None, None, Some(true)).expect("mute null sink");
    assert!(muted.muted);
    let unmuted = audio
        .set(None, None, Some(false))
        .expect("unmute null sink");
    assert!(!unmuted.muted);
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn mpris_backend_reads_a_private_session_player_and_sends_pause() {
    require_isolated_validation_container();
    use zbus::{
        blocking::connection::Builder,
        interface,
        zvariant::{OwnedValue, Value},
    };

    struct FakeRoot;

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl FakeRoot {
        #[zbus(property)]
        fn identity(&self) -> &str {
            "PC Remote Runtime Fixture"
        }
    }

    struct FakePlayer {
        paused: Arc<AtomicBool>,
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        #[zbus(property)]
        fn playback_status(&self) -> &str {
            if self.paused.load(Ordering::SeqCst) {
                "Paused"
            } else {
                "Playing"
            }
        }
        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_seek(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            12_000_000
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            HashMap::from([
                (
                    "xesam:title".to_string(),
                    OwnedValue::try_from(Value::from("Runtime fixture track"))
                        .expect("title value"),
                ),
                (
                    "mpris:length".to_string(),
                    OwnedValue::try_from(Value::from(240_000_000i64)).expect("length value"),
                ),
            ])
        }
        fn play(&self) {}
        fn pause(&self) {
            self.paused.store(true, Ordering::SeqCst);
        }
        fn previous(&self) {}
        fn next(&self) {}
        fn seek(&self, _offset: i64) {}
    }

    let paused = Arc::new(AtomicBool::new(false));
    let player = FakePlayer {
        paused: paused.clone(),
    };
    let _service = Builder::session()
        .expect("session bus available")
        .name("org.mpris.MediaPlayer2.pcremote_runtime_fixture")
        .expect("own private MPRIS fixture name")
        .serve_at("/org/mpris/MediaPlayer2", FakeRoot)
        .expect("export MPRIS root interface")
        .serve_at("/org/mpris/MediaPlayer2", player)
        .expect("export MPRIS player interface")
        .build()
        .expect("start MPRIS fixture");

    let media = platform::media();
    let state = media.read();
    let session = state
        .sessions
        .iter()
        .find(|session| session.app == "PC Remote Runtime Fixture")
        .expect("backend discovers the private MPRIS player");
    assert_eq!(session.title, "Runtime fixture track");
    assert!(session.playing);
    assert_eq!(session.position_seconds, Some(12.0));
    media
        .control(
            &session.id,
            &serde_json::from_str("\"pause\"").expect("deserialize Pause command"),
            None,
            Box::new(|| true),
        )
        .expect("MPRIS Pause roundtrip");
    assert!(paused.load(Ordering::SeqCst), "fixture received Pause");
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn missing_logind_returns_safe_errors_without_contacting_power_services() {
    require_isolated_validation_container();
    let power = platform::power();
    let _ = power.sleep_supported();
    assert!(matches!(power.sleep(), Err(PlatformError::Unavailable(_))));
    assert!(matches!(
        power.shutdown(),
        Err(PlatformError::Unavailable(_))
    ));
}

#[test]
#[ignore = "requires isolated Linux validation container"]
fn system_backend_reports_host_identity_network_and_monotonic_time() {
    require_isolated_validation_container();
    let system = platform::system();
    let os_name = system.os_name();
    assert!(
        os_name.starts_with("Ubuntu"),
        "unexpected container OS name: {os_name}"
    );

    let expected_hostname = fs::read_to_string("/etc/hostname")
        .expect("container hostname file")
        .trim()
        .to_owned();
    assert_eq!(system.hostname(), expected_hostname);

    let macs = system.mac_addresses();
    assert!(
        !macs.is_empty(),
        "the container network adapter exposes a MAC address"
    );
    for mac in &macs {
        assert_eq!(mac.len(), 17, "normalized MAC address: {mac}");
        assert!(
            mac.bytes().enumerate().all(|(index, byte)| {
                if [2, 5, 8, 11, 14].contains(&index) {
                    byte == b':'
                } else {
                    byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
                }
            }),
            "MAC address is lowercase and colon separated: {mac}"
        );
    }

    let before = system.monotonic_ms();
    thread::sleep(Duration::from_millis(20));
    let after = system.monotonic_ms();
    assert!(
        after > before,
        "monotonic time advances ({before} -> {after})"
    );
}
