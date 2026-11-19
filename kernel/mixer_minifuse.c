// SPDX-License-Identifier: GPL-2.0
/*
 * Arturia MiniFuse mixer quirk for the USB audio driver
 *
 * The MiniFuse interfaces stream audio with standard UAC2, but their
 * front-panel switches (INST, 48V, DIRECT MONO), the loopback mixer and
 * the level meters are controlled with vendor-defined class requests
 * sent to the audio control interface. This file exposes them as ALSA
 * controls.
 *
 * Protocol summary (all requests on the default control endpoint, class
 * type, recipient interface, wIndex = entity 0 / interface 0):
 *
 *   GET_STATE   0xA1/0xB2  wValue 0                  -> state block
 *   GET_METERS  0xA1/0xC2  wValue 0                  -> meter block
 *   GET_PARAM   0xA1/0xA2  wValue (param << 8) | idx -> u16 LE
 *   SET_PARAM   0x21/0x22  wValue (param << 8) | idx <- u16 LE
 *
 * The state block is an array of u16 LE entries; for the known
 * parameters the parameter ID equals the index of its first entry.
 * Multi-channel parameters either use consecutive entries (one per
 * channel, addressed by the request's index byte) or pack the channels
 * into the bits of a single entry.
 *
 * The device does not notify the host when a hardware button is
 * pressed, so the state block is polled and ALSA change notifications
 * are generated for the controls whose value changed.
 *
 * Several details of the protocol are not confirmed yet; every such
 * assumption is marked "UNVERIFIED" below and kept in one place.
 */

#include <linux/bitmap.h>
#include <linux/bits.h>
#include <linux/limits.h>
#include <linux/moduleparam.h>
#include <linux/mutex.h>
#include <linux/slab.h>
#include <linux/unaligned.h>
#include <linux/usb.h>
#include <linux/usb/audio.h>
#include <linux/workqueue.h>

#include <sound/control.h>
#include <sound/core.h>
#include <sound/tlv.h>

#include "usbaudio.h"
#include "mixer.h"
#include "helper.h"
#include "mixer_minifuse.h"

/*** Transport ***/

/*
 * All requests go to entity 0 of the audio control interface 0.
 * Interface 5 is the DFU interface; vendor requests sent there wedge
 * the device's control endpoint until it is replugged, so it must
 * never be addressed.
 */
#define MINIFUSE_CTRL_IFNUM	0
#define MINIFUSE_CTRL_ENTITY	0
#define MINIFUSE_DFU_IFNUM	5
#define MINIFUSE_WINDEX		((MINIFUSE_CTRL_ENTITY << 8) | MINIFUSE_CTRL_IFNUM)

static_assert(MINIFUSE_CTRL_IFNUM != MINIFUSE_DFU_IFNUM);

#define MINIFUSE_REQ_SET_PARAM	0x22
#define MINIFUSE_REQ_GET_PARAM	0xa2
#define MINIFUSE_REQ_GET_STATE	0xb2
#define MINIFUSE_REQ_GET_METERS	0xc2

#define MINIFUSE_REQTYPE_SET	(USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE)
#define MINIFUSE_REQTYPE_GET	(USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE)

/* SET_PARAM takes a 1-byte payload for this parameter only (UNVERIFIED) */
#define MINIFUSE_PARAM_1BYTE	0x0d

/* Upper bounds for the per-model sizes */
#define MINIFUSE_MAX_STATE_SIZE	128
#define MINIFUSE_MAX_METER_SIZE	64
#define MINIFUSE_MAX_ENTRIES	(MINIFUSE_MAX_STATE_SIZE / 2)
#define MINIFUSE_MAX_CTLS	64

/*** Level encoding ***/

/*
 * Loopback gains are signed Q8.8 dB (1/256 dB per step); the control
 * value is the raw device value.
 *
 * UNVERIFIED range: the spec expects -60..+12 dB for the channel gains,
 * but idle "off" channels read 0xC0FE (-63.008 dB), so the minimum is
 * set to that value and treated as mute. The maximum is the expected
 * +12 dB. Values read from the device outside this range are clamped.
 */
#define MINIFUSE_GAIN_MIN	(-16130)	/* 0xC0FE, -63.008 dB */
#define MINIFUSE_GAIN_MAX	(12 * 256)	/* +12 dB */

/*
 * Q8.8 dB to 1/100 dB, rounded to nearest; a plain expression so it can
 * be used in static initialisers (only ever given constants).
 */
#define MINIFUSE_Q88_TO_CDB(x) \
	(((x) * 100 + ((x) < 0 ? -128 : 128)) / 256)

static const DECLARE_TLV_DB_MINMAX_MUTE(minifuse_gain_tlv,
					MINIFUSE_Q88_TO_CDB(MINIFUSE_GAIN_MIN),
					MINIFUSE_Q88_TO_CDB(MINIFUSE_GAIN_MAX));

