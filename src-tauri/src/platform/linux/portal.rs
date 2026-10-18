//! Consented native Wayland input through the XDG RemoteDesktop portal.
//! Authorization is requested only from the local UI, never by phone events.
use crate::platform::PlatformError;
use futures_util::StreamExt;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use zbus::{
    zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const INTERRUPTED: &str = "Controle interrompido.";
static INPUT_GENERATION: AtomicU64 = AtomicU64::new(0);
static INPUT_BLOCKED: AtomicBool = AtomicBool::new(false);
type Options<'a> = HashMap<&'a str, Value<'a>>;
type Results = HashMap<String, OwnedValue>;

pub fn is_wayland() -> bool {
    wayland_session(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    )
}

fn wayland_session(session: Option<&str>, display: Option<&str>) -> bool {
    match session {
        Some("wayland") => true,
        Some("x11") => false,
        _ => display.is_some_and(|value| !value.is_empty()),
    }
}

#[derive(Debug)]
pub enum Event {
    Motion(i16, i16),
    Button(i32, u8),
    Scroll(i8),
    Key(u32, bool),
    Chord(Vec<u32>, u32),
    Text(String),
}

impl Event {
    fn timeout(&self) -> Duration {
        // Text uses two ordered, checked D-Bus calls per scalar. A whole-text
        // 400ms timeout can truncate valid 1024-byte messages on a busy desktop.
        let extra = match self {
            Self::Text(text) => 12 * text.chars().count().min(1024) as u64,
            _ => 0,
        };
        Duration::from_millis(400 + extra)
    }
}

enum Request {
    Prepare(mpsc::Sender<Result<(), String>>),
    Input(Event, Instant, u64, mpsc::Sender<Result<(), String>>),
}

struct Worker {
    tx: mpsc::SyncSender<Request>,
    ready: Arc<AtomicBool>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(1);
        let ready = Arc::new(AtomicBool::new(false));
        let live = ready.clone();
        std::thread::Builder::new()
            .name("wayland-input".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let mut session: Option<Session> = None;
                for request in rx {
                    match request {
                        Request::Prepare(reply) => {
                            if live.load(Ordering::SeqCst) {
                                let _ = reply.send(Ok(()));
                                continue;
                            }
                            if let Some(previous) = session.take() {
                                runtime.block_on(previous.close());
                            }
                            let opened = runtime.block_on(async {
                                tokio::time::timeout(
                                    Duration::from_secs(120),
                                    Session::open(live.clone()),
                                )
                                .await
                                .map_err(|_| "O pedido de autorização expirou.".to_string())?
                            });
                            match opened {
                                Ok(value) => {
                                    session = Some(value);
                                    let _ = reply.send(Ok(()));
                                }
                                Err(error) => {
                                    let _ = reply.send(Err(error));
                                }
                            }
                        }
                        Request::Input(event, deadline, generation, reply) => {
                            // Never replay movements queued while permission was being requested.
                            if Instant::now() >= deadline
                                || !live.load(Ordering::SeqCst)
                                || INPUT_BLOCKED.load(Ordering::SeqCst)
                                || generation != INPUT_GENERATION.load(Ordering::SeqCst)
                            {
                                let _ = reply.send(Err(
                                    "Autorize o controle nas Configurações do PC Remote.".into(),
                                ));
                                continue;
                            }
                            let budget = event.timeout();
                            let result = match &session {
                                Some(session) => runtime.block_on(async {
                                    tokio::time::timeout(budget, session.send(event, generation))
                                        .await
                                        .map_err(|_| {
                                            "O portal de controle não respondeu.".to_string()
                                        })?
                                }),
                                None => Err("Controle não autorizado.".into()),
                            };
                            if result.as_ref().is_err_and(|error| error != INTERRUPTED) {
                                live.store(false, Ordering::SeqCst);
                                if let Some(previous) = session.take() {
                                    runtime.block_on(previous.close());
                                }
                            }
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .expect("create Wayland input worker");
        Worker { tx, ready }
    })
}

pub fn authorized() -> bool {
    WORKER
        .get()
        .is_some_and(|worker| worker.ready.load(Ordering::SeqCst))
}

/// Cancel before Core takes its state lock, including in-progress long text.
pub fn interrupt() {
    INPUT_BLOCKED.store(true, Ordering::SeqCst);
    INPUT_GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub fn resume() {
    INPUT_BLOCKED.store(false, Ordering::SeqCst);
}

pub fn prepare() -> Result<(), String> {
    if !is_wayland() {
        return Ok(());
    }
    let (tx, rx) = mpsc::channel();
    worker()
        .tx
        .try_send(Request::Prepare(tx))
        .map_err(|_| "Há uma autorização em andamento. Tente novamente.".to_string())?;
    rx.recv_timeout(Duration::from_secs(130))
        .map_err(|_| "O pedido de autorização expirou. Tente novamente.".to_string())?
}

pub fn send(event: Event) -> Result<(), PlatformError> {
    if !authorized() || INPUT_BLOCKED.load(Ordering::SeqCst) {
        return Err(PlatformError::Unavailable(
            "Autorize o controle nas Configurações do PC Remote.".into(),
        ));
    }
    let (tx, rx) = mpsc::channel();
    let budget = event.timeout();
    worker()
        .tx
        .try_send(Request::Input(
            event,
            Instant::now() + Duration::from_millis(250),
            INPUT_GENERATION.load(Ordering::SeqCst),
            tx,
        ))
        .map_err(|_| PlatformError::Unavailable("Portal ocupado.".into()))?;
    rx.recv_timeout(budget + Duration::from_millis(1500))
        .map_err(|_| PlatformError::Unavailable("Portal indisponível.".into()))?
        .map_err(PlatformError::Unavailable)
}

fn token() -> String {
    format!("pcremote_{}", uuid::Uuid::new_v4().simple())
}

async fn request<B: Serialize + DynamicType>(
    conn: &Connection,
    method: &str,
    token: &str,
    body: &B,
) -> Result<Results, String> {
    let sender = conn
        .unique_name()
        .ok_or("D-Bus sem nome único.")?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("{PATH}/request/{sender}/{token}");
    let request = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Request")
        .await
        .map_err(|e| e.to_string())?;
    // Subscribe before calling: fast portal responses must not be lost.
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|e| e.to_string())?;
    let portal = Proxy::new(conn, DEST, PATH, INTERFACE)
        .await
        .map_err(|e| e.to_string())?;
    let handle: OwnedObjectPath = portal.call(method, body).await.map_err(|e| e.to_string())?;
    if handle.as_str() != path {
        return Err("Resposta inesperada do portal.".into());
    }
    let response = tokio::time::timeout(Duration::from_secs(120), responses.next()).await;
    let message = match response {
        Ok(Some(message)) => message,
        _ => {
            let _ = request.call::<_, _, ()>("Close", &()).await;
            return Err("O pedido de autorização expirou.".into());
        }
    };
    let (code, results): (u32, Results) =
        message.body().deserialize().map_err(|e| e.to_string())?;
    if code != 0 {
        return Err(
            "Controle não autorizado. Você pode tentar novamente nas Configurações.".into(),
        );
    }
    Ok(results)
}

struct Session {
    conn: Connection,
    path: OwnedObjectPath,
    closed: tokio::task::JoinHandle<()>,
}

impl Session {
    async fn open(ready: Arc<AtomicBool>) -> Result<Self, String> {
        let conn = Connection::session().await.map_err(|e| e.to_string())?;
        let create_token = token();
        let session_token = token();
        let mut options = Options::new();
        options.insert("handle_token", Value::from(create_token.as_str()));
        options.insert("session_handle_token", Value::from(session_token.as_str()));
        let results = request(&conn, "CreateSession", &create_token, &(options,)).await?;
        let value = results.get("session_handle").ok_or("Portal sem sessão.")?;
        // session_handle is historically a string, rather than an object path variant.
        let path = OwnedObjectPath::try_from(
            <&str>::try_from(value)
                .map_err(|e| e.to_string())?
                .to_string(),
        )
        .map_err(|e| e.to_string())?;
        let result = Self::start(&conn, &path, ready).await;
        match result {
            Ok(closed) => Ok(Self { conn, path, closed }),
            Err(error) => {
                close_session(&conn, &path).await;
                Err(error)
            }
        }
    }

    async fn start(
        conn: &Connection,
        path: &OwnedObjectPath,
        ready: Arc<AtomicBool>,
    ) -> Result<tokio::task::JoinHandle<()>, String> {
        let select_token = token();
        let mut options = Options::new();
        options.insert("handle_token", Value::from(select_token.as_str()));
        options.insert("types", Value::from(3u32)); // keyboard + pointer; no screen capture
        request(conn, "SelectDevices", &select_token, &(path, options)).await?;
        let start_token = token();
        let mut options = Options::new();
        options.insert("handle_token", Value::from(start_token.as_str()));
        let session = Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Session")
            .await
            .map_err(|e| e.to_string())?;
        let mut closed = session
            .receive_signal("Closed")
            .await
            .map_err(|e| e.to_string())?;
        let results = request(conn, "Start", &start_token, &(path, "", options)).await?;
        let devices = results
            .get("devices")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        if devices & 3 != 3 {
            return Err("Autorize mouse e teclado para usar o controle.".into());
        }
        ready.store(true, Ordering::SeqCst);
        Ok(tokio::spawn(async move {
            closed.next().await;
            ready.store(false, Ordering::SeqCst);
        }))
    }

    async fn close(self) {
        self.closed.abort();
        close_session(&self.conn, &self.path).await;
    }

    async fn send(&self, event: Event, generation: u64) -> Result<(), String> {
        if generation != INPUT_GENERATION.load(Ordering::SeqCst)
            || INPUT_BLOCKED.load(Ordering::SeqCst)
        {
            return Err(INTERRUPTED.into());
        }
        let proxy = Proxy::new(&self.conn, DEST, PATH, INTERFACE)
            .await
            .map_err(|e| e.to_string())?;
        let options = Options::new();
        match event {
            Event::Motion(dx, dy) => {
                proxy
                    .call::<_, _, ()>(
                        "NotifyPointerMotion",
                        &(&self.path, &options, f64::from(dx), f64::from(dy)),
                    )
                    .await
            }
            Event::Scroll(steps) => {
                proxy
                    .call::<_, _, ()>(
                        "NotifyPointerAxisDiscrete",
                        &(&self.path, &options, 0u32, -i32::from(steps)),
                    )
                    .await
            }
            Event::Button(button, count) => {
                for _ in 0..count {
                    proxy
                        .call::<_, _, ()>(
                            "NotifyPointerButton",
                            &(&self.path, &options, button, 1u32),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    proxy
                        .call::<_, _, ()>(
                            "NotifyPointerButton",
                            &(&self.path, &options, button, 0u32),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            Event::Key(sym, pressed) => {
                proxy
                    .call::<_, _, ()>(
                        "NotifyKeyboardKeysym",
                        &(&self.path, &options, sym as i32, u32::from(pressed)),
                    )
                    .await
            }
            Event::Text(text) => {
                for ch in text.chars() {
                    self.chord(&proxy, &[], super::input::char_to_keysym(ch), generation)
                        .await?;
                }
                return Ok(());
            }
            Event::Chord(modifiers, sym) => {
                return self.chord(&proxy, &modifiers, sym, generation).await
            }
        }
        .map_err(|e| e.to_string())
    }

    async fn chord(
        &self,
        proxy: &Proxy<'_>,
        modifiers: &[u32],
        sym: u32,
        generation: u64,
    ) -> Result<(), String> {
        let mut pressed = Vec::with_capacity(modifiers.len() + 1);
        for &key in modifiers.iter().chain(std::iter::once(&sym)) {
            if generation != INPUT_GENERATION.load(Ordering::SeqCst) {
                break;
            }
            self.key(proxy, key, true).await?;
            pressed.push(key);
        }
        // Always release accepted presses before returning from an interruption.
        for &key in pressed.iter().rev() {
            self.key(proxy, key, false).await?;
        }
        if generation != INPUT_GENERATION.load(Ordering::SeqCst) {
            return Err(INTERRUPTED.into());
        }
        Ok(())
    }

    async fn key(&self, proxy: &Proxy<'_>, sym: u32, pressed: bool) -> Result<(), String> {
        // A stalled method must fail promptly even for the longest text event.
        tokio::time::timeout(
            Duration::from_millis(400),
            proxy.call::<_, _, ()>(
                "NotifyKeyboardKeysym",
                &(&self.path, Options::new(), sym as i32, u32::from(pressed)),
            ),
        )
        .await
        .map_err(|_| "O portal de teclado não respondeu.".to_string())?
        .map_err(|error| error.to_string())
    }
}

async fn close_session(conn: &Connection, path: &OwnedObjectPath) {
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        if let Ok(proxy) =
            Proxy::new(conn, DEST, path.as_str(), "org.freedesktop.portal.Session").await
        {
            let _ = proxy.call::<_, _, ()>("Close", &()).await;
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wayland_wins_even_with_an_xwayland_display() {
        assert!(wayland_session(Some("wayland"), Some("wayland-0")));
        assert!(wayland_session(None, Some("wayland-0")));
        assert!(!wayland_session(Some("x11"), Some("wayland-0")));
        assert!(!wayland_session(None, None));
        assert!(!wayland_session(None, Some("")));
    }
    #[test]
    fn remote_input_cannot_open_a_permission_dialog() {
        assert!(matches!(
            send(Event::Motion(1, 1)),
            Err(PlatformError::Unavailable(_))
        ));
        assert!(!authorized());
    }
}
