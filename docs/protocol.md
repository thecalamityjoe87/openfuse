# MiniFuse control protocol

This document describes how a host controls the non-audio features of the Arturia
MiniFuse family of USB audio interfaces: input switches, phantom power, direct
monitoring, the loopback mixer and level meters.

It is the **only** input for implementing OpenFuse. It describes the device's
external interface (what goes over the USB wire and what it means). It does not
contain, and must not be extended with, code from any vendor software. If something
is unknown here, find it out by experimenting with the hardware and record the result
in this document; do not guess.

Audio streaming, MIDI and sample-rate selection use the standard USB Audio Class 2.0
and MIDI class mechanisms and already work with the stock Linux `snd-usb-audio`
driver. They are out of scope except where noted.

**Status markers** used throughout:

- ✅ **verified**: observed on real hardware.
- ⚠️ **unverified**: strongly indicated but not yet observed; confirm before relying on it.
- ❓ **unknown**: needs investigation.

---

## 1. Devices

| Model | USB VID:PID | Status |
|---|---|---|
| MiniFuse 2 | `1c75:af90` | ✅ verified (firmware bcdDevice 1.50) |
| MiniFuse 1 | ❓ | not yet characterised |
| MiniFuse 4 | ❓ | not yet characterised |
| MiniFuse 2 OTG | ❓ | ⚠️ believed to share the MiniFuse 2 parameter IDs |

`iProduct` is the string `MiniFuse 2` (etc.).

### 1.1 MiniFuse 2 USB topology ✅

| Interface | Class | Purpose | Linux driver |
|---|---|---|---|
| 0 | Audio / Control (UAC2) | Audio control; **also carries the MiniFuse control protocol** | `snd-usb-audio` |
| 1 | Audio / Streaming | Playback, 4 channels, S32_LE | `snd-usb-audio` |
| 2 | Audio / Streaming | Capture | `snd-usb-audio` |
| 3 | Audio / Control | Audio control for the MIDI function (header only) | `snd-usb-audio` |
| 4 | MIDI Streaming | DIN MIDI in/out | `snd-usb-audio` |
| 5 | Application-specific / DFU 1.1 | Firmware update | none |

UAC2 entities on interface 0: Clock Source **41**, Clock Selector **40**, Input
Terminals 1 and 2, Output Terminals 20 and 22, Extension Unit 30 (no controls
advertised).

---

## 2. Transport ✅

All control traffic is **USB Audio class-specific control requests on the default
control endpoint, addressed to interface 0, entity 0**:

- `wIndex = 0x0000` (high byte = entity 0, low byte = interface 0).
- `bmRequestType = 0x21` for host-to-device (SET), `0xA1` for device-to-host (GET).
- `bRequest` values are vendor-defined codes in the class request space (see §3).

The firmware **STALLs** the same requests sent with any other `bmRequestType`
(`0xA0`, `0xC0`, `0xC1` were tried). ✅

### 2.1 Consequences on Linux ✅

Because the requests target interface 0, which `snd-usb-audio` owns, the Linux usbfs
layer refuses them from userspace unless userspace claims interface 0. Claiming it
means detaching `snd-usb-audio`, which removes the sound card. So:

- A userspace program **cannot** use this protocol without interrupting audio.
- The intended implementation sends these requests **from inside `snd-usb-audio`**,
  as a device-specific mixer quirk, and exposes the result as ALSA controls (§7).

### 2.2 Do not touch interface 5 ✅

Sending these requests (or anything other than standard DFU requests) to the DFU
interface caused the device's control endpoint to stop answering class requests on
interface 0 until it was unplugged and replugged. Never address interface 5 from the
control code.

---

## 3. Requests

| Name | bmRequestType | bRequest | wValue | wIndex | wLength | Data | Status |
|---|---|---|---|---|---|---|---|
| GET_STATE | `0xA1` | `0xB2` | `0x0000` | `0x0000` | state size | state block (§4) | ✅ |
| GET_METERS | `0xA1` | `0xC2` | `0x0000` | `0x0000` | meter size | meter block (§5) | ✅ transfer; ⚠️ format |
| GET_PARAM | `0xA1` | `0xA2` | `(paramId << 8) \| index` | `0x0000` | 2 | u16 LE | ✅ |
| SET_PARAM | `0x21` | `0x22` | `(paramId << 8) \| index` | `0x0000` | 2 (see note) | u16 LE | ✅ |