/*
 * Meters: each meter is a u32 LE; strongly indicated to be the peak
 * absolute sample value of a 32-bit sample (full scale about 2^31, so
 * dBFS = 20 * log10(value / 2^31)). The kernel exposes the raw value;
 * dB conversion is left to userspace.
 *
 * UNVERIFIED: exact full scale and whether a read resets the peak.
 */
#define MINIFUSE_METER_BYTES	4

/*
 * The device does not meter playback, so the driver does: the peak of
 * the samples it sends, per playback channel, in the same raw format
 * as the device's own meters (peak absolute value as a 32-bit sample).
 * Reading the control returns the peak since the previous read.
 */
#define MINIFUSE_PLAYBACK_CHANNELS	4

static s64 minifuse_meter_decode(const u8 *p)
{
	return get_unaligned_le32(p);
}

/*** Module parameters ***/

/*
 * Bitfield write strategy (INST switches, loopback mutes). See
 * minifuse_write_bit(). false = one bit per request (index = bit
 * number, value 0/1); true = read-modify-write of the whole bitfield.
 */
#define MINIFUSE_BITFIELD_RMW_DEFAULT	false

static bool minifuse_bitfield_rmw = MINIFUSE_BITFIELD_RMW_DEFAULT;
module_param(minifuse_bitfield_rmw, bool, 0644);
MODULE_PARM_DESC(minifuse_bitfield_rmw,
		 "MiniFuse: write bitfield controls as whole value (read-modify-write) instead of one bit per request");

#define MINIFUSE_POLL_MS_DEFAULT	500
#define MINIFUSE_POLL_MS_MIN		50
#define MINIFUSE_POLL_IDLE_MS		1000

static unsigned int minifuse_poll_ms = MINIFUSE_POLL_MS_DEFAULT;
module_param(minifuse_poll_ms, uint, 0644);
MODULE_PARM_DESC(minifuse_poll_ms,
		 "MiniFuse: state polling interval in ms (0 = off, min 50)");

/*** Device descriptions ***/

struct minifuse_model {
	u32 usb_id;
	const char *name;
	u16 state_size;		/* GET_STATE wLength in bytes */
	u16 meter_size;		/* GET_METERS wLength in bytes */
	u8 num_inputs;		/* bits used in the INST bitfield */
	u8 num_lb_channels;	/* loopback mixer channels */

	/* parameter IDs; equal to the index of the first state entry */
	u8 p_inst;		/* bitfield, bit n = input n + 1 */
	u8 p_phantom;
	u8 p_direct_mono;
	u8 p_lb_gain_l;		/* num_lb_channels consecutive entries */
	u8 p_lb_gain_r;		/* num_lb_channels consecutive entries */
	u8 p_lb_enable;
	u8 p_lb_to_usb;
	u8 p_lb_to_monitor;
	u8 p_lb_mutes;		/* bitfield, bit n = loopback channel n */
};

static const struct minifuse_model minifuse_models[] = {
	{
		.usb_id = USB_ID(0x1c75, 0xaf90),
		.name = "MiniFuse 2",
		.state_size = 48,
		.meter_size = 16,
		.num_inputs = 2,
		.num_lb_channels = 6,
		.p_inst = 0x00,
		.p_phantom = 0x04,
		.p_direct_mono = 0x05,
		.p_lb_gain_l = 0x06,
		.p_lb_gain_r = 0x0c,
		.p_lb_enable = 0x13,
		.p_lb_to_usb = 0x14,
		.p_lb_to_monitor = 0x15,
		.p_lb_mutes = 0x16,
	},
};

/*** Driver data ***/

enum minifuse_ctl_type {
	MINIFUSE_CTL_SWITCH,	/* whole entry, 0 or 1 */
	MINIFUSE_CTL_BIT,	/* one bit of a bitfield entry */
	MINIFUSE_CTL_GAIN,	/* Q8.8 dB entry */
	MINIFUSE_CTL_METER,	/* GET_METERS, read-only */
	MINIFUSE_CTL_STATE,	/* raw state block, read-only */
};

struct minifuse_ctl {
	struct snd_kcontrol *kctl;
	u8 type;
	u8 param;	/* parameter ID */
	u8 index;	/* channel (consecutive entries) or bit number */
};

struct minifuse_data {
	struct usb_mixer_interface *mixer;
	const struct minifuse_model *model;

	/* protects state[] and serialises the USB requests */
	struct mutex lock;
	u16 state[MINIFUSE_MAX_ENTRIES];

	struct delayed_work poll_work;
	bool polling;

	unsigned int num_ctls;
	struct minifuse_ctl ctls[MINIFUSE_MAX_CTLS];

	/* written from the URB path, read and cleared by the control */
	u32 playback_peak[MINIFUSE_PLAYBACK_CHANNELS];
};

