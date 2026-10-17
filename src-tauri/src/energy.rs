//! power.v1: lock, sleep, shut down or restart the PC from the phone, now or at a time.
//!
//! One schedule per PC, kept in RAM only. "Interromper" refuses new actions and drops
//! the schedule; so does revoking the phone that made it. Windows is never forced:
//! programs with unsaved work may still hold a shutdown and show their own prompt.
use crate::{
    core::{now_ms, Core, Runtime},
    protocol::{PowerAction, PowerState, ScheduledPower, MAX_POWER_DELAY},
};
use serde::Serialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

/// A `power.run` with 0 waits this long, so its ack leaves before Windows acts.
pub const IMMEDIATE_DELAY: Duration = Duration::from_millis(500);
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_LIMIT: usize = 3;
/// Schedules of at least 2 min get a second notification 60 s before.
const WARNING: Duration = Duration::from_secs(60);
const WARN_FROM: u32 = 120;

/// The Windows side, injected so tests never lock, sleep or shut down the machine.
pub trait PowerSystem: Send + Sync {
    fn sleep_supported(&self) -> bool;
    /// Blocks while the PC sleeps; call it from its own thread.
    fn run(&self, action: PowerAction) -> Result<(), String>;
}

pub struct Scheduled {
    pub action: PowerAction,
    pub due: Instant,
    /// Unix milliseconds, for the panel.
    pub due_at: u64,
    pub device: Uuid,
    pub device_name: String,
    /// "às 23:40" or "amanhã às 01:10", in local time when it was scheduled.
    pub when: String,
    /// The 60 s notice is still to be shown.
    pub warn: bool,
}

/// The schedule as the panel shows it.
#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
pub struct PowerScheduleView {
    pub action: PowerAction,
    #[serde(rename = "dueAt")]
    pub due_at: u64,
    #[serde(rename = "deviceName")]
    pub device_name: String,
}

/// What `power.run` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Accepted {
    /// Run after the ack (see IMMEDIATE_DELAY).
    Now(PowerAction),
    Scheduled,
}

/// "Desligar", "Suspender"… as the start of a sentence.
pub fn verb(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Lock => "Bloquear",
        PowerAction::Sleep => "Suspender",
        PowerAction::Shutdown => "Desligar",
        PowerAction::Restart => "Reiniciar",
    }
}

fn noun(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Lock => "bloqueio",
        PowerAction::Sleep => "suspensão",
        PowerAction::Shutdown => "desligamento",
        PowerAction::Restart => "reinício",
    }
}

fn scheduled_title(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Lock => "Bloqueio agendado",
        PowerAction::Sleep => "Suspensão agendada",
        PowerAction::Shutdown => "Desligamento agendado",
        PowerAction::Restart => "Reinício agendado",
    }
}

const HOW_TO_CANCEL: &str = "Para cancelar, use o PC Remote ou o ícone da bandeja.";

/// "às 23:40" from a local clock time plus `delay` seconds; "amanhã às …" past midnight.
pub(crate) fn clock_from(hour: u16, minute: u16, second: u16, delay: u32) -> String {
    let total =
        u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(second) + u64::from(delay);
    let rest = total % 86_400;
    let text = format!("{:02}:{:02}", rest / 3600, rest % 3600 / 60);
    if total >= 86_400 {
        format!("amanhã às {text}")
    } else {
        format!("às {text}")
    }
}

fn clock_after(delay: u32) -> String {
    let (hour, minute, second) = crate::platform::system().local_hms();
    clock_from(hour, minute, second, delay)
}

fn remaining(due: Instant) -> u32 {
    let left = due.saturating_duration_since(Instant::now());
    let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    seconds.clamp(1, u64::from(MAX_POWER_DELAY)) as u32
}

