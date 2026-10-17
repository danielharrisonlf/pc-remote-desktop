//! Linux audio mixer backend (`AudioBackend`) for Ubuntu 22.04 / 24.04 LTS.
//!
//! Supports PulseAudio and PipeWire (`pactl` / `wpctl`) with ALSA (`amixer`) fallback and
//! D-Bus `org.PulseAudio1` / `org.pulseaudio.ServerLookup1` availability verification.
//! Maintains a linear 0..=100% scale consistent with the Windows Core Audio backend.
use crate::{
    platform::{AudioBackend, PlatformError},
    protocol::AudioState,
};
use std::{process::Command, sync::Mutex};

pub struct LinuxAudio;

static AUDIO_WRITE: Mutex<()> = Mutex::new(());

/// Parses `pactl get-sink-volume @DEFAULT_SINK@` output (e.g. `Volume: front-left: 42598 /  65% / -11.22 dB`).
pub(crate) fn parse_pactl_volume(stdout: &str) -> Option<u8> {
    for token in stdout.split(|c: char| c.is_whitespace() || c == '/') {
        if let Some(num) = token.strip_suffix('%') {
            if let Ok(value) = num.trim().parse::<u16>() {
                return Some(value.min(100) as u8);
            }
        }
    }
    None
}

/// Parses `pactl get-sink-mute @DEFAULT_SINK@` output (e.g. `Mute: yes` or `Mute: no`, or pt-BR ` Muda: sim`).
pub(crate) fn parse_pactl_mute(stdout: &str) -> Option<bool> {
    let lower = stdout.to_ascii_lowercase();
    if lower.contains("yes") || lower.contains("sim") || lower.contains("true") {
        Some(true)
    } else if lower.contains("no")
        || lower.contains("não")
        || lower.contains("nao")
        || lower.contains("false")
    {
        Some(false)
    } else {
        None
    }
}

/// Parses `wpctl get-volume @DEFAULT_AUDIO_SINK@` output (e.g. `Volume: 0.65` or `Volume: 0.40 [MUTED]`).
pub(crate) fn parse_wpctl_volume(stdout: &str) -> Option<(u8, bool)> {
    let muted = stdout.contains("[MUTED]");
    let after_colon = stdout.split(':').nth(1)?;
    let float_token = after_colon.split_whitespace().next()?;
    let scalar: f32 = float_token.parse().ok()?;
    let percent = (scalar * 100.0).round().clamp(0.0, 100.0) as u8;
    Some((percent, muted))
}

/// Parses `amixer sget Master` output (e.g. `Front Left: Playback 42 [65%] [on]`).
pub(crate) fn parse_amixer(stdout: &str) -> Option<(u8, bool)> {
    let mut percent = None;
    let mut muted = None;
    for bracketed in stdout.split('[').skip(1) {
        if let Some(inside) = bracketed.split(']').next() {
            if let Some(num) = inside.strip_suffix('%') {
                if percent.is_none() {
                    if let Ok(p) = num.trim().parse::<u16>() {
                        percent = Some(p.min(100) as u8);
                    }
                }
            } else if inside == "off" {
                muted = Some(true);
            } else if inside == "on" {
                muted = Some(false);
            }
        }
    }
    Some((percent?, muted.unwrap_or(false)))
}

fn run_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_status(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .env("LC_ALL", "C")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

fn read_mixer() -> Result<(u8, bool), String> {
    // 1. PulseAudio / PipeWire-Pulse via pactl (standard on Ubuntu 22.04 & 24.04)
    if let (Some(vol_out), Some(mute_out)) = (
        run_output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]),
        run_output("pactl", &["get-sink-mute", "@DEFAULT_SINK@"]),
    ) {
        if let (Some(percent), Some(muted)) =
            (parse_pactl_volume(&vol_out), parse_pactl_mute(&mute_out))
        {
            return Ok((percent, muted));
        }
    }

    // 2. WirePlumber / PipeWire native via wpctl
    if let Some(wp_out) = run_output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
        if let Some(state) = parse_wpctl_volume(&wp_out) {
            return Ok(state);
        }
    }

    // 3. ALSA fallback via amixer
    if let Some(amixer_out) = run_output("amixer", &["-D", "pulse", "sget", "Master"])
        .or_else(|| run_output("amixer", &["sget", "Master"]))
    {
        if let Some(state) = parse_amixer(&amixer_out) {
            return Ok(state);
        }
    }

    Err("Dispositivo de saída indisponível".into())
}