/*** USB requests ***/

/*
 * Request addressing for multi-channel parameters (UNVERIFIED for
 * anything but channel 0): the channel goes in the low byte of wValue.
 */
static u16 minifuse_wvalue(u8 param, u8 index)
{
	return (param << 8) | index;
}

static u16 minifuse_set_param_len(u8 param)
{
	return param == MINIFUSE_PARAM_1BYTE ? 1 : 2;
}

/* GET_STATE / GET_METERS; the caller holds priv->lock */
static int minifuse_get_block(struct minifuse_data *priv, u8 request,
			      void *buf, u16 size)
{
	struct usb_device *dev = priv->mixer->chip->dev;
	int err;

	err = snd_usb_ctl_msg(dev, usb_rcvctrlpipe(dev, 0), request,
			      MINIFUSE_REQTYPE_GET, 0, MINIFUSE_WINDEX,
			      buf, size);
	if (err < 0)
		return err;
	return err == size ? 0 : -EPROTO;
}

static int minifuse_get_param(struct minifuse_data *priv, u8 param, u8 index,
			      u16 *val)
{
	struct usb_device *dev = priv->mixer->chip->dev;
	__le16 buf;
	int err;

	err = snd_usb_ctl_msg(dev, usb_rcvctrlpipe(dev, 0),
			      MINIFUSE_REQ_GET_PARAM, MINIFUSE_REQTYPE_GET,
			      minifuse_wvalue(param, index), MINIFUSE_WINDEX,
			      &buf, sizeof(buf));
	if (err < 0)
		return err;
	if (err != sizeof(buf))
		return -EPROTO;
	*val = le16_to_cpu(buf);
	return 0;
}

static int minifuse_set_param(struct minifuse_data *priv, u8 param, u8 index,
			      u16 val)
{
	struct usb_device *dev = priv->mixer->chip->dev;
	__le16 buf = cpu_to_le16(val);
	int err;

	/* a 1-byte payload is the low byte, i.e. the first byte of buf */
	err = snd_usb_ctl_msg(dev, usb_sndctrlpipe(dev, 0),
			      MINIFUSE_REQ_SET_PARAM, MINIFUSE_REQTYPE_SET,
			      minifuse_wvalue(param, index), MINIFUSE_WINDEX,
			      &buf, minifuse_set_param_len(param));
	return err < 0 ? err : 0;
}

/*** State cache ***/

static long minifuse_ctl_value(const struct minifuse_ctl *ctl,
			       const u16 *state)
{
	switch (ctl->type) {
	case MINIFUSE_CTL_SWITCH:
		return !!state[ctl->param];
	case MINIFUSE_CTL_BIT:
		return (state[ctl->param] >> ctl->index) & 1;
	case MINIFUSE_CTL_GAIN:
		return clamp_t(long, (s16)state[ctl->param + ctl->index],
			       MINIFUSE_GAIN_MIN, MINIFUSE_GAIN_MAX);
	default:
		return 0;
	}
}

/*
 * Re-read the state block and mark the controls whose value changed.
 * The caller holds priv->lock.
 */
static int minifuse_update_state(struct minifuse_data *priv,
				 unsigned long *changed)
{
	unsigned int n = priv->model->state_size / 2;
	u16 old[MINIFUSE_MAX_ENTRIES];
	u8 buf[MINIFUSE_MAX_STATE_SIZE];
	unsigned int i;
	int err;

	err = minifuse_get_block(priv, MINIFUSE_REQ_GET_STATE, buf,
				 priv->model->state_size);
	if (err < 0)
		return err;

	memcpy(old, priv->state, sizeof(old));
	for (i = 0; i < n; i++)
		priv->state[i] = get_unaligned_le16(buf + 2 * i);

	if (!changed)
		return 0;

	for (i = 0; i < priv->num_ctls; i++) {
		const struct minifuse_ctl *ctl = &priv->ctls[i];
		bool diff;

		if (ctl->type == MINIFUSE_CTL_METER)
			continue;
		if (ctl->type == MINIFUSE_CTL_STATE)
			diff = memcmp(old, priv->state, n * 2);
		else
			diff = minifuse_ctl_value(ctl, old) !=
			       minifuse_ctl_value(ctl, priv->state);
		if (diff)
			__set_bit(i, changed);
	}
	return 0;
}

/* Called without priv->lock held */
static void minifuse_notify(struct minifuse_data *priv,
			    const unsigned long *changed)
{
	struct snd_card *card = priv->mixer->chip->card;
	unsigned int i;

	for_each_set_bit(i, changed, priv->num_ctls)
		snd_ctl_notify(card, SNDRV_CTL_EVENT_MASK_VALUE,
			       &priv->ctls[i].kctl->id);
}

/*** Polling ***/

