/* SPDX-License-Identifier: GPL-2.0 */
#ifndef __USB_MIXER_MINIFUSE_H
#define __USB_MIXER_MINIFUSE_H

#include <sound/pcm.h>

struct snd_usb_audio;
struct usb_mixer_interface;

int snd_minifuse_init(struct usb_mixer_interface *mixer);

/*
 * Called with the PCM data about to be sent to the device (one URB), so
 * the peak level of what is played back can be metered whichever
 * application plays it.
 */
void snd_minifuse_playback_tap(struct snd_usb_audio *chip, const void *buf,
			       unsigned int bytes, unsigned int channels,
			       snd_pcm_format_t format);

#endif /* __USB_MIXER_MINIFUSE_H */