impl Runtime {
    pub(crate) fn power_state(&self) -> Option<PowerState> {
        self.power_feature.then(|| PowerState {
            allowed: !self.settings.input_blocked,
            scheduled: self.power_schedule.as_ref().map(|p| ScheduledPower {
                action: p.action,
                remaining_seconds: remaining(p.due),
            }),
        })
    }
    pub(crate) fn power_view(&self) -> Option<PowerScheduleView> {
        self.power_schedule.as_ref().map(|p| PowerScheduleView {
            action: p.action,
            due_at: p.due_at,
            device_name: p.device_name.clone(),
        })
    }
    /// Drops the schedule; the caller publishes the state.
    pub(crate) fn drop_power_schedule(&mut self) -> bool {
        let had = self.power_schedule.take().is_some();
        if had {
            self.revision += 1;
        }
        had
    }
    fn power_rate_ok(&mut self) -> bool {
        let now = Instant::now();
        while self
            .power_events
            .front()
            .is_some_and(|at| now.duration_since(*at) >= RATE_WINDOW)
        {
            self.power_events.pop_front();
        }
        if self.power_events.len() >= RATE_LIMIT {
            return false;
        }
        self.power_events.push_back(now);
        true
    }
}

impl Core {
    pub fn set_power_system(&self, system: Box<dyn PowerSystem>) {
        let _ = self.power_system.set(system);
    }

    fn notify(&self, title: &str, body: &str) {
        if let Some(notify) = self.notifier.get() {
            notify(title, body);
        }
    }

    /// `power.run` after the field rules: `Now` runs after the ack, a delay replaces the schedule.
    pub fn power_run(
        &self,
        device: Uuid,
        epoch: u64,
        action: PowerAction,
        delay: u32,
    ) -> Result<Accepted, String> {
        let sleep_ok = action != PowerAction::Sleep
            || self
                .power_system
                .get()
                .is_some_and(|system| system.sleep_supported());
        let mut s = self.state.lock().unwrap();
        if !s.session_ok(device, epoch) {
            return Err("unauthorized".into());
        }
        if !s.power_feature || s.settings.input_blocked {
            return Err("unavailable".into());
        }
        if !s.power_rate_ok() {
            return Err("rate_limited".into());
        }
        if !sleep_ok {
            return Err("unavailable".into());
        }
        if delay == 0 {
            if s.drop_power_schedule() {
                self.publish_state_locked(&s);
            }
            return Ok(Accepted::Now(action));
        }
        let device_name = s
            .secrets
            .devices
            .iter()
            .find(|d| d.id == device)
            .map_or_else(|| "celular".to_string(), |d| d.name.clone());
        let when = clock_after(delay);
        s.power_schedule = Some(Scheduled {
            action,
            due: Instant::now() + Duration::from_secs(delay.into()),
            due_at: now_ms() + u64::from(delay) * 1000,
            device,
            device_name: device_name.clone(),
            when: when.clone(),
            warn: delay >= WARN_FROM,
        });
        s.revision += 1;
        self.publish_state_locked(&s);
        drop(s);
        self.notify(
            scheduled_title(action),
            &format!(
                "{} {when}, pedido por {device_name}. {HOW_TO_CANCEL}",
                verb(action)
            ),
        );
        Ok(Accepted::Scheduled)
    }

    /// `power.cancel`, the panel and the tray. True when something was scheduled.
    pub fn power_cancel(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        let had = s.drop_power_schedule();
        if had {
            self.publish_state_locked(&s);
        }
        had
    }

    /// Tray label of the schedule, `None` when nothing is scheduled.
    pub fn power_label(&self) -> Option<String> {
        let s = self.state.lock().unwrap();
        s.power_schedule
            .as_ref()
            .map(|p| format!("Cancelar {} ({})", noun(p.action), p.when))
    }