static void minifuse_schedule_poll(struct minifuse_data *priv)
{
	unsigned int ms = READ_ONCE(minifuse_poll_ms);

	if (!READ_ONCE(priv->polling))
		return;
	if (!ms)
		ms = MINIFUSE_POLL_IDLE_MS;	/* re-check the parameter */
	else
		ms = max(ms, MINIFUSE_POLL_MS_MIN);
	schedule_delayed_work(&priv->poll_work, msecs_to_jiffies(ms));
}

/*
 * The poll does not take a PM reference: it is cancelled from the
 * suspend callback and restarted on resume, so it only runs while the
 * device is awake, and it does not keep the device from autosuspending.
 */
static void minifuse_poll_work(struct work_struct *work)
{
	struct minifuse_data *priv =
		container_of(to_delayed_work(work), struct minifuse_data,
			     poll_work);
	struct snd_usb_audio *chip = priv->mixer->chip;
	DECLARE_BITMAP(changed, MINIFUSE_MAX_CTLS);
	int err = 0;

	if (atomic_read(&chip->shutdown))
		return;

	if (READ_ONCE(minifuse_poll_ms)) {
		bitmap_zero(changed, MINIFUSE_MAX_CTLS);
		scoped_guard(mutex, &priv->lock)
			err = minifuse_update_state(priv, changed);

		if (err == -ENODEV || err == -ESHUTDOWN)
			return;
		if (err < 0)
			usb_audio_err_ratelimited(chip, "MiniFuse: state poll failed: %d\n",
						  err);
		else
			minifuse_notify(priv, changed);
	}

	minifuse_schedule_poll(priv);
}

/*** Writes ***/

/*
 * Write one bit of a bitfield parameter (INST switches, loopback mutes).
 *
 * How the device expects bitfields to be written is not known yet
 * (spec section 8, Q1). Both candidates are implemented here, selected
 * by the minifuse_bitfield_rmw parameter:
 *
 * - per bit (default): SET_PARAM with index = bit number, value 0 or 1.
 *   This is the same (parameter, channel) addressing that the protocol
 *   uses for multi-channel parameters stored in consecutive entries, so
 *   it is the more likely interpretation. It also names one channel
 *   per request, so it cannot undo a hardware button press on another
 *   channel that happens between a read and a write.
 *
 * - read-modify-write: GET_PARAM the bitfield, change the bit, and
 *   SET_PARAM the whole value with index 0.
 *
 * A wrong choice only mis-sets INST/mute bits. The state is read back
 * after every write and a mismatch is logged, so the first write on
 * real hardware answers Q1.
 *
 * The caller holds priv->lock.
 */
static int minifuse_write_bit(struct minifuse_data *priv, u8 param, u8 bit,
			      bool on)
{
	u16 val;
	int err;

	if (!READ_ONCE(minifuse_bitfield_rmw))
		return minifuse_set_param(priv, param, bit, on);

	err = minifuse_get_param(priv, param, 0, &val);
	if (err < 0)
		return err;
	if (on)
		val |= BIT(bit);
	else
		val &= ~BIT(bit);
	return minifuse_set_param(priv, param, 0, val);
}

/* The caller holds priv->lock */
static int minifuse_write_ctl(struct minifuse_data *priv,
			      const struct minifuse_ctl *ctl, long val)
{
	switch (ctl->type) {
	case MINIFUSE_CTL_SWITCH:
		return minifuse_set_param(priv, ctl->param, 0, val);
	case MINIFUSE_CTL_BIT:
		return minifuse_write_bit(priv, ctl->param, ctl->index, val);
	case MINIFUSE_CTL_GAIN:
		return minifuse_set_param(priv, ctl->param, ctl->index,
					  (u16)(s16)val);
	default:
		return -EPERM;
	}
}

/*** Control callbacks ***/

static struct minifuse_data *minifuse_kctl_priv(struct snd_kcontrol *kctl)
{
	struct usb_mixer_elem_info *elem = kctl->private_data;

	return elem->head.mixer->private_data;
}

static struct minifuse_ctl *minifuse_kctl_to_ctl(struct snd_kcontrol *kctl,
						 struct minifuse_data **priv)
{
	struct usb_mixer_elem_info *elem = kctl->private_data;

	*priv = minifuse_kctl_priv(kctl);
	return &(*priv)->ctls[elem->control];
}

static int minifuse_gain_info(struct snd_kcontrol *kctl,
			      struct snd_ctl_elem_info *uinfo)
{
	uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
	uinfo->count = 1;
	uinfo->value.integer.min = MINIFUSE_GAIN_MIN;
	uinfo->value.integer.max = MINIFUSE_GAIN_MAX;
	uinfo->value.integer.step = 1;
	return 0;
}

static int minifuse_ctl_get(struct snd_kcontrol *kctl,
			    struct snd_ctl_elem_value *ucontrol)
{
	struct minifuse_data *priv;
	struct minifuse_ctl *ctl = minifuse_kctl_to_ctl(kctl, &priv);