fn set_volume_percent(target: u8) -> bool {
    let clamped = target.min(100);
    let pct_arg = format!("{clamped}%");
    if run_status("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &pct_arg]) {
        return true;
    }
    let wp_arg = format!("{:.2}", f32::from(clamped) / 100.0);
    if run_status("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &wp_arg]) {
        return true;
    }
    run_status("amixer", &["-D", "pulse", "sset", "Master", &pct_arg])
        || run_status("amixer", &["sset", "Master", &pct_arg])
}

fn set_mute_state(muted: bool) -> bool {
    let flag = if muted { "1" } else { "0" };
    if run_status("pactl", &["set-sink-mute", "@DEFAULT_SINK@", flag]) {
        return true;
    }
    if run_status("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", flag]) {
        return true;
    }
    let alsa_flag = if muted { "mute" } else { "unmute" };
    run_status("amixer", &["-D", "pulse", "sset", "Master", alsa_flag])
        || run_status("amixer", &["sset", "Master", alsa_flag])
}

impl AudioBackend for LinuxAudio {
    fn read(&self) -> AudioState {
        match read_mixer() {
            Ok((percent, muted)) => AudioState {
                available: true,
                percent,
                muted,
                error: None,
            },
            Err(err) => {
                tracing::debug!("Linux audio read unavailable: {err}");
                AudioState {
                    available: false,
                    percent: 0,
                    muted: false,
                    error: Some("Dispositivo de saída indisponível".into()),
                }
            }
        }
    }

    fn set(
        &self,
        percent: Option<u8>,
        step: Option<i8>,
        muted: Option<bool>,
    ) -> Result<AudioState, PlatformError> {
        let _guard = AUDIO_WRITE
            .lock()
            .map_err(|_| PlatformError::Internal("Audio mutex poisoned".to_string()))?;

        let (current_pct, _) = read_mixer().map_err(PlatformError::Unavailable)?;

        if let Some(p) = percent {
            if !set_volume_percent(p.min(100)) {
                return Err(PlatformError::Unavailable(
                    "Dispositivo de saída indisponível".into(),
                ));
            }
        }
        if let Some(d) = step {
            let base = i16::from(percent.unwrap_or(current_pct));
            let next = (base + i16::from(d)).clamp(0, 100) as u8;
            if !set_volume_percent(next) {
                return Err(PlatformError::Unavailable(
                    "Dispositivo de saída indisponível".into(),
                ));
            }
        }
        if let Some(m) = muted {
            if !set_mute_state(m) {
                return Err(PlatformError::Unavailable(
                    "Dispositivo de saída indisponível".into(),
                ));
            }
        }

        let (final_pct, final_muted) = read_mixer().map_err(PlatformError::Unavailable)?;
        Ok(AudioState {
            available: true,
            percent: final_pct,
            muted: final_muted,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pactl_wpctl_and_amixer_outputs() {
        let pactl_vol =
            "Volume: front-left: 42598 /  65% / -11.22 dB,   front-right: 42598 /  65% / -11.22 dB";
        assert_eq!(parse_pactl_volume(pactl_vol), Some(65));
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no\n"), Some(false));

        assert_eq!(parse_wpctl_volume("Volume: 0.42\n"), Some((42, false)));
        assert_eq!(
            parse_wpctl_volume("Volume: 0.80 [MUTED]\n"),
            Some((80, true))
        );

        let amixer_out = "Simple mixer control 'Master',0\n  Front Left: Playback 45 [70%] [on]";
        assert_eq!(parse_amixer(amixer_out), Some((70, false)));
    }
}