Notes:

- **SET_PARAM length:** 2 bytes for every parameter, except **paramId `0x0D`, which
  takes 1 byte**. ⚠️ (`0x0D` has no known meaning on MiniFuse 2.)
- **Sizes for MiniFuse 2:** state = **48 bytes** ✅, meters = **16 bytes** ✅. Request
  exactly these sizes. A larger `wLength` is accepted and the device returns the
  real size, but implementations should use the exact sizes per model.
- **Timeouts:** 500 ms for GET_PARAM/SET_PARAM, 1000 ms for GET_STATE/GET_METERS.
- A successful SET_PARAM takes effect immediately; GET_PARAM and GET_STATE reflect
  the new value straight away. ✅ (Direct Mono, §4.2)

---

## 4. State block

GET_STATE returns an array of little-endian `u16` values. For MiniFuse 2 that is
**24 entries** (48 bytes). In this document `state[n]` means the n-th u16.

For the parameters verified so far, **the paramId equals the position of the
parameter's first entry in the state array**. Multi-channel parameters either occupy
consecutive entries (one per channel) or pack channels into one bitfield entry, as
listed below.

### 4.1 MiniFuse 2 state layout

| state[n] | paramId | Meaning | Encoding | Status |
|---|---|---|---|---|
| 0x00 | 0x00 | INST (Hi-Z) switches | bitfield: bit 0 = input 1, bit 1 = input 2 | ✅ |
| 0x01 | ❓ | unknown (read 0) | | ❓ |
| 0x02 | ❓ | unknown (read 0) | | ❓ |
| 0x03 | ❓ | unknown (read 0) | | ❓ |
| 0x04 | 0x04 | 48 V phantom power (both inputs) | 0 = off, 1 = on | ✅ |
| 0x05 | 0x05 | Direct monitor mono | 0 = off (stereo), 1 = on (mono) | ✅ |
| 0x06–0x0B | 0x06 | Loopback mixer, channel 0–5 gain to **left** | dB, §4.3 | ⚠️ |
| 0x0C–0x11 | 0x0C | Loopback mixer, channel 0–5 gain to **right** | dB, §4.3 | ⚠️ |
| 0x12 | ❓ | possibly loopback output level | dB, §4.3 | ❓ |
| 0x13 | 0x13 | Loopback enable | 0/1 | ⚠️ |
| 0x14 | 0x14 | Loopback mix → USB inputs 1/2 | 0/1 | ⚠️ |
| 0x15 | 0x15 | Loopback mix → speakers/phones | 0/1 | ⚠️ |
| 0x16 | 0x16 | Loopback channel mutes | bitfield, bit n = channel n (6 channels) | ⚠️ |
| 0x17 | ❓ | unknown (read 0) | | ❓ |

Values observed on an idle unit with factory settings:
`state[0x06..0x11] = 0xC0FE ×4, 0xFCFE, 0xC0FE, 0xC0FE ×5, 0xFCFE`; everything else 0.

**Mic/Line input type is not reported.** Plugging and unplugging a cable on input 1
changed nothing in the state block. ✅

### 4.2 Hardware buttons ✅

INST 1, INST 2, 48 V and DIRECT MONO are physical buttons. Pressing one changes the
corresponding state entry (observed via GET_STATE polled every 100 ms). The device
does **not** push notifications for these changes, so the host must poll GET_STATE to
stay in sync. Writing an entry with SET_PARAM changes the device state and its LED.

The analog knobs (GAIN 1/2, MONITOR, MIX, headphone level) are **not** readable. ✅
Each knob was turned end to end while GET_STATE was polled every 100 ms, and no
state entry changed. The input meters are post-gain, so GAIN changes show up as level
changes, not as a position.