	guard(mutex)(&priv->lock);
	ucontrol->value.integer.value[0] = minifuse_ctl_value(ctl, priv->state);
	return 0;
}

/*
 * Write a control value: re-read the device state first (a button may
 * have been pressed since the last poll), write only if needed, then
 * read the state back. *old and *now receive the control value before
 * and after the write.
 */
static int minifuse_put_locked(struct minifuse_data *priv,
			       const struct minifuse_ctl *ctl, long val,
			       unsigned long *changed, long *old, long *now)
{
	int err;

	CLASS(snd_usb_lock, pm)(priv->mixer->chip);
	if (pm.err < 0)
		return pm.err;

	guard(mutex)(&priv->lock);

	err = minifuse_update_state(priv, changed);
	if (err < 0)
		return err;
	*old = minifuse_ctl_value(ctl, priv->state);
	*now = *old;
	if (*old == val)
		return 0;

	err = minifuse_write_ctl(priv, ctl, val);
	if (err < 0)
		return err;
	err = minifuse_update_state(priv, changed);
	if (err < 0)
		return err;
	*now = minifuse_ctl_value(ctl, priv->state);
	return 0;
}

static int minifuse_ctl_put(struct snd_kcontrol *kctl,
			    struct snd_ctl_elem_value *ucontrol)
{
	struct minifuse_data *priv;
	struct minifuse_ctl *ctl = minifuse_kctl_to_ctl(kctl, &priv);
	struct snd_usb_audio *chip = priv->mixer->chip;
	long val = ucontrol->value.integer.value[0];
	DECLARE_BITMAP(changed, MINIFUSE_MAX_CTLS);
	long old, now;
	int err;

	if (ctl->type == MINIFUSE_CTL_GAIN) {
		if (val < MINIFUSE_GAIN_MIN || val > MINIFUSE_GAIN_MAX)
			return -EINVAL;
	} else if (val < 0 || val > 1) {
		return -EINVAL;
	}

	bitmap_zero(changed, MINIFUSE_MAX_CTLS);
	err = minifuse_put_locked(priv, ctl, val, changed, &old, &now);
	if (err < 0)
		return err;

	if (old != val && now != val)
		dev_warn_ratelimited(&chip->dev->dev,
				     "MiniFuse: \"%s\": wrote %ld, device reports %ld (param 0x%02x index %u, bitfield_rmw=%d)\n",
				     kctl->id.name, val, now, ctl->param,
				     ctl->index, READ_ONCE(minifuse_bitfield_rmw));

	minifuse_notify(priv, changed);
	return old != val;
}

static int minifuse_meter_info(struct snd_kcontrol *kctl,
			       struct snd_ctl_elem_info *uinfo)
{
	struct minifuse_data *priv = minifuse_kctl_priv(kctl);

	uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER64;
	uinfo->count = priv->model->meter_size / MINIFUSE_METER_BYTES;
	uinfo->value.integer64.min = 0;
	uinfo->value.integer64.max = U32_MAX;
	uinfo->value.integer64.step = 1;
	return 0;
}

static int minifuse_meter_get(struct snd_kcontrol *kctl,
			      struct snd_ctl_elem_value *ucontrol)
{
	struct minifuse_data *priv = minifuse_kctl_priv(kctl);
	unsigned int i, n = priv->model->meter_size / MINIFUSE_METER_BYTES;
	u8 buf[MINIFUSE_MAX_METER_SIZE];
	int err;

	CLASS(snd_usb_lock, pm)(priv->mixer->chip);
	if (pm.err < 0)
		return pm.err;

	scoped_guard(mutex, &priv->lock)
		err = minifuse_get_block(priv, MINIFUSE_REQ_GET_METERS, buf,
					 priv->model->meter_size);
	if (err < 0)
		return err;

	for (i = 0; i < n; i++)
		ucontrol->value.integer64.value[i] =
			minifuse_meter_decode(buf + i * MINIFUSE_METER_BYTES);
	return 0;
}

static int minifuse_playback_meter_info(struct snd_kcontrol *kctl,
					struct snd_ctl_elem_info *uinfo)
{
	uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER64;
	uinfo->count = MINIFUSE_PLAYBACK_CHANNELS;
	uinfo->value.integer64.min = 0;
	uinfo->value.integer64.max = U32_MAX;
	uinfo->value.integer64.step = 1;
	return 0;
}

static int minifuse_playback_meter_get(struct snd_kcontrol *kctl,
				       struct snd_ctl_elem_value *ucontrol)
{
	struct minifuse_data *priv = minifuse_kctl_priv(kctl);
	unsigned int i;

	/* no USB traffic: the peaks are collected by the playback tap */
	for (i = 0; i < MINIFUSE_PLAYBACK_CHANNELS; i++)
		ucontrol->value.integer64.value[i] =
			xchg(&priv->playback_peak[i], 0);
	return 0;
}