    /// Poll tick: the 60 s notice, then the action itself at the time.
    pub fn power_tick(self: &Arc<Self>) {
        let mut s = self.state.lock().unwrap();
        let Some(p) = s.power_schedule.as_mut() else {
            return;
        };
        let now = Instant::now();
        if p.due > now {
            if p.warn && p.due.duration_since(now) <= WARNING {
                p.warn = false;
                let action = p.action;
                drop(s);
                self.notify(
                    scheduled_title(action),
                    &format!(
                        "O PC vai {} em 1 minuto. {HOW_TO_CANCEL}",
                        verb(action).to_lowercase()
                    ),
                );
            }
            return;
        }
        let action = p.action;
        s.drop_power_schedule();
        self.publish_state_locked(&s);
        let blocked = s.settings.input_blocked;
        drop(s);
        if blocked {
            self.diagnostic(
                "Ação de energia agendada descartada: o controle pelo celular está bloqueado.",
            );
            return;
        }
        self.power_execute(action);
    }

    /// Runs the action on its own thread; sleep returns only after the PC wakes up.
    pub fn power_execute(self: &Arc<Self>, action: PowerAction) {
        let core = self.clone();
        std::thread::spawn(move || {
            if core.state.lock().unwrap().settings.input_blocked {
                core.diagnostic(
                    "Ação de energia descartada: o controle pelo celular está bloqueado.",
                );
                return;
            }
            let result = match core.power_system.get() {
                Some(system) => system.run(action),
                None => Err("indisponível".into()),
            };
            if let Err(error) = result {
                let message = format!(
                    "Não foi possível {} o PC: {error}",
                    verb(action).to_lowercase()
                );
                core.diagnostic(&message);
                core.notify("Ação de energia não concluída", &message);
            }
        });
    }
}

