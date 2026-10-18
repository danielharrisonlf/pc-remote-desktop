#![cfg(target_os = "linux")]

use pc_remote_desktop_lib::platform::linux::portal::{self, Event};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use zbus::{
    blocking::connection::Builder,
    interface,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
const SESSION_PATH: &str = "/org/freedesktop/portal/session/pcremote_fixture";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    CreateSession,
    SelectDevices,
    Start,
}

#[derive(Debug)]
struct SeenRequest {
    kind: RequestKind,
    request_path: String,
    handle_token: String,
    session_token: Option<String>,
    response: u32,
    selected_devices: Option<u32>,
    parent_window: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InputCall {
    Motion(f64, f64),
    Button(i32, u32),
    Axis(u32, i32),
    Keysym(i32, u32),
}

#[derive(Default)]
struct FixtureState {
    deny_first_start: AtomicBool,
    requests: Mutex<Vec<SeenRequest>>,
    input: Mutex<Vec<InputCall>>,
    session_closes: AtomicUsize,
}

struct FakePortal(Arc<FixtureState>);

fn option_string(options: &HashMap<String, OwnedValue>, key: &str) -> Result<String, String> {
    let value = options
        .get(key)
        .ok_or_else(|| format!("missing option {key}"))?;
    <&str>::try_from(value)
        .map(str::to_owned)
        .map_err(|error| error.to_string())
}

fn option_u32(options: &HashMap<String, OwnedValue>, key: &str) -> Result<u32, String> {
    let value = options
        .get(key)
        .ok_or_else(|| format!("missing option {key}"))?;
    u32::try_from(value).map_err(|error| error.to_string())
}

fn request_path(
    header: &Header<'_>,
    options: &HashMap<String, OwnedValue>,
) -> Result<(String, String), String> {
    let sender = header
        .sender()
        .ok_or_else(|| "D-Bus call has no unique sender".to_string())?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = option_string(options, "handle_token")?;
    Ok((format!("{PORTAL_PATH}/request/{sender}/{token}"), token))
}

fn owned_value(value: Value<'_>) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(value).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

async fn emit_response(
    connection: &zbus::Connection,
    request: &str,
    response: u32,
    results: HashMap<String, OwnedValue>,
) -> zbus::Result<()> {
    connection
        .emit_signal(
            None::<&str>,
            request,
            REQUEST_INTERFACE,
            "Response",
            &(response, results),
        )
        .await
}

fn failed(error: impl ToString) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(error.to_string())
}

#[interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl FakePortal {
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let (path, handle_token) = request_path(&header, &options).map_err(failed)?;
        let session_token = option_string(&options, "session_handle_token").map_err(failed)?;
        self.0.requests.lock().unwrap().push(SeenRequest {
            kind: RequestKind::CreateSession,
            request_path: path.clone(),
            handle_token,
            session_token: Some(session_token),
            response: 0,
            selected_devices: None,
            parent_window: None,
        });

        let results = HashMap::from([(
            "session_handle".to_string(),
            owned_value(Value::from(SESSION_PATH))?,
        )]);
        emit_response(connection, &path, 0, results)
            .await
            .map_err(failed)?;
        OwnedObjectPath::try_from(path.as_str()).map_err(failed)
    }

    async fn select_devices(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        session: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected RemoteDesktop session path"));
        }
        let (path, handle_token) = request_path(&header, &options).map_err(failed)?;
        let selected_devices = option_u32(&options, "types").map_err(failed)?;
        self.0.requests.lock().unwrap().push(SeenRequest {
            kind: RequestKind::SelectDevices,
            request_path: path.clone(),
            handle_token,
            session_token: None,
            response: 0,
            selected_devices: Some(selected_devices),
            parent_window: None,
        });
        emit_response(connection, &path, 0, HashMap::new())
            .await
            .map_err(failed)?;
        OwnedObjectPath::try_from(path.as_str()).map_err(failed)
    }

    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        session: OwnedObjectPath,
        parent_window: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected RemoteDesktop session path"));
        }
        let (path, handle_token) = request_path(&header, &options).map_err(failed)?;
        let denied = self.0.deny_first_start.swap(false, Ordering::SeqCst);
        let response = u32::from(denied);
        self.0.requests.lock().unwrap().push(SeenRequest {
            kind: RequestKind::Start,
            request_path: path.clone(),
            handle_token,
            session_token: None,
            response,
            selected_devices: None,
            parent_window: Some(parent_window),
        });
        let results = if denied {
            HashMap::new()
        } else {
            HashMap::from([("devices".to_string(), owned_value(Value::from(3u32))?)])
        };
        emit_response(connection, &path, response, results)
            .await
            .map_err(failed)?;
        OwnedObjectPath::try_from(path.as_str()).map_err(failed)
    }

    async fn notify_pointer_motion(
        &self,
        session: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        dx: f64,
        dy: f64,
    ) -> zbus::fdo::Result<()> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected input session path"));
        }
        self.0.input.lock().unwrap().push(InputCall::Motion(dx, dy));
        Ok(())
    }

    async fn notify_pointer_button(
        &self,
        session: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        button: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected input session path"));
        }
        self.0
            .input
            .lock()
            .unwrap()
            .push(InputCall::Button(button, state));
        Ok(())
    }

    async fn notify_pointer_axis_discrete(
        &self,
        session: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        axis: u32,
        steps: i32,
    ) -> zbus::fdo::Result<()> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected input session path"));
        }
        self.0
            .input
            .lock()
            .unwrap()
            .push(InputCall::Axis(axis, steps));
        Ok(())
    }

    async fn notify_keyboard_keysym(
        &self,
        session: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        keysym: i32,
        state: u32,
    ) -> zbus::fdo::Result<()> {
        if session.as_str() != SESSION_PATH {
            return Err(failed("unexpected input session path"));
        }
        // A real portal round trip is not zero-cost. This makes the 1 KiB text test
        // expose per-call and whole-request timeouts that a same-tick mock misses.
        tokio::time::sleep(Duration::from_millis(1)).await;
        self.0
            .input
            .lock()
            .unwrap()
            .push(InputCall::Keysym(keysym, state));
        Ok(())
    }
}