static int minifuse_state_info(struct snd_kcontrol *kctl,
			       struct snd_ctl_elem_info *uinfo)
{
	struct minifuse_data *priv = minifuse_kctl_priv(kctl);

	uinfo->type = SNDRV_CTL_ELEM_TYPE_BYTES;
	uinfo->count = priv->model->state_size;
	return 0;
}

static int minifuse_state_get(struct snd_kcontrol *kctl,
			      struct snd_ctl_elem_value *ucontrol)
{
	struct minifuse_data *priv = minifuse_kctl_priv(kctl);
	unsigned int i;

	guard(mutex)(&priv->lock);
	for (i = 0; i < priv->model->state_size / 2; i++)
		put_unaligned_le16(priv->state[i],
				   ucontrol->value.bytes.data + 2 * i);
	return 0;
}

static const struct snd_kcontrol_new minifuse_switch_ctl = {
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER,
	.name = "",
	.info = snd_ctl_boolean_mono_info,
	.get = minifuse_ctl_get,
	.put = minifuse_ctl_put,
};

static const struct snd_kcontrol_new minifuse_gain_ctl = {
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER,
	.name = "",
	.access = SNDRV_CTL_ELEM_ACCESS_READWRITE |
		  SNDRV_CTL_ELEM_ACCESS_TLV_READ,
	.info = minifuse_gain_info,
	.get = minifuse_ctl_get,
	.put = minifuse_ctl_put,
	.tlv = { .p = minifuse_gain_tlv },
};

static const struct snd_kcontrol_new minifuse_meter_ctl = {
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER,
	.name = "",
	.access = SNDRV_CTL_ELEM_ACCESS_READ |
		  SNDRV_CTL_ELEM_ACCESS_VOLATILE,
	.info = minifuse_meter_info,
	.get = minifuse_meter_get,
};

static const struct snd_kcontrol_new minifuse_playback_meter_ctl = {
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER,
	.name = "",
	.access = SNDRV_CTL_ELEM_ACCESS_READ |
		  SNDRV_CTL_ELEM_ACCESS_VOLATILE,
	.info = minifuse_playback_meter_info,
	.get = minifuse_playback_meter_get,
};

static const struct snd_kcontrol_new minifuse_state_ctl = {
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER,
	.name = "",
	.access = SNDRV_CTL_ELEM_ACCESS_READ,
	.info = minifuse_state_info,
	.get = minifuse_state_get,
};

/*** Control creation ***/

static int minifuse_add_ctl(struct minifuse_data *priv,
			    const struct snd_kcontrol_new *ncontrol,
			    u8 type, u8 param, u8 index, const char *name)
{
	struct usb_mixer_interface *mixer = priv->mixer;
	unsigned int entries = priv->model->state_size / 2;
	struct usb_mixer_elem_info *elem;
	struct minifuse_ctl *ctl;
	struct snd_kcontrol *kctl;
	int err;

	if (WARN_ON(priv->num_ctls >= MINIFUSE_MAX_CTLS))
		return -E2BIG;

	/* reject model table entries that point outside the state block */
	if ((type == MINIFUSE_CTL_SWITCH || type == MINIFUSE_CTL_BIT) &&
	    WARN_ON(param >= entries))
		return -EINVAL;
	if (type == MINIFUSE_CTL_GAIN && WARN_ON(param + index >= entries))
		return -EINVAL;
	if (type == MINIFUSE_CTL_BIT && WARN_ON(index >= 16))
		return -EINVAL;

	/*
	 * USB_MIXER_BESPOKEN makes the generic mixer code (resume, proc)
	 * ignore these elements; head.id 0 is not used by any unit.
	 */
	elem = kzalloc_obj(*elem);
	if (!elem)
		return -ENOMEM;
	elem->head.mixer = mixer;
	elem->head.id = 0;
	elem->control = priv->num_ctls;
	elem->channels = 1;
	elem->val_type = USB_MIXER_BESPOKEN;

	kctl = snd_ctl_new1(ncontrol, elem);
	if (!kctl) {
		kfree(elem);
		return -ENOMEM;
	}
	kctl->private_free = snd_usb_mixer_elem_free;
	strscpy(kctl->id.name, name, sizeof(kctl->id.name));

	err = snd_usb_mixer_add_control(&elem->head, kctl);
	if (err < 0)
		return err;

	ctl = &priv->ctls[priv->num_ctls++];
	ctl->kctl = kctl;
	ctl->type = type;
	ctl->param = param;
	ctl->index = index;
	return 0;
}

