//! clipboard.v2: images in the clipboard, with the rules of `clipboard.set`/`clipboard.event`.
//!
//! PC → phone: a copy without text becomes a PNG pushed with files.v2 (`file.push` with a
//! `clipboard` field). Phone → PC: an offer with a `clipboard` field whose bytes go to the
//! Windows clipboard instead of Downloads. Text always wins over an image.
use crate::{
    clipboard,
    core::{Core, Runtime, TransferDirection},
    files::{Accepted, SafeName},
    outbox::{ClipPush, OutItem},
    wic::{self, ImageError},
};
use uuid::Uuid;

/// Writes an image from the phone: (PNG, CF_DIBV5, expected sequence) → new sequence.
pub type ImageWriter = Box<dyn Fn(&[u8], &[u8], Option<u32>) -> Result<u32, String> + Send + Sync>;

/// Name of every clipboard push; the phone keeps only the latest one.
pub const PUSH_STEM: &str = "Imagem do PC";

impl Runtime {
    /// Automatic copies go to the phone only with "Sincronização em tempo real" on and
    /// the phone asking for them (`sync.set`).
    pub(crate) fn download_permitted(&self) -> bool {
        self.settings.realtime
            && self.client_sync.download
            && !self.client_sync.paused
            && self
                .connected
                .is_some_and(|id| !self.revoked_devices.contains(&id))
    }

    /// Consent and ordering of an image from the phone. `base_revision` is checked at the
    /// offer; at the end the clipboard sequence stands in for it.
    pub(crate) fn clipboard_image_consent(
        &self,
        event_id: Uuid,
        automatic: bool,
        base_revision: Option<u64>,
    ) -> Result<(), &'static str> {
        let consent = self.settings.realtime && self.client_sync.upload && !self.client_sync.paused;
        if automatic && !consent {
            return Err("unavailable");
        }
        let stale = automatic && base_revision.is_some_and(|base| base != self.clipboard_revision);
        if self.event_set.contains(&event_id) || stale {
            return Err("conflict");
        }
        Ok(())
    }

    fn remember_event(&mut self, event_id: Uuid) {
        self.event_set.insert(event_id);
        self.event_ids.push_back(event_id);
        if self.event_ids.len() > 256 {
            if let Some(old) = self.event_ids.pop_front() {
                self.event_set.remove(&old);
            }
        }
    }
}

impl Core {
    pub fn set_image_writer(&self, writer: ImageWriter) {
        let _ = self.image_writer.set(writer);
    }

    /// Listener thread: the copy `sequence` has no text. Reads and converts the image
    /// without the state lock, then queues it if nothing newer was copied meanwhile.
    pub(crate) fn clipboard_image_local(
        &self,
        sequence: u32,
        device: Uuid,
        epoch: u64,
        revision: u64,
    ) {
        let raw = match clipboard::read_image() {
            Ok(Some(raw)) => raw,
            Ok(None) => return,
            Err(code) if code == "too_large" => {
                self.diagnostic(
                    "Imagem copiada no PC não foi ao celular: passa de 32 MB ou 50 megapixels.",
                );
                return;
            }
            // Sensitive content and a busy clipboard are skipped silently, like text.
            Err(_) => return,
        };
        match wic::png_for_phone(raw) {
            Ok(png) => {
                let _ =
                    self.push_clipboard_image(png, device, epoch, revision, true, Some(sequence));
            }
            Err(ImageError::TooLarge) => self.diagnostic(
                "Imagem copiada no PC não foi ao celular: passa de 32 MB ou 50 megapixels.",
            ),
            Err(ImageError::Invalid) => {
                self.diagnostic("Imagem copiada no PC não foi ao celular: formato não reconhecido.")
            }
        }
    }