struct FakeSession(Arc<FixtureState>);

#[interface(name = "org.freedesktop.portal.Session")]
impl FakeSession {
    async fn close(&self) -> zbus::fdo::Result<()> {
        self.0.session_closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
#[ignore = "requires PC_REMOTE_PORTAL_TEST=1 under a private dbus-run-session"]
fn remote_desktop_consent_tokens_input_text_and_revocation_round_trip() {
    assert_eq!(
        std::env::var("PC_REMOTE_PORTAL_TEST").as_deref(),
        Ok("1"),
        "run only with the explicit marker in a private D-Bus session"
    );
    assert!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "dbus-run-session must provide a private session bus"
    );
    assert!(
        portal::is_wayland(),
        "the fixture must exercise Wayland mode"
    );
    assert!(!portal::authorized());

    let state = Arc::new(FixtureState {
        deny_first_start: AtomicBool::new(true),
        ..FixtureState::default()
    });
    let _service = Builder::session()
        .expect("private session bus available")
        .name(PORTAL_NAME)
        .expect("own the portal bus name on the private bus")
        .serve_at(PORTAL_PATH, FakePortal(state.clone()))
        .expect("export RemoteDesktop interface")
        .serve_at(SESSION_PATH, FakeSession(state.clone()))
        .expect("export portal Session.Close")
        .build()
        .expect("start fake portal");

    // Phone-controlled events cannot trigger a permission prompt or move input
    // before the user explicitly accepts the local Settings action.
    assert!(portal::send(Event::Motion(2, 3)).is_err());
    assert!(state.requests.lock().unwrap().is_empty());
    assert!(state.input.lock().unwrap().is_empty());

    // Exercise a real denial response after a session was created. The app must
    // return the denial and close the temporary session instead of authorizing.
    let denied = portal::prepare().expect_err("the first Start response is denied");
    assert!(
        denied.contains("não autorizado"),
        "unexpected denial: {denied}"
    );
    assert!(!portal::authorized());
    assert_eq!(state.session_closes.load(Ordering::SeqCst), 1);
    assert!(portal::send(Event::Motion(9, 4)).is_err());
    assert!(state.input.lock().unwrap().is_empty());

    // The fixture emitted each Response immediately before replying to the
    // method call. Success proves the client subscribed before making the call
    // and matched all three opaque request handles correctly.
    portal::prepare().expect("the second Start response grants keyboard+pointer");
    assert!(portal::authorized());
    assert!(portal::send(Event::Motion(-3, 7)).is_ok());
    assert!(portal::send(Event::Button(272, 2)).is_ok());
    assert!(portal::send(Event::Scroll(2)).is_ok());
    assert!(portal::send(Event::Key(0xff0d, false)).is_ok());
    assert!(portal::send(Event::Text("\u{00e9}\u{1f98a}".into())).is_ok());
    assert!(portal::send(Event::Chord(vec![0xffe1, 0xffe3], 0x61)).is_ok());

    // The protocol's maximum text payload is 1,024 UTF-8 bytes. The fixture
    // adds 1 ms per D-Bus NotifyKeyboardKeysym call to test the full boundary.
    let long_text = "a".repeat(1024);
    let started = Instant::now();
    portal::send(Event::Text(long_text)).unwrap_or_else(|error| {
        panic!(
            "1 KiB text failed after {:?}, {} notifications: {error:?}",
            started.elapsed(),
            state.input.lock().unwrap().len()
        )
    });
    println!("1024-byte text delivered in {:?}", started.elapsed());

    let input = state.input.lock().unwrap().clone();
    let long_text_calls = vec![InputCall::Keysym(0x61, 1), InputCall::Keysym(0x61, 0)];
    let long_text_start = input.len() - 2048;
    assert_eq!(
        &input[long_text_start..],
        long_text_calls.repeat(1024),
        "each UTF-8 byte is marshaled as a matching key press/release"
    );
    assert_eq!(
        &input[..long_text_start],
        &[
            InputCall::Motion(-3.0, 7.0),
            InputCall::Button(272, 1),
            InputCall::Button(272, 0),
            InputCall::Button(272, 1),
            InputCall::Button(272, 0),
            InputCall::Axis(0, -2),
            InputCall::Keysym(0xff0d, 0),
            InputCall::Keysym(0xe9, 1),
            InputCall::Keysym(0xe9, 0),
            InputCall::Keysym(0x0100_0000 | 0x1f98a, 1),
            InputCall::Keysym(0x0100_0000 | 0x1f98a, 0),
            InputCall::Keysym(0xffe1, 1),
            InputCall::Keysym(0xffe3, 1),
            InputCall::Keysym(0x61, 1),
            InputCall::Keysym(0x61, 0),
            InputCall::Keysym(0xffe3, 0),
            InputCall::Keysym(0xffe1, 0),
        ]
    );

    let requests = state.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.kind)
            .collect::<Vec<_>>(),
        [
            RequestKind::CreateSession,
            RequestKind::SelectDevices,
            RequestKind::Start,
            RequestKind::CreateSession,
            RequestKind::SelectDevices,
            RequestKind::Start,
        ]
    );
    assert_eq!(
        requests
            .iter()
            .map(|request| request.response)
            .collect::<Vec<_>>(),
        [0, 0, 1, 0, 0, 0]
    );
    assert_eq!(requests[1].selected_devices, Some(3));
    assert_eq!(requests[4].selected_devices, Some(3));
    assert_eq!(requests[2].parent_window.as_deref(), Some(""));
    assert_eq!(requests[5].parent_window.as_deref(), Some(""));

    let mut tokens = HashSet::new();
    for request in requests.iter() {
        assert!(request.handle_token.starts_with("pcremote_"));
        assert!(tokens.insert(request.handle_token.as_str()));
        if let Some(session_token) = &request.session_token {
            assert!(session_token.starts_with("pcremote_"));
            assert!(tokens.insert(session_token.as_str()));
        }
        let (root, suffix) = request
            .request_path
            .split_once("/request/")
            .expect("portal request object path contains /request/");
        assert_eq!(root, PORTAL_PATH);
        let (sender, path_token) = suffix
            .split_once('/')
            .expect("portal request object path contains sender and token");
        assert!(!sender.is_empty());
        assert_eq!(path_token, request.handle_token);
    }
    drop(requests);

    // Local Interromper cancels text before Core's state lock is acquired, keeps
    // consent, and releases the last accepted key. Later input stays blocked
    // until the local Liberar action resumes the backend.
    state.input.lock().unwrap().clear();
    let typing = thread::spawn(|| portal::send(Event::Text("a".repeat(1024))));
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.input.lock().unwrap().len() < 10 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(state.input.lock().unwrap().len() >= 10);
    let stopped = Instant::now();
    portal::interrupt();
    assert!(typing.join().unwrap().is_err());
    assert!(
        stopped.elapsed() < Duration::from_millis(500),
        "stop must preempt the remaining text"
    );
    assert!(portal::authorized(), "local stop retains portal consent");
    let interrupted = state.input.lock().unwrap().clone();
    assert!(interrupted.len() < 2048);
    assert_eq!(interrupted.len() % 2, 0);
    for pair in interrupted.chunks_exact(2) {
        assert_eq!(
            pair,
            &[InputCall::Keysym(0x61, 1), InputCall::Keysym(0x61, 0)]
        );
    }
    assert!(portal::send(Event::Motion(1, 1)).is_err());
    portal::resume();
    assert!(portal::send(Event::Motion(1, 1)).is_ok());

    // Closing the portal session revokes input immediately, without replaying
    // queued events or asking the phone to grant permissions again.
    _service
        .emit_signal(None::<&str>, SESSION_PATH, SESSION_INTERFACE, "Closed", &())
        .expect("emit portal session revocation");
    let deadline = Instant::now() + Duration::from_secs(2);
    while portal::authorized() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!portal::authorized(), "Closed clears local authorization");
    let input_count = state.input.lock().unwrap().len();
    assert!(portal::send(Event::Motion(1, 1)).is_err());
    assert_eq!(
        state.input.lock().unwrap().len(),
        input_count,
        "revoked input does not reach the portal"
    );
}
