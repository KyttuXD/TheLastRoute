//! Last Route: the nineteen ALSA functions, four constants and three opaque types this crate uses, written out by
//! hand from `alsa/pcm.h`. They replace the `quad-alsa-sys` crate, a generated binding of the whole library (about
//! 2,500 declarations) of which nothing else was used. Types match the C header: `snd_pcm_uframes_t` is an unsigned
//! long, `snd_pcm_sframes_t` a long, the enums are C ints.

#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

#[repr(C)]
pub struct snd_pcm_t {
    _private: [u8; 0],
}
#[repr(C)]
pub struct snd_pcm_hw_params_t {
    _private: [u8; 0],
}
#[repr(C)]
pub struct snd_pcm_sw_params_t {
    _private: [u8; 0],
}

pub type snd_pcm_uframes_t = c_ulong;
pub type snd_pcm_sframes_t = c_long;

/// `snd_pcm_stream_t`
pub const SND_PCM_STREAM_PLAYBACK: c_uint = 0;
/// `snd_pcm_access_t`
pub const SND_PCM_ACCESS_RW_INTERLEAVED: c_uint = 3;
/// `snd_pcm_format_t`
pub const SND_PCM_FORMAT_FLOAT_LE: c_int = 14;
pub const SND_PCM_FORMAT_FLOAT_BE: c_int = 15;

#[link(name = "asound")]
extern "C" {
    pub fn snd_pcm_open(pcm: *mut *mut snd_pcm_t, name: *const c_char, stream: c_uint, mode: c_int) -> c_int;
    pub fn snd_pcm_prepare(pcm: *mut snd_pcm_t) -> c_int;
    pub fn snd_pcm_wait(pcm: *mut snd_pcm_t, timeout: c_int) -> c_int;
    pub fn snd_pcm_writei(pcm: *mut snd_pcm_t, buffer: *const c_void, size: snd_pcm_uframes_t) -> snd_pcm_sframes_t;
    pub fn snd_pcm_recover(pcm: *mut snd_pcm_t, err: c_int, silent: c_int) -> c_int;

    pub fn snd_pcm_hw_params_malloc(ptr: *mut *mut snd_pcm_hw_params_t) -> c_int;
    pub fn snd_pcm_hw_params_free(obj: *mut snd_pcm_hw_params_t);
    pub fn snd_pcm_hw_params_any(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t) -> c_int;
    pub fn snd_pcm_hw_params_set_access(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, access: c_uint) -> c_int;
    pub fn snd_pcm_hw_params_set_format(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, format: c_int) -> c_int;
    pub fn snd_pcm_hw_params_set_buffer_size(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: snd_pcm_uframes_t) -> c_int;
    pub fn snd_pcm_hw_params_set_channels(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: c_uint) -> c_int;
    pub fn snd_pcm_hw_params_set_rate_near(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t, val: *mut c_uint, dir: *mut c_int) -> c_int;
    pub fn snd_pcm_hw_params(pcm: *mut snd_pcm_t, params: *mut snd_pcm_hw_params_t) -> c_int;

    pub fn snd_pcm_sw_params_malloc(ptr: *mut *mut snd_pcm_sw_params_t) -> c_int;
    pub fn snd_pcm_sw_params_free(obj: *mut snd_pcm_sw_params_t);
    pub fn snd_pcm_sw_params_current(pcm: *mut snd_pcm_t, params: *mut snd_pcm_sw_params_t) -> c_int;
    pub fn snd_pcm_sw_params_set_start_threshold(pcm: *mut snd_pcm_t, params: *mut snd_pcm_sw_params_t, val: snd_pcm_uframes_t) -> c_int;
    pub fn snd_pcm_sw_params(pcm: *mut snd_pcm_t, params: *mut snd_pcm_sw_params_t) -> c_int;
}