    /// "Buscar do PC" (`clipboard.get`) when the clipboard holds an image and no text.
    pub(crate) fn clipboard_image_get(
        &self,
        device: Uuid,
        epoch: u64,
        revision: u64,
    ) -> Result<(), String> {
        let raw = clipboard::read_image()?.ok_or("unavailable")?;
        let png = wic::png_for_phone(raw).map_err(|error| {
            if error == ImageError::TooLarge {
                self.diagnostic(
                    "Imagem do PC não foi ao celular: passa de 32 MB ou 50 megapixels.",
                );
            }
            "unavailable".to_string()
        })?;
        self.push_clipboard_image(png, device, epoch, revision, false, None)
    }

    /// Queues `png` for the phone's clipboard, ahead of the files that wait. `sequence`
    /// (automatic copies) must still be the latest copy, or a newer one replaced it.
    pub(crate) fn push_clipboard_image(
        &self,
        png: Vec<u8>,
        device: Uuid,
        epoch: u64,
        revision: u64,
        automatic: bool,
        sequence: Option<u32>,
    ) -> Result<(), String> {
        let id = Uuid::new_v4();
        let path = self.clipboard_folder()?.join(format!("clip-{id}.png"));
        std::fs::write(&path, &png).map_err(|e| e.to_string())?;
        let mut s = self.state.lock().unwrap();
        let current = sequence.is_none_or(|sequence| s.clipboard_sequence == sequence);
        let allowed = s.session_ok(device, epoch)
            && s.clipboard_image
            && (!automatic || s.download_permitted());
        if !current || !allowed {
            drop(s);
            let _ = std::fs::remove_file(&path);
            return Err("unavailable".into());
        }
        let item = OutItem {
            id,
            path,
            name: SafeName {
                stem: PUSH_STEM.into(),
                ext: "png".into(),
            },
            size: png.len() as u64,
            clipboard: Some(ClipPush {
                revision,
                automatic,
            }),
        };
        s.enqueue_clipboard(item, &self.events);
        Ok(())
    }