**Estimating GAIN from the idle noise floor** (OpenFuse, experimental). The input meters
are post-gain, so with nothing but noise at the input the meter's floor rises with the
knob. Measured on one MiniFuse 2 with nothing plugged in, GAIN 1 held at five positions
for 6 s each (median of the meter's peaks, input 2 stayed near −95 dBFS throughout):

| GAIN 1 | 7:30 (min) | 9:00 | 12:00 | 3:00 | 4:30 (max) |
|---|---|---|---|---|---|
| idle floor, dBFS | −92.4 | −92.8 | −85.5 | −72.3 | −63.0 |

The knob is not linear in dB: the floor is flat (converter noise) over about the bottom
fifth of the travel, then rises about 22 dB per unit of rotation up to 12:00, about 40
above it and about 56 near the top. A straight line between minimum and maximum read
12:00 as 23%, so `src/knobs.rs` interpolates between the measured points instead. The
estimate is only valid while the input is quiet (a signal is not distinguishable from
gain), is held while one is present, and is a few percent off between the measured
points. MONITOR, MIX and PHONES act after the converters, on the analog monitor path,
so no value reaches the computer; OpenFuse draws them at a fixed position.

**No other path carries the knobs** ✅ (2026-11-16). The device has no interrupt
endpoint and no UAC2 Feature Unit, so it cannot push a change. With every knob
turned end to end, the 48-byte state block (about 3,000 reads over 4 min) never
changed, the device sent no MIDI, and only GAIN 1/2 moved a meter (their own
input). The vendor app's request set has no knob read either.

**Converter floor differs per input** ✅. On the same unit, with nothing plugged in
and GAIN at minimum, input 1 idles at about −92.4 dBFS and input 2 at about
−96.3. Input 2's preamp noise therefore shows earlier in the knob's travel. A
user calibration of GAIN 2 read −96.3 / −94.5 / −86.7 / −73.0 / −63.0 dBFS at
7:30 / 9:00 / 12:00 / 3:00 / 4:30. With the converter's share removed, the
preamp noise rises 12.1 dB from 9:00 to 12:00, 14.2 to 3:00 and 10.1 to 4:30
(the guitar measurement gave 14.8 and 10.8 for the last two). So 9:00 is about
17 dB above minimum, not the ~10 a straight line from 7:30 to 12:00 gives. The
model is: floor = converter noise + gain × (preamp noise + source noise).

### 4.3 Level encoding ⚠️

Loopback gains are **signed 16-bit fixed point, 1/256 dB per step (Q8.8)**:

- `0xFCFE` = −770 → −3.008 dB
- `0xC0FE` = −16130 → −63.008 dB, observed for "off" channels; it probably is, or is
  below, the minimum.

Expected ranges: channel gains −60 dB … +12 dB, loopback output level −64 dB … 0 dB.
Confirm the minimum ("off") value and the maximum by writing values and reading them
back.

### 4.4 Loopback channel order ❓

The 6 loopback mixer channels are believed to be analog inputs 1–2 and the USB
playback channels, but the order is unknown. In the factory default, channel 4 feeds
left only and channel 5 feeds right only (both at about −3 dB), which suggests 4/5 are
a stereo playback pair. Determine the order by playing signal into each source and
watching the loopback capture.

---

## 5. Meter block ⚠️

GET_METERS returns **16 bytes** on MiniFuse 2 ✅: **4 meters × u32 little-endian**.

| Meter | Source | Status |
|---|---|---|
| 0 | input 1 | ✅ responds to turning GAIN 1 |
| 1 | input 2 | ✅ responds to turning GAIN 2 |
| 2 | **not** the main output; most likely loopback left | ✅ stays 0 during playback with loopback off (OpenFuse app shows it as loopback L) |
| 3 | **not** the main output; most likely loopback right | ✅ stays 0 during playback with loopback off |

Verified with the OpenFuse driver: while music played through the MiniFuse (playback
stream running) with loopback off, meters 2/3 stayed 0. The device does **not** meter
playback. Output meters must therefore be computed on the host from the playback
stream (the official Windows app's output meters presumably work this way). OpenFuse
meters the samples in the driver (the `Playback Level Meter` control), with a
PipeWire/PulseAudio sink capture as the fallback for an older module. Whether
2/3 are the loopback meters is still to be confirmed by enabling loopback (⚠️).

**Format** ⚠️ (strongly indicated): each meter is the **peak absolute sample value**
as a 32-bit audio sample (24-bit data, low byte always `00`). Full scale ≈ 2³¹, so
`dBFS = 20 · log10(value / 2³¹)`; 0 means silence.

Observed values: idle inputs `0x7500`–`0xF300` (about −97 to −91 dBFS); peaks while
turning GAIN with no source up to `0x1A6900` (about −62 dBFS). Values change a lot
from read to read, so they are peaks, not averages.

An earlier guess that meters were Q8.8 dB is **wrong**: values larger than 16 bits
were observed (e.g. `0x177100`).

Still to confirm (§8 Q5): the exact full-scale value (feed a clipping signal) and
whether a read resets the peak.

Polling rate: around every 30–50 ms for a smooth display.

---

## 6. Other device information

- **Sample rate:** standard UAC2 clock source 41 / clock selector 40. Rates 44.1, 48,
  88.2, 96, 176.4 and 192 kHz. Handled by `snd-usb-audio`; do not reimplement.
- **Firmware version:** ⚠️ `bcdDevice` (1.50 on the test unit) is believed to encode
  the firmware version. Read it from the device descriptor; do not use the DFU
  interface for it.
- **Firmware update:** out of scope.

---

## 7. Implementation guidance (non-normative)

### 7.1 Kernel: `snd-usb-audio` mixer quirk

Model it on the existing device-specific mixer quirks in `sound/usb/` (for example
the Focusrite Scarlett support):

- Match on VID:PID; add the controls when the mixer is created.
- Send the requests in §3 with the driver's own USB control-message helpers. The
  driver owns interface 0, so no claiming is needed.
- Suggested ALSA controls for MiniFuse 2:

| ALSA control | Type | Backing |
|---|---|---|
| `Line In 1 Inst Switch` / `Line In 2 Inst Switch` | boolean | state[0x00] bit 0 / bit 1 |
| `Phantom Power Switch` | boolean | state[0x04] |
| `Direct Monitor Mono Switch` | boolean | state[0x05] |
| `Loopback Switch` | boolean | state[0x13] |
| `Loopback To USB In Switch` | boolean | state[0x14] |
| `Loopback To Monitor Switch` | boolean | state[0x15] |
| `Loopback Ch N Left/Right Volume` | integer, dB TLV | state[0x06+N], state[0x0C+N] |
| `Loopback Ch N Mute` | boolean | state[0x16] bit N |
| `Level Meter` | read-only integer array | GET_METERS |

- The hardware buttons change state behind the driver's back, so refresh the cache
  periodically (GET_STATE every ~250–500 ms) and send ALSA change notifications for
  entries that differ. Meters are read on demand.
- Keep the `wLength` sizes model-specific (§3).
- Secure Boot: out-of-tree builds must be MOK-signed. Ubuntu's DKMS handles this.

### 7.2 GUI

- Talk only to ALSA controls, never to USB directly.
- Pick the device render (`assets/render/<model>_{front,back}.svg`) from the card's
  USB ID. Each control in the render has a stable element `id` (`in1-inst`,
  `in2-inst`, `phantom-48v`, `direct-mono`, `meter-L-0…5`, `usb-host`, …); see
  `tools/render_devices.py`.

---

## 8. Open questions and how to resolve them

| # | Question | How to resolve |
|---|---|---|
| Q1 | SET_PARAM for bitfields (INST, mutes): is `index` a bit number with value 0/1, or must the value be the whole bitfield? | Write `(0x00<<8)\|1` with value 1 and check whether state[0x00] becomes 2 (bit) or 1 (whole value). Restore afterwards. |
| Q2 | Meaning of state[0x01–0x03], [0x12], [0x17] | Change settings and watch which entries move; try GET_PARAM on them. |
| Q3 | Level encoding limits (§4.3) | Write −60 dB, 0 dB, +12 dB, and read back. |
| Q4 | Loopback channel order (§4.4) | Signal into each source with loopback enabled; observe the loopback capture. |
| Q5 | Meter format and order (§5) | Known-level signal into each input. |
| Q6 | MiniFuse 1 / 4 / 2 OTG: PIDs, state size, parameter IDs | Same procedure on each unit. |
| Q7 | ~~Are any knob positions reported?~~ **Answered: no** (§4.2). | — |

Record every answer in this document with its status marker.
