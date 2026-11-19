//! Minimal safe wrapper around alsa-lib's control API (libasound.so.2).
//!
//! Only what OpenFuse needs: open a card's control interface, look up
//! elements by name, read/write boolean and integer values, dB conversion via
//! the element's TLV, change-event subscription, and element listing.
//!
//! Why not the `alsa` crate's `Ctl`: it does not expose an element's
//! min/max, which the raw-volume fallback needs; its own FFI layer
//! (`alsa-sys`) is used directly instead.

use alsa_sys as sys;
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_long, c_uint};
use std::ptr;

use crate::alsa_controls::Iface;

pub const TYPE_BOOLEAN: c_uint = sys::SND_CTL_ELEM_TYPE_BOOLEAN;
pub const TYPE_INTEGER: c_uint = sys::SND_CTL_ELEM_TYPE_INTEGER;
pub const TYPE_INTEGER64: c_uint = sys::SND_CTL_ELEM_TYPE_INTEGER64;
pub const EVENT_MASK_VALUE: c_uint = 1;
pub const EVENT_MASK_REMOVE: c_uint = 0xFFFF_FFFF;

#[derive(Debug, Clone)]
pub struct AlsaError(pub String);

impl std::fmt::Display for AlsaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Result<T> = std::result::Result<T, AlsaError>;

fn check(rc: c_int, what: &str) -> Result<c_int> {
    if rc < 0 {
        // SAFETY: snd_strerror returns a static string for any error code.
        let msg = unsafe { CStr::from_ptr(sys::snd_strerror(rc)) }.to_string_lossy().into_owned();
        Err(AlsaError(format!("{what}: {msg}")))
    } else {
        Ok(rc)
    }
}

fn iface_code(i: Iface) -> c_uint {
    match i {
        Iface::Card => sys::SND_CTL_ELEM_IFACE_CARD,
        Iface::Mixer => sys::SND_CTL_ELEM_IFACE_MIXER,
    }
}

pub fn iface_name(code: c_uint) -> &'static str {
    match code {
        sys::SND_CTL_ELEM_IFACE_CARD => "CARD",
        sys::SND_CTL_ELEM_IFACE_HWDEP => "HWDEP",
        sys::SND_CTL_ELEM_IFACE_MIXER => "MIXER",
        sys::SND_CTL_ELEM_IFACE_PCM => "PCM",
        sys::SND_CTL_ELEM_IFACE_RAWMIDI => "RAWMIDI",
        sys::SND_CTL_ELEM_IFACE_TIMER => "TIMER",
        _ => "OTHER",
    }
}

/// An open control interface (`hw:N`), non-blocking.
pub struct Ctl {
    raw: *mut sys::snd_ctl_t,
}

// The handle is used from one thread at a time (the UI thread).
unsafe impl Send for Ctl {}

impl Ctl {
    pub fn open(name: &str) -> Result<Ctl> {
        let c = CString::new(name).map_err(|_| AlsaError("bad device name".into()))?;
        let mut raw = ptr::null_mut();
        // SAFETY: valid out-pointer and NUL-terminated name.
        check(unsafe { sys::snd_ctl_open(&mut raw, c.as_ptr(), sys::SND_CTL_NONBLOCK as c_int) }, name)?;
        Ok(Ctl { raw })
    }

    /// Look up an element; fails if the card does not have it.
    pub fn element(&self, name: &str, index: u32, iface: Iface) -> Result<Elem> {
        let id = ElemId::new()?;
        let cname = CString::new(name).map_err(|_| AlsaError("bad control name".into()))?;
        let info = ElemInfo::new()?;
        // SAFETY: all pointers come from the matching *_malloc calls.
        unsafe {
            sys::snd_ctl_elem_id_set_interface(id.0, iface_code(iface));
            sys::snd_ctl_elem_id_set_name(id.0, cname.as_ptr());
            sys::snd_ctl_elem_id_set_index(id.0, index);
            sys::snd_ctl_elem_info_set_id(info.0, id.0);
            check(sys::snd_ctl_elem_info(self.raw, info.0), name)?;
            // the info call fills in the numid; keep the complete id
            sys::snd_ctl_elem_info_get_id(info.0, id.0);
            let typ = sys::snd_ctl_elem_info_get_type(info.0);
            let count = sys::snd_ctl_elem_info_get_count(info.0);
            // alsa-lib asserts on the wrong accessor for a type, so ask per type.
            let (min, max) = match typ {
                TYPE_INTEGER => (
                    sys::snd_ctl_elem_info_get_min(info.0) as i64,
                    sys::snd_ctl_elem_info_get_max(info.0) as i64,
                ),
                TYPE_INTEGER64 => (sys::snd_ctl_elem_info_get_min64(info.0), sys::snd_ctl_elem_info_get_max64(info.0)),
                TYPE_BOOLEAN => (0, 1),
                _ => (0, 0),
            };
            let mut db: c_long = 0;
            let has_db = typ == TYPE_INTEGER && sys::snd_ctl_convert_to_dB(self.raw, id.0, min as c_long, &mut db) >= 0;
            Ok(Elem { id, name: name.to_string(), typ, count, min, max, has_db })
        }
    }