static int minifuse_create_controls(struct minifuse_data *priv)
{
	const struct minifuse_model *m = priv->model;
	static const struct {
		const char *name;
		size_t offset;
	} switches[] = {
		{ "Phantom Power Switch",
		  offsetof(struct minifuse_model, p_phantom) },
		{ "Direct Monitor Mono Switch",
		  offsetof(struct minifuse_model, p_direct_mono) },
		{ "Loopback Switch",
		  offsetof(struct minifuse_model, p_lb_enable) },
		{ "Loopback To USB In Switch",
		  offsetof(struct minifuse_model, p_lb_to_usb) },
		{ "Loopback To Monitor Switch",
		  offsetof(struct minifuse_model, p_lb_to_monitor) },
	};
	char name[SNDRV_CTL_ELEM_ID_NAME_MAXLEN];
	unsigned int i;
	int err;

	for (i = 0; i < m->num_inputs; i++) {
		snprintf(name, sizeof(name), "Line In %u Inst Switch", i + 1);
		err = minifuse_add_ctl(priv, &minifuse_switch_ctl,
				       MINIFUSE_CTL_BIT, m->p_inst, i, name);
		if (err < 0)
			return err;
	}

	for (i = 0; i < ARRAY_SIZE(switches); i++) {
		u8 param = *((const u8 *)m + switches[i].offset);

		err = minifuse_add_ctl(priv, &minifuse_switch_ctl,
				       MINIFUSE_CTL_SWITCH, param, 0,
				       switches[i].name);
		if (err < 0)
			return err;
	}

	/* channel N in the control name is loopback channel N - 1 */
	for (i = 0; i < m->num_lb_channels; i++) {
		snprintf(name, sizeof(name), "Loopback Ch %u Left Volume", i + 1);
		err = minifuse_add_ctl(priv, &minifuse_gain_ctl,
				       MINIFUSE_CTL_GAIN, m->p_lb_gain_l, i,
				       name);
		if (err < 0)
			return err;

		snprintf(name, sizeof(name), "Loopback Ch %u Right Volume", i + 1);
		err = minifuse_add_ctl(priv, &minifuse_gain_ctl,
				       MINIFUSE_CTL_GAIN, m->p_lb_gain_r, i,
				       name);
		if (err < 0)
			return err;

		snprintf(name, sizeof(name), "Loopback Ch %u Mute", i + 1);
		err = minifuse_add_ctl(priv, &minifuse_switch_ctl,
				       MINIFUSE_CTL_BIT, m->p_lb_mutes, i,
				       name);
		if (err < 0)
			return err;
	}

	err = minifuse_add_ctl(priv, &minifuse_meter_ctl, MINIFUSE_CTL_METER,
			       0, 0, "Level Meter");
	if (err < 0)
		return err;

	err = minifuse_add_ctl(priv, &minifuse_playback_meter_ctl,
			       MINIFUSE_CTL_METER, 0, 0,
			       "Playback Level Meter");
	if (err < 0)
		return err;

	/* raw state block, for diagnostics and the open protocol questions */
	return minifuse_add_ctl(priv, &minifuse_state_ctl, MINIFUSE_CTL_STATE,
				0, 0, "Device State Raw");
}

/*** Lifecycle ***/

static void minifuse_private_free(struct usb_mixer_interface *mixer)
{
	struct minifuse_data *priv = mixer->private_data;

	WRITE_ONCE(priv->polling, false);
	cancel_delayed_work_sync(&priv->poll_work);
	mutex_destroy(&priv->lock);
	kfree(priv);
	mixer->private_data = NULL;
}

/*
 * Playback tap. Runs for every URB, in atomic context: only reads the
 * data and updates the peaks. The mixer (and so priv) lives as long as
 * the card, which outlives any open playback stream.
 */
static struct minifuse_data *minifuse_chip_priv(struct snd_usb_audio *chip)
{
	struct usb_mixer_interface *mixer;

	list_for_each_entry(mixer, &chip->mixer_list, list)
		if (mixer->private_free == minifuse_private_free)
			return mixer->private_data;
	return NULL;
}

/* One sample as a signed 32-bit value (left-aligned), or false if unsupported */
static bool minifuse_sample_size(snd_pcm_format_t format, unsigned int *size)
{
	switch (format) {
	case SNDRV_PCM_FORMAT_S16_LE:
		*size = 2;
		return true;
	case SNDRV_PCM_FORMAT_S24_3LE:
		*size = 3;
		return true;
	case SNDRV_PCM_FORMAT_S32_LE:
		*size = 4;
		return true;
	default:
		return false;
	}
}

static s32 minifuse_sample(const u8 *p, unsigned int size)
{
	switch (size) {
	case 2:
		return (s32)((u32)get_unaligned_le16(p) << 16);
	case 3:
		return (s32)(((u32)p[0] << 8) | ((u32)p[1] << 16) |
			     ((u32)p[2] << 24));
	default:
		return (s32)get_unaligned_le32(p);
	}
}

