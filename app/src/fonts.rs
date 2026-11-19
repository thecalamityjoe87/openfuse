//! Fonts. The app embeds Inter (SIL OFL, assets/fonts/) for the UI and for
//! the text in the device renders, so it looks the same on every desktop and
//! needs no system fonts.

use crate::ui::theme;
use resvg::usvg::fontdb;
use std::sync::{Arc, OnceLock};

pub const UI_FAMILY: &str = "Inter";

/// Font database for resvg: Inter only (fast to build, deterministic).
pub fn db() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        for f in theme::FONTS {
            db.load_font_data(f.to_vec());
        }
        db.set_sans_serif_family(UI_FAMILY);
        Arc::new(db)
    })
    .clone()
}