    pub fn read(&self, el: &Elem) -> Result<Vec<i64>> {
        let v = ElemValue::new()?;
        // SAFETY: id and value were allocated by alsa-lib.
        unsafe {
            sys::snd_ctl_elem_value_set_id(v.0, el.id.0);
            check(sys::snd_ctl_elem_read(self.raw, v.0), &el.name)?;
            Ok((0..el.count)
                .map(|i| match el.typ {
                    TYPE_BOOLEAN => sys::snd_ctl_elem_value_get_boolean(v.0, i) as i64,
                    TYPE_INTEGER64 => sys::snd_ctl_elem_value_get_integer64(v.0, i),
                    TYPE_INTEGER => sys::snd_ctl_elem_value_get_integer(v.0, i) as i64,
                    _ => 0,
                })
                .collect())
        }
    }

    pub fn write(&self, el: &Elem, vals: &[i64]) -> Result<()> {
        let v = ElemValue::new()?;
        // SAFETY: as above; indices are < count.
        unsafe {
            sys::snd_ctl_elem_value_set_id(v.0, el.id.0);
            for (i, x) in vals.iter().enumerate().take(el.count as usize) {
                match el.typ {
                    TYPE_BOOLEAN => sys::snd_ctl_elem_value_set_boolean(v.0, i as c_uint, *x as c_long),
                    TYPE_INTEGER64 => sys::snd_ctl_elem_value_set_integer64(v.0, i as c_uint, *x),
                    TYPE_INTEGER => sys::snd_ctl_elem_value_set_integer(v.0, i as c_uint, *x as c_long),
                    _ => return Err(AlsaError(format!("{}: unsupported control type", el.name))),
                }
            }
            check(sys::snd_ctl_elem_write(self.raw, v.0), &el.name)?;
        }
        Ok(())
    }

    /// Raw value -> dB (TLV). Errors if the element has no dB information.
    pub fn to_db(&self, el: &Elem, raw: i64) -> Result<f64> {
        let mut out: c_long = 0;
        // SAFETY: valid handles.
        check(unsafe { sys::snd_ctl_convert_to_dB(self.raw, el.id.0, raw as c_long, &mut out) }, &el.name)?;
        Ok(out as f64 / 100.0)
    }

    pub fn raw_from_db(&self, el: &Elem, db: f64) -> Result<i64> {
        let mut out: c_long = 0;
        // SAFETY: valid handles; dir 0 = round to nearest.
        check(
            unsafe { sys::snd_ctl_convert_from_dB(self.raw, el.id.0, (db * 100.0).round() as c_long, &mut out, 0) },
            &el.name,
        )?;
        Ok(out as i64)
    }

    pub fn subscribe(&self) -> Result<()> {
        // SAFETY: valid handle.
        check(unsafe { sys::snd_ctl_subscribe_events(self.raw, 1) }, "subscribe")?;
        Ok(())
    }