    /// `file.end` of an image from the phone: decodes it and writes the clipboard.
    /// Blocking (WIC and the clipboard); the caller then answers `file.done` or the error.
    pub fn clipboard_image_received(
        &self,
        accepted: &Accepted,
        bytes: &[u8],
    ) -> Result<(), &'static str> {
        let target = accepted.clipboard.as_ref().ok_or("internal")?;
        let (png, dib) = wic::clipboard_formats(bytes).map_err(ImageError::code)?;
        self.file_check(accepted, accepted.size).map_err(|code| {
            if code == "cancelled" {
                "unavailable"
            } else {
                code
            }
        })?;
        let mut s = self.state.lock().unwrap();
        s.clipboard_image_consent(target.event_id, target.automatic, None)?;
        let writer = self.image_writer.get().ok_or("unavailable")?;
        let sequence = writer(
            &png,
            &dib,
            target.automatic.then_some(target.expected_sequence),
        )
        .map_err(|error| match error.as_str() {
            "conflict" => "conflict",
            "clipboard_busy" => "clipboard_busy",
            _ => "internal",
        })?;
        // The listener sees this write too; it never goes back to the phone.
        s.suppressed_sequence = sequence;
        s.clipboard_sequence = sequence;
        s.clipboard_revision += 1;
        s.remember_event(target.event_id);
        s.record_image(
            TransferDirection::ToPc,
            bytes.len() as u64,
            target.automatic,
        );
        s.revision += 1;
        self.publish_state_locked(&s);
        Ok(())
    }

    /// Leftovers of an earlier run: temporary images and parts in the clipboard folder.
    pub fn clean_clipboard_folder(&self) {
        let Ok(folder) = self.clipboard_folder() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let ours = entry.file_name().to_str().is_some_and(|name| {
                (name.starts_with("clip-") && name.ends_with(".png"))
                    || name.ends_with(".pcremote-part")
            });
            if ours && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{Outgoing, TransferKind},
        protocol::{ClipboardOffer, Features},
        secrets::{self, Device, Secrets},
    };
    use std::sync::{Arc, Mutex};

    type Written = Arc<Mutex<Vec<(Vec<u8>, Vec<u8>, Option<u32>)>>>;

    fn session(clipboard_image: bool) -> (Arc<Core>, Uuid, String, u64, Written) {
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
            name: "Imagens".into(),
            token_hash: secrets::token_hash(&token),
        });
        let written: Written = Arc::new(Mutex::new(Vec::new()));
        let sink = written.clone();
        core.set_image_writer(Box::new(move |png, dib, expected| {
            let mut all = sink.lock().unwrap();
            // A fake clipboard whose sequence is 1000 plus the number of writes.
            if expected.is_some_and(|expected| expected != 1000 + all.len() as u32) {
                return Err("conflict".into());
            }
            all.push((png.to_vec(), dib.to_vec(), expected));
            Ok(1000 + all.len() as u32)
        }));
        let features = Features {
            files: true,
            files_v2: true,
            clipboard_image,
            ..Features::default()
        };
        let epoch = core.connect_authenticated(id, &token, features).unwrap();
        core.set_client_sync(true, true, false);
        core.state.lock().unwrap().clipboard_sequence = 1000;
        (core, id, token, epoch, written)
    }

    fn pushes(events: &mut tokio::sync::broadcast::Receiver<Outgoing>) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let Outgoing::Wire { message, .. } = event {
                if message["type"] == "file.push" {
                    out.push(message);
                }
            }
        }
        out
    }

    fn offer(automatic: bool, base: u64) -> ClipboardOffer {
        ClipboardOffer {
            event_id: Uuid::new_v4(),
            automatic,
            base_revision: base,
        }
    }

    #[test]
    fn pushes_carry_the_clipboard_field_go_first_and_replace_each_other() {
        let (core, id, _, epoch, _) = session(true);
        let mut events = core.events.subscribe();
        let folder = crate::files::tests::temp_folder("clip-queue");
        let file = folder.join("relatorio.pdf");
        std::fs::write(&file, b"%PDF-1.7").unwrap();
        let other = folder.join("planilha.csv");
        std::fs::write(&other, b"a,b").unwrap();
        // A file is in flight and another waits.
        core.enqueue_outgoing(vec![file, other]);
        let first = pushes(&mut events);
        assert_eq!(first.len(), 1);
        assert!(first[0].get("clipboard").is_none());
        let png = crate::wic::tests::sample_png();
        core.push_clipboard_image(png.clone(), id, epoch, 5, true, None)
            .unwrap();
        // It does not interrupt the file in flight and is hidden from the panel.
        assert!(pushes(&mut events).is_empty());
        let status = core.status();
        assert_eq!(status.outgoing.unwrap().name, "relatorio.pdf");
        assert_eq!(status.outgoing_queue, vec!["planilha.csv".to_string()]);
        // A newer copy replaces the queued image.
        core.push_clipboard_image(png.clone(), id, epoch, 6, true, None)
            .unwrap();
        let temp = core.clipboard_folder().unwrap();
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 1);
        // The file in flight ends: the image goes before the waiting file.
        let transfer: Uuid = first[0]["transferId"].as_str().unwrap().parse().unwrap();
        core.outgoing_refused(id, epoch, transfer, "Sem espaço no celular.");
        let next = pushes(&mut events);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0]["name"], "Imagem do PC.png");
        assert_eq!(next[0]["mime"], "image/png");
        assert_eq!(next[0]["size"], png.len());
        assert_eq!(
            next[0]["clipboard"],
            serde_json::json!({"revision":6,"automatic":true})
        );
        assert!(core.status().outgoing.is_none());
        // The phone fetches and confirms it: recorded as an image, not as a file.
        let image: Uuid = next[0]["transferId"].as_str().unwrap().parse().unwrap();
        let fetched = core
            .outgoing_fetch(image, next[0]["ticket"].as_str().unwrap())
            .unwrap();
        assert!(fetched.clipboard);
        core.outgoing_finish(&fetched, Ok("Imagem do PC.png".into()));
        let status = core.status();
        assert_eq!(status.transfers[0].kind, TransferKind::Image);
        assert_eq!(status.transfers[0].bytes, png.len() as u64);
        assert_eq!(status.transfers[0].direction, TransferDirection::ToPhone);
        assert!(status
            .file_history
            .iter()
            .all(|f| f.name != "Imagem do PC.png"));
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);
        // Then the waiting file.
        assert_eq!(pushes(&mut events)[0]["name"], "planilha.csv");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn newer_copies_and_session_end_drop_announced_or_sending_images() {
        let (core, id, token, epoch, _) = session(true);
        let mut events = core.events.subscribe();
        let png = crate::wic::tests::sample_png();
        core.push_clipboard_image(png.clone(), id, epoch, 1, true, None)
            .unwrap();
        let push = pushes(&mut events).remove(0);
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        let ticket = push["ticket"].as_str().unwrap().to_string();
        // Announced: a newer copy makes the ticket worthless.
        core.state.lock().unwrap().drop_clipboard_push();
        assert_eq!(
            core.outgoing_fetch(transfer, &ticket).err(),
            Some("unauthorized")
        );
        // Sending: the socket is told "unavailable" (by_user) and nothing reaches the panel.
        core.push_clipboard_image(png.clone(), id, epoch, 2, true, None)
            .unwrap();
        let push = pushes(&mut events).remove(0);
        let transfer: Uuid = push["transferId"].as_str().unwrap().parse().unwrap();
        let fetched = core
            .outgoing_fetch(transfer, push["ticket"].as_str().unwrap())
            .unwrap();
        core.state.lock().unwrap().drop_clipboard_push();
        assert!(fetched
            .abort
            .by_user
            .load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(core.outgoing_check(&fetched, 1), Err("cancelled"));
        assert!(core.status().outgoing_result.is_none());
        // A stale automatic copy (another copy came after it) is not queued.
        core.state.lock().unwrap().clipboard_sequence = 77;
        assert!(core
            .push_clipboard_image(png.clone(), id, epoch, 3, true, Some(76))
            .is_err());
        // A new connection never receives the image of the previous one.
        core.push_clipboard_image(png.clone(), id, epoch, 3, true, Some(77))
            .unwrap();
        pushes(&mut events);
        let features = Features {
            files: true,
            files_v2: true,
            clipboard_image: true,
            ..Features::default()
        };
        core.connect_authenticated(id, &token, features).unwrap();
        core.outgoing_connected();
        assert!(pushes(&mut events).is_empty());
        assert_eq!(
            std::fs::read_dir(core.clipboard_folder().unwrap())
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn automatic_pushes_follow_realtime_and_need_clipboard_v2() {
        let (core, id, _, epoch, _) = session(false);
        let png = crate::wic::tests::sample_png();
        assert!(core
            .push_clipboard_image(png.clone(), id, epoch, 1, false, None)
            .is_err());
        let (core, id, _, epoch, _) = session(true);
        core.state.lock().unwrap().settings.realtime = false;
        assert!(core
            .push_clipboard_image(png.clone(), id, epoch, 1, true, None)
            .is_err());
        // "Buscar do PC" does not depend on the switch.
        assert!(core
            .push_clipboard_image(png, id, epoch, 1, false, None)
            .is_ok());
    }

    #[test]
    fn offers_follow_consent_dedup_and_base_revision() {
        let (core, id, _, epoch, _) = session(true);
        let ticket = |offer: &ClipboardOffer, mime: &str, size: u64| {
            core.file_offer(
                id,
                epoch,
                Uuid::new_v4(),
                "Imagem.png",
                size,
                mime,
                Some(offer),
            )
        };
        assert_eq!(ticket(&offer(true, 0), "image/gif", 10), Err("file_type"));
        assert_eq!(
            ticket(&offer(true, 0), "image/png", 33_554_433),
            Err("too_large")
        );
        assert_eq!(ticket(&offer(true, 3), "image/png", 10), Err("conflict"));
        core.state.lock().unwrap().settings.realtime = false;
        assert_eq!(ticket(&offer(true, 0), "image/png", 10), Err("unavailable"));
        // Manual sends ignore the switch and the base revision.
        assert!(ticket(&offer(false, 9), "image/jpeg", 10).is_ok());
        let seen = Uuid::new_v4();
        core.state.lock().unwrap().event_set.insert(seen);
        core.state.lock().unwrap().file_slot = None;
        let duplicate = ClipboardOffer {
            event_id: seen,
            automatic: false,
            base_revision: 0,
        };
        assert_eq!(ticket(&duplicate, "image/png", 10), Err("conflict"));
        // Without clipboard.v2 the field is unknown.
        let (core, id, _, epoch, _) = session(false);
        assert_eq!(
            core.file_offer(
                id,
                epoch,
                Uuid::new_v4(),
                "a.png",
                10,
                "image/png",
                Some(&offer(false, 0))
            ),
            Err("invalid_message")
        );
    }

    #[test]
    fn received_images_reach_the_clipboard_once_and_never_echo() {
        let (core, id, _, epoch, written) = session(true);
        let png = crate::wic::tests::sample_png();
        let clip = offer(true, 0);
        let transfer = Uuid::new_v4();
        let ticket = core
            .file_offer(
                id,
                epoch,
                transfer,
                "Imagem.png",
                png.len() as u64,
                "image/png",
                Some(&clip),
            )
            .unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        assert!(
            core.status().incoming_file.is_none(),
            "images are not listed as files"
        );
        assert!(accepted.folder.ends_with("clipboard-temp"));
        let mut events = core.events.subscribe();
        core.clipboard_image_received(&accepted, &png).unwrap();
        let (kept, dib, expected) = written.lock().unwrap()[0].clone();
        assert_eq!(kept, png);
        assert_eq!(&dib[..4], &124u32.to_le_bytes());
        assert_eq!(expected, Some(1000));
        let s = core.state.lock().unwrap();
        assert_eq!(
            (
                s.suppressed_sequence,
                s.clipboard_sequence,
                s.clipboard_revision
            ),
            (1001, 1001, 1)
        );
        assert_eq!(s.transfers.back().unwrap().kind, TransferKind::Image);
        assert!(s.event_set.contains(&clip.event_id));
        drop(s);
        assert!(
            matches!(events.try_recv(), Ok(Outgoing::Wire { message, .. }) if message["type"] == "state")
        );
        // Its own write is not a new copy for the phone.
        core.clipboard_event_local(1001);
        assert!(events.try_recv().is_err());
        // The same event again is a conflict; garbage is file_type.
        assert_eq!(
            core.clipboard_image_received(&accepted, &png),
            Err("conflict")
        );
        core.file_release(&accepted.abort);
        let transfer = Uuid::new_v4();
        let ticket = core
            .file_offer(
                id,
                epoch,
                transfer,
                "x.png",
                16,
                "image/png",
                Some(&offer(false, 0)),
            )
            .unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        let mut fake = crate::wic::PNG_SIGNATURE.to_vec();
        fake.extend_from_slice(b"no image");
        assert_eq!(
            core.clipboard_image_received(&accepted, &fake),
            Err("file_type")
        );
        // An automatic image based on a clipboard that changed meanwhile is refused.
        core.file_release(&accepted.abort);
        let transfer = Uuid::new_v4();
        let ticket = core
            .file_offer(
                id,
                epoch,
                transfer,
                "x.png",
                png.len() as u64,
                "image/png",
                Some(&offer(true, 1)),
            )
            .unwrap();
        let accepted = core.file_begin(transfer, &ticket).unwrap();
        written.lock().unwrap().push((Vec::new(), Vec::new(), None));
        assert_eq!(
            core.clipboard_image_received(&accepted, &png),
            Err("conflict")
        );
    }
}