void snd_minifuse_playback_tap(struct snd_usb_audio *chip, const void *buf,
			       unsigned int bytes, unsigned int channels,
			       snd_pcm_format_t format)
{
	u32 peak[MINIFUSE_PLAYBACK_CHANNELS] = {};
	struct minifuse_data *priv;
	unsigned int size, stride, frames, f, c, n;
	const u8 *p = buf;

	if (!channels || !minifuse_sample_size(format, &size))
		return;
	priv = minifuse_chip_priv(chip);
	if (!priv)
		return;

	stride = channels * size;
	frames = bytes / stride;
	n = min_t(unsigned int, channels, MINIFUSE_PLAYBACK_CHANNELS);

	for (f = 0; f < frames; f++, p += stride) {
		for (c = 0; c < n; c++) {
			s32 v = minifuse_sample(p + c * size, size);
			u32 a = v < 0 ? -(u32)v : (u32)v;

			if (a > peak[c])
				peak[c] = a;
		}
	}

	for (c = 0; c < n; c++) {
		u32 old = READ_ONCE(priv->playback_peak[c]);

		while (old < peak[c] &&
		       !try_cmpxchg(&priv->playback_peak[c], &old, peak[c]))
			;
	}
}

static void minifuse_private_suspend(struct usb_mixer_interface *mixer)
{
	struct minifuse_data *priv = mixer->private_data;

	WRITE_ONCE(priv->polling, false);
	cancel_delayed_work_sync(&priv->poll_work);
}

/*
 * The device keeps its own settings and may have been changed with the
 * buttons while suspended, so resume adopts the device state rather
 * than restoring the cache.
 */
static int minifuse_private_resume(struct usb_mixer_interface *mixer)
{
	struct minifuse_data *priv = mixer->private_data;
	DECLARE_BITMAP(changed, MINIFUSE_MAX_CTLS);
	int err;

	bitmap_zero(changed, MINIFUSE_MAX_CTLS);
	scoped_guard(mutex, &priv->lock)
		err = minifuse_update_state(priv, changed);
	if (err < 0)
		usb_audio_err(mixer->chip,
			      "MiniFuse: cannot read state on resume: %d\n",
			      err);
	else
		minifuse_notify(priv, changed);

	WRITE_ONCE(priv->polling, true);
	minifuse_schedule_poll(priv);
	return 0;
}

static const struct minifuse_model *minifuse_find_model(u32 usb_id)
{
	unsigned int i;

	for (i = 0; i < ARRAY_SIZE(minifuse_models); i++)
		if (minifuse_models[i].usb_id == usb_id)
			return &minifuse_models[i];
	return NULL;
}

int snd_minifuse_init(struct usb_mixer_interface *mixer)
{
	struct snd_usb_audio *chip = mixer->chip;
	const struct minifuse_model *model;
	struct minifuse_data *priv;
	int err;

	model = minifuse_find_model(chip->usb_id);
	if (!model)
		return 0;

	/*
	 * The MIDI function has its own audio control interface, which
	 * gets a mixer too; the protocol lives on interface 0 only.
	 */
	if (snd_usb_ctrl_intf(mixer->hostif) != MINIFUSE_CTRL_IFNUM)
		return 0;

	if (WARN_ON(model->state_size > MINIFUSE_MAX_STATE_SIZE ||
		    model->state_size % 2 ||
		    model->meter_size > MINIFUSE_MAX_METER_SIZE ||
		    model->meter_size % MINIFUSE_METER_BYTES))
		return -EINVAL;

	priv = kzalloc_obj(*priv);
	if (!priv)
		return -ENOMEM;

	priv->mixer = mixer;
	priv->model = model;
	mutex_init(&priv->lock);
	INIT_DELAYED_WORK(&priv->poll_work, minifuse_poll_work);

	/*
	 * If the device does not answer, leave audio working without the
	 * extra controls instead of failing the whole card.
	 */
	scoped_guard(mutex, &priv->lock)
		err = minifuse_update_state(priv, NULL);
	if (err < 0) {
		usb_audio_err(chip,
			      "Arturia %s: cannot read device state (%d), controls disabled\n",
			      model->name, err);
		mutex_destroy(&priv->lock);
		kfree(priv);
		return 0;
	}

	mixer->private_data = priv;
	mixer->private_free = minifuse_private_free;
	mixer->private_suspend = minifuse_private_suspend;
	mixer->private_resume = minifuse_private_resume;

	err = minifuse_create_controls(priv);
	if (err < 0)
		return err;

	usb_audio_info(chip, "Arturia %s: %u controls, firmware %x.%02x\n",
		       model->name, priv->num_ctls,
		       le16_to_cpu(chip->dev->descriptor.bcdDevice) >> 8,
		       le16_to_cpu(chip->dev->descriptor.bcdDevice) & 0xff);

	WRITE_ONCE(priv->polling, true);
	minifuse_schedule_poll(priv);
	return 0;
}