    /// Pending element events (non-blocking): (name, index, mask).
    pub fn read_events(&self) -> Result<Vec<(String, u32, c_uint)>> {
        let mut ev = ptr::null_mut();
        // SAFETY: event is malloc'd here and freed below.
        check(unsafe { sys::snd_ctl_event_malloc(&mut ev) }, "event")?;
        let mut out = Vec::new();
        let res = loop {
            // SAFETY: valid handles.
            let rc = unsafe { sys::snd_ctl_read(self.raw, ev) };
            if rc == 0 || rc == -libc::EAGAIN {
                break Ok(());
            }
            if rc < 0 {
                break check(rc, "read events").map(|_| ());
            }
            // SAFETY: rc > 0 means `ev` holds an event.
            unsafe {
                if sys::snd_ctl_event_get_type(ev) == sys::SND_CTL_EVENT_ELEM {
                    let name = CStr::from_ptr(sys::snd_ctl_event_elem_get_name(ev)).to_string_lossy().into_owned();
                    let index = sys::snd_ctl_event_elem_get_index(ev);
                    let mask = sys::snd_ctl_event_elem_get_mask(ev);
                    out.push((name, index, mask));
                }
            }
            if out.len() > 4096 {
                break Ok(());
            }
        };
        // SAFETY: allocated above.
        unsafe { sys::snd_ctl_event_free(ev) };
        res.map(|_| out)
    }

    /// Every element on the card: (iface, name, index).
    pub fn list_elements(&self) -> Result<Vec<(&'static str, String, u32)>> {
        let mut list = ptr::null_mut();
        // SAFETY: list is malloc'd here, its space allocated for `count` ids, and freed below.
        unsafe {
            check(sys::snd_ctl_elem_list_malloc(&mut list), "list")?;
            let res = (|| {
                check(sys::snd_ctl_elem_list(self.raw, list), "list")?;
                let count = sys::snd_ctl_elem_list_get_count(list);
                check(sys::snd_ctl_elem_list_alloc_space(list, count), "list")?;
                check(sys::snd_ctl_elem_list(self.raw, list), "list")?;
                let used = sys::snd_ctl_elem_list_get_used(list);
                let mut out = Vec::new();
                for i in 0..used {
                    let name = CStr::from_ptr(sys::snd_ctl_elem_list_get_name(list, i)).to_string_lossy().into_owned();
                    out.push((
                        iface_name(sys::snd_ctl_elem_list_get_interface(list, i)),
                        name,
                        sys::snd_ctl_elem_list_get_index(list, i),
                    ));
                }
                sys::snd_ctl_elem_list_free_space(list);
                Ok(out)
            })();
            sys::snd_ctl_elem_list_free(list);
            res
        }
    }
}

impl Drop for Ctl {
    fn drop(&mut self) {
        // SAFETY: opened by snd_ctl_open.
        unsafe { sys::snd_ctl_close(self.raw) };
    }
}

pub struct ElemId(*mut sys::snd_ctl_elem_id_t);
unsafe impl Send for ElemId {}

impl ElemId {
    fn new() -> Result<ElemId> {
        let mut p = ptr::null_mut();
        // SAFETY: out-pointer.
        check(unsafe { sys::snd_ctl_elem_id_malloc(&mut p) }, "id")?;
        Ok(ElemId(p))
    }
}

impl Drop for ElemId {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_ctl_elem_id_malloc.
        unsafe { sys::snd_ctl_elem_id_free(self.0) };
    }
}

struct ElemInfo(*mut sys::snd_ctl_elem_info_t);

impl ElemInfo {
    fn new() -> Result<ElemInfo> {
        let mut p = ptr::null_mut();
        // SAFETY: out-pointer.
        check(unsafe { sys::snd_ctl_elem_info_malloc(&mut p) }, "info")?;
        Ok(ElemInfo(p))
    }
}

impl Drop for ElemInfo {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_ctl_elem_info_malloc.
        unsafe { sys::snd_ctl_elem_info_free(self.0) };
    }
}

struct ElemValue(*mut sys::snd_ctl_elem_value_t);

impl ElemValue {
    fn new() -> Result<ElemValue> {
        let mut p = ptr::null_mut();
        // SAFETY: out-pointer.
        check(unsafe { sys::snd_ctl_elem_value_malloc(&mut p) }, "value")?;
        Ok(ElemValue(p))
    }
}

impl Drop for ElemValue {
    fn drop(&mut self) {
        // SAFETY: allocated by snd_ctl_elem_value_malloc.
        unsafe { sys::snd_ctl_elem_value_free(self.0) };
    }
}

/// A control element of the card.
pub struct Elem {
    id: ElemId,
    pub name: String,
    pub typ: c_uint,
    pub count: u32,
    pub min: i64,
    pub max: i64,
    pub has_db: bool,
}

impl Elem {
    pub fn is_boolean(&self) -> bool {
        self.typ == TYPE_BOOLEAN
    }
}
