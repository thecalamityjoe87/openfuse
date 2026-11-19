//! Find MiniFuse cards via /proc/asound and choose a backend.

use super::Backend;
use super::Status;
use super::alsa::AlsaBackend;
use super::mock::MockBackend;
use crate::models::{self, MINIFUSE2, ModelSpec};
use std::path::Path;

pub const ARTURIA_VID: &str = "1c75";

#[derive(Debug, Clone)]
pub struct FoundCard {
    pub index: u32,
    pub usbid: String,
    pub card_id: String,
    pub model: &'static ModelSpec,
}

impl FoundCard {
    pub fn hw(&self) -> String {
        format!("hw:{}", self.index)
    }
}

fn read_trim(p: &Path) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// MiniFuse cards, in card order. Matches on USB ID, then (for Arturia
/// devices whose PID is not in the table yet) on the card name.
pub fn find_cards(proc_root: &Path) -> Vec<FoundCard> {
    let Ok(rd) = std::fs::read_dir(proc_root) else { return vec![] };
    let mut dirs: Vec<(u32, std::path::PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let idx = name.strip_prefix("card")?.parse::<u32>().ok()?;
            Some((idx, e.path()))
        })
        .collect();
    dirs.sort_by_key(|d| d.0);
    let mut found = vec![];
    for (index, d) in dirs {
        let usbid = read_trim(&d.join("usbid")).to_ascii_lowercase();
        if usbid.is_empty() {
            continue;
        }
        let card_id = read_trim(&d.join("id"));
        let mut model = models::model_for_usb_id(&usbid);
        if model.is_none() && usbid.starts_with(&format!("{ARTURIA_VID}:")) {
            model = models::model_for_card_name(&card_id);
        }
        if let Some(model) = model {
            found.push(FoundCard { index, usbid, card_id, model });
        }
    }
    found
}

/// Pick the backend: AlsaBackend when the OpenFuse controls exist, otherwise
/// MockBackend with a reason suitable for the SIMULATED banner.
pub fn create_backend(
    mock: bool,
    model_key: Option<&str>,
    card: Option<u32>,
    proc_root: &Path,
    animate: bool,
) -> Box<dyn Backend> {
    let model = model_key.and_then(models::model_by_key);
    if mock {
        return Box::new(MockBackend::new(model.unwrap_or(&MINIFUSE2), "Simulated device (--mock)", animate, Status::Simulated, None));
    }
    let mut cards = find_cards(proc_root);
    if let Some(n) = card {
        cards.retain(|c| c.index == n);
    }
    let Some(c) = cards.into_iter().next() else {
        return Box::new(MockBackend::new(
            model.unwrap_or(&MINIFUSE2),
            "No MiniFuse found",
            false, // nothing is shown while there is no device: don't simulate activity
            Status::NoDevice,
            None,
        ));
    };
    let (m, idx) = (c.model, c.index);
    match AlsaBackend::new(c) {
        Err(e) => Box::new(MockBackend::new(
            m,
            &format!("{} found on card {idx}, but its controls could not be opened ({e}) — simulated", m.name),
            animate,
            Status::ControlsMissing,
            None,
        )),
        Ok(mut be) if be.keys().is_empty() => {
            be.stop();
            Box::new(MockBackend::new(
                m,
                &format!(
                    "{} found on card {idx}, but the OpenFuse controls are missing (kernel module not loaded?) — simulated",
                    m.name
                ),
                animate,
                Status::ControlsMissing,
                None,
            ))
        }
        Ok(be) => Box::new(be),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(dir: &Path, cards: &[(u32, Option<&str>, &str)]) {
        for (idx, usbid, cid) in cards {
            let d = dir.join(format!("card{idx}"));
            std::fs::create_dir(&d).unwrap();
            if let Some(u) = usbid {
                std::fs::write(d.join("usbid"), format!("{u}\n")).unwrap();
            }
            std::fs::write(d.join("id"), format!("{cid}\n")).unwrap();
        }
    }

    #[test]
    fn find_cards_by_usb_id_and_name() {
        let t = tempfile::tempdir().unwrap();
        proc(
            t.path(),
            &[
                (0, None, "Generic"),
                (3, Some("1c75:af90"), "M2"),
                (5, Some("1c75:aaaa"), "MiniFuse4"),
                (6, Some("1234:af90"), "MiniFuse1"),
                (12, Some("1C75:AF90"), "M2"),
            ],
        );
        let found = find_cards(t.path());
        let got: Vec<(u32, &str)> = found.iter().map(|c| (c.index, c.model.key)).collect();
        assert_eq!(got, vec![(3, "minifuse2"), (5, "minifuse4"), (12, "minifuse2")]);
        assert_eq!(found[0].hw(), "hw:3");
    }

    #[test]
    fn no_card_falls_back_to_simulation() {
        let t = tempfile::tempdir().unwrap();
        proc(t.path(), &[(0, None, "Generic")]);
        let be = create_backend(false, None, None, t.path(), false);
        assert!(be.simulated());
        assert_eq!(be.status(), Status::NoDevice);
        assert!(be.description().contains("No MiniFuse"));
        // --card that does not match also falls back
        let be = create_backend(false, Some("minifuse4"), Some(9), t.path(), false);
        assert!(be.simulated() && be.model().key == "minifuse4");
    }

    #[test]
    fn mock_flag_always_simulates() {
        let be = create_backend(true, Some("minifuse1"), None, Path::new("/nonexistent"), false);
        assert!(be.simulated() && be.status() == Status::Simulated && be.model().key == "minifuse1");
    }
}