/// The active platform's power system implementation. Only the app wires this in (lib.rs); tests use their own.
pub use crate::platform::PlatformPowerSystem as Windows;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        core::Outgoing,
        protocol::Features,
        secrets::{self, Device, Secrets},
    };
    use std::sync::Mutex;

    /// Records actions instead of running them.
    pub(crate) struct Recorder {
        pub ran: Arc<Mutex<Vec<PowerAction>>>,
        pub sleeps: bool,
        pub fails: bool,
    }

    impl PowerSystem for Recorder {
        fn sleep_supported(&self) -> bool {
            self.sleeps
        }
        fn run(&self, action: PowerAction) -> Result<(), String> {
            self.ran.lock().unwrap().push(action);
            if self.fails {
                Err("recusado".into())
            } else {
                Ok(())
            }
        }
    }

    type Shown = Arc<Mutex<Vec<(String, String)>>>;
    type Ran = Arc<Mutex<Vec<PowerAction>>>;
    /// Core, phone id, token, epoch, actions run and notifications shown.
    type PowerSession = (Arc<Core>, Uuid, String, u64, Ran, Shown);

    pub(crate) fn power_core(power: bool, sleeps: bool) -> PowerSession {
        let core = Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        );
        let id = Uuid::new_v4();
        let token = crate::core::random_hex();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id,
            name: "Pixel de teste".into(),
            token_hash: secrets::token_hash(&token),
        });
        let ran = Arc::new(Mutex::new(Vec::new()));
        core.set_power_system(Box::new(Recorder {
            ran: ran.clone(),
            sleeps,
            fails: false,
        }));
        let shown: Shown = Arc::new(Mutex::new(Vec::new()));
        let sink = shown.clone();
        core.set_notifier(Box::new(move |title, body| {
            sink.lock().unwrap().push((title.into(), body.into()));
        }));
        let features = Features {
            input: true,
            power,
            ..Features::default()
        };
        let epoch = core.connect_authenticated(id, &token, features).unwrap();
        (core, id, token, epoch, ran, shown)
    }

    fn wait_for(ran: &Arc<Mutex<Vec<PowerAction>>>, count: usize) {
        for _ in 0..200 {
            if ran.lock().unwrap().len() >= count {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("power action did not run");
    }

    fn snapshots(events: &mut tokio::sync::broadcast::Receiver<Outgoing>) -> usize {
        let mut count = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(&event, Outgoing::Wire { message, .. } if message["type"] == "state") {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn local_clock_labels_cross_midnight() {
        assert_eq!(clock_from(23, 10, 5, 1800), "às 23:40");
        assert_eq!(clock_from(23, 50, 0, 1200), "amanhã às 00:10");
        assert_eq!(clock_from(0, 0, 59, 0), "às 00:00");
        assert_eq!(clock_from(12, 0, 0, MAX_POWER_DELAY), "amanhã às 12:00");
    }

    #[test]
    fn snapshot_carries_power_only_when_negotiated() {
        let (core, ..) = power_core(false, true);
        assert!(core.snapshot().power.is_none());
        let wire = serde_json::to_value(core.snapshot()).unwrap();
        assert!(wire.get("power").is_none());
        let (core, ..) = power_core(true, true);
        assert_eq!(
            core.snapshot().power,
            Some(PowerState {
                allowed: true,
                scheduled: None
            })
        );
    }

    #[test]
    fn schedule_replaces_cancels_and_reports_remaining_seconds() {
        let (core, id, _, epoch, ran, shown) = power_core(true, true);
        let mut events = core.events.subscribe();
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Shutdown, 1800),
            Ok(Accepted::Scheduled)
        );
        assert_eq!(snapshots(&mut events), 1);
        let scheduled = core.snapshot().power.unwrap().scheduled.unwrap();
        assert_eq!(scheduled.action, PowerAction::Shutdown);
        assert!((1799..=1800).contains(&scheduled.remaining_seconds));
        let view = core.status().power_scheduled.unwrap();
        assert_eq!(view.device_name, "Pixel de teste");
        assert!(view.due_at > now_ms());
        let (title, body) = shown.lock().unwrap()[0].clone();
        assert_eq!(title, "Desligamento agendado");
        assert!(
            body.starts_with("Desligar às ") || body.starts_with("Desligar amanhã às "),
            "{body}"
        );
        assert!(body.contains("pedido por Pixel de teste"), "{body}");
        assert!(core
            .power_label()
            .unwrap()
            .starts_with("Cancelar desligamento ("));
        // A new schedule replaces the old one.
        core.power_run(id, epoch, PowerAction::Sleep, 600).unwrap();
        assert_eq!(
            core.snapshot().power.unwrap().scheduled.unwrap().action,
            PowerAction::Sleep
        );
        assert_eq!(snapshots(&mut events), 1);
        assert!(core.power_cancel());
        assert_eq!(snapshots(&mut events), 1);
        assert!(!core.power_cancel());
        assert!(core.snapshot().power.unwrap().scheduled.is_none());
        assert!(core.power_label().is_none());
        assert!(ran.lock().unwrap().is_empty());
    }

    #[test]
    fn immediate_runs_drop_the_schedule_and_the_tick_runs_due_actions() {
        let (core, id, _, epoch, ran, shown) = power_core(true, true);
        core.power_run(id, epoch, PowerAction::Restart, 3600)
            .unwrap();
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Lock, 0),
            Ok(Accepted::Now(PowerAction::Lock))
        );
        assert!(core.status().power_scheduled.is_none());
        core.power_execute(PowerAction::Lock);
        wait_for(&ran, 1);
        // Due now: the tick runs it once and clears it.
        core.state.lock().unwrap().power_events.clear();
        core.power_run(id, epoch, PowerAction::Shutdown, 120)
            .unwrap();
        core.power_tick();
        assert!(core.status().power_scheduled.is_some());
        {
            let mut s = core.state.lock().unwrap();
            let p = s.power_schedule.as_mut().unwrap();
            p.due = Instant::now() + Duration::from_secs(30);
        }
        core.power_tick();
        let notices = shown.lock().unwrap().clone();
        assert!(
            notices
                .last()
                .unwrap()
                .1
                .starts_with("O PC vai desligar em 1 minuto."),
            "{notices:?}"
        );
        core.state
            .lock()
            .unwrap()
            .power_schedule
            .as_mut()
            .unwrap()
            .due = Instant::now();
        core.power_tick();
        wait_for(&ran, 2);
        assert_eq!(
            *ran.lock().unwrap(),
            vec![PowerAction::Lock, PowerAction::Shutdown]
        );
        assert!(core.status().power_scheduled.is_none());
        core.power_tick();
        assert_eq!(ran.lock().unwrap().len(), 2);
    }

    #[test]
    fn interromper_blocks_and_cancels_and_revocation_cancels_its_own_schedule() {
        let (core, id, token, epoch, ran, _) = power_core(true, true);
        core.power_run(id, epoch, PowerAction::Shutdown, 900)
            .unwrap();
        core.stop_input_with_save(|_| Ok(())).unwrap();
        let power = core.snapshot().power.unwrap();
        assert!(!power.allowed && power.scheduled.is_none());
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Lock, 0),
            Err("unavailable".into())
        );
        // Cancel is always accepted, even blocked.
        assert!(!core.power_cancel());
        // An action that was already on its way is dropped once blocked.
        core.power_execute(PowerAction::Lock);
        std::thread::sleep(Duration::from_millis(100));
        assert!(ran.lock().unwrap().is_empty());
        core.set_input_blocked_with_save(false, |_| Ok(())).unwrap();
        assert!(core.snapshot().power.unwrap().allowed);
        core.power_run(id, epoch, PowerAction::Restart, 900)
            .unwrap();
        // Another phone's schedule survives a revocation; the one that asked for it does not.
        let other = Uuid::new_v4();
        core.state.lock().unwrap().secrets.devices.push(Device {
            id: other,
            name: "Outro".into(),
            token_hash: secrets::token_hash(&crate::core::random_hex()),
        });
        core.revoke_with_save(other, |_| Ok(())).unwrap();
        assert!(core.status().power_scheduled.is_some());
        core.revoke_with_save(id, |_| Ok(())).unwrap();
        assert!(core.status().power_scheduled.is_none());
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Lock, 0),
            Err("unauthorized".into())
        );
        let _ = token;
    }

    #[test]
    fn rate_limit_negotiation_and_sleep_support() {
        let (core, id, _, epoch, ..) = power_core(false, true);
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Lock, 0),
            Err("unavailable".into())
        );
        let (core, id, _, epoch, ..) = power_core(true, false);
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Sleep, 0),
            Err("unavailable".into())
        );
        let (core, id, _, epoch, ..) = power_core(true, true);
        for _ in 0..RATE_LIMIT {
            core.power_run(id, epoch, PowerAction::Sleep, 60).unwrap();
        }
        assert_eq!(
            core.power_run(id, epoch, PowerAction::Sleep, 60),
            Err("rate_limited".into())
        );
        // power.cancel is not rate limited.
        assert!(core.power_cancel());
    }

    #[test]
    fn failures_become_a_diagnostic_and_a_notification() {
        let core = Core::ephemeral(
            Secrets {
                pc_id: Uuid::new_v4(),
                certificate_pem: String::new(),
                private_key_pem: String::new(),
                devices: Vec::new(),
            },
            "00".into(),
        );
        let ran = Arc::new(Mutex::new(Vec::new()));
        core.set_power_system(Box::new(Recorder {
            ran: ran.clone(),
            sleeps: true,
            fails: true,
        }));
        let shown: Shown = Arc::new(Mutex::new(Vec::new()));
        let sink = shown.clone();
        core.set_notifier(Box::new(move |title, body| {
            sink.lock().unwrap().push((title.into(), body.into()));
        }));
        core.power_execute(PowerAction::Shutdown);
        wait_for(&ran, 1);
        for _ in 0..200 {
            if !shown.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            shown.lock().unwrap()[0],
            (
                "Ação de energia não concluída".to_string(),
                "Não foi possível desligar o PC: recusado".to_string()
            )
        );
        assert!(core
            .status()
            .diagnostics
            .iter()
            .any(|entry| entry.message == "Não foi possível desligar o PC: recusado"));
    }

    /// Read-only query: nothing is locked, suspended or shut down.
    #[test]
    fn capability_query_is_harmless() {
        let _ = Windows.sleep_supported();
    }
}
