// PATCHED for Last Route: the stream opens at the device's own mix rate (like Godot's WASAPI driver, so Windows does no
// resampling and the mixer resamples the 44.1 kHz cues the way Godot does) with a 10 ms buffer (Godot's measured
// output latency; upstream asked for 4096 frames, ~93 ms).
// https://github.com/floooh/sokol/blob/master/sokol_audio.h
// https://github.com/norse-rs/audir/blob/master/audir/src/wasapi/mod.rs

use crate::PlaySoundParams;

pub use crate::mixer::Playback;

use winapi::shared::guiddef::{CLSID, IID};
use winapi::shared::ksmedia;
use winapi::shared::minwindef::*;
use winapi::shared::mmreg::*;
use winapi::um::audioclient::*;
use winapi::um::audiosessiontypes::*;
use winapi::um::combaseapi::*;
use winapi::um::mmdeviceapi::*;
use winapi::um::objbase::*;
use winapi::um::synchapi::*;
use winapi::um::winbase::*;

use std::sync::mpsc;

// thanks sokol_audio!
// https://github.com/floooh/sokol/blob/master/sokol_audio.h#L559
static IID_IAudioClient: IID = IID {
    Data1: 0x1cb9ad4c,
    Data2: 0xdbfa,
    Data3: 0x4c32,
    Data4: [0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2],
};
static IID_IMMDeviceEnumerator: IID = IID {
    Data1: 0xa95664d2,
    Data2: 0x9614,
    Data3: 0x4f35,
    Data4: [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6],
};
static CLSID_IMMDeviceEnumerator: CLSID = CLSID {
    Data1: 0xbcde0395,
    Data2: 0xe52f,
    Data3: 0x467c,
    Data4: [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e],
};
static IID_IAudioRenderClient: IID = IID {
    Data1: 0xf294acfc,
    Data2: 0x3146,
    Data3: 0x4483,
    Data4: [0xa7, 0xbf, 0xad, 0xdc, 0xa7, 0xc2, 0x60, 0xe2],
};
static IID_Devinterface_Audio_Render: IID = IID {
    Data1: 0xe6327cad,
    Data2: 0xdcec,
    Data3: 0x4949,
    Data4: [0xae, 0x8a, 0x99, 0x1e, 0x97, 0x6a, 0x79, 0xd2],
};
static IID_IActivateAudioInterface_Completion_Handler: IID = IID {
    Data1: 0x94ea2b94,
    Data2: 0xe9cc,
    Data3: 0x49e0,
    Data4: [0xc0, 0xff, 0xee, 0x64, 0xca, 0x8f, 0x5b, 0x90],
};

mod consts {
    pub const CHANNELS: u32 = 2;
    pub const SAMPLE_RATE: u32 = 44100;
    /// Requested buffer: 10 ms at the device rate (WASAPI raises it to the engine period if that is longer).
    pub const BUFFER_MS: f64 = 10.0;
}

/// Wait before the next device (re)open attempt: 250 ms doubling to a 5 s cap.
fn retry_delay_ms(attempt: u32) -> u64 {
    (250u64 << attempt.min(5)).min(5000)
}

/// How long one wait for the buffer event may take before the device is probed anyway (an invalidated endpoint stops
/// signalling, so an infinite wait would hang forever).
const WAIT_MS: u32 = 200;

/// One device session: opens the default endpoint and mixes until the device is lost. Returns why it ended; the caller
/// reopens it. The mixer outlives sessions, so sounds keep their state across a device change.
unsafe fn run_session(
    buffer_end_event: winapi::shared::ntdef::HANDLE,
    builder: &mut Option<crate::mixer::MixerBuilder>,
    mixer: &mut Option<crate::mixer::Mixer>,
    rate: &mut u32,
) -> String {
    let mut device_enumerator: *mut IMMDeviceEnumerator = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &CLSID_IMMDeviceEnumerator,
        std::ptr::null_mut(),
        CLSCTX_ALL,
        &IID_IMMDeviceEnumerator,
        &mut device_enumerator as *mut _ as _,
    );
    if hr < 0 || device_enumerator.is_null() {
        return format!("CoCreateInstance failed 0x{:x}", hr as u32);
    }

    let mut device: *mut IMMDevice = std::ptr::null_mut();
    let hr = (*device_enumerator).GetDefaultAudioEndpoint(eRender, eConsole, &mut device);
    (*device_enumerator).Release();
    if hr < 0 || device.is_null() {
        return format!("GetDefaultAudioEndpoint failed 0x{:x}", hr as u32);
    }

    let mut state = 0;
    (*device).GetState(&mut state);
    if state & DEVICE_STATE_ACTIVE == 0 {
        (*device).Release();
        return "default device not active".into();
    }

    let mut audio_client: *mut IAudioClient = std::ptr::null_mut();
    let hr = (*device).Activate(
        &IID_IAudioClient,
        CLSCTX_ALL,
        std::ptr::null_mut(),
        &mut audio_client as *mut _ as _,
    );
    (*device).Release();
    if hr < 0 || audio_client.is_null() {
        return format!("Device Activate failed 0x{:x}", hr as u32);
    }

    // the shared-mode engine's own rate (Godot's driver mixes at it too: 48 kHz on most PCs). The mixer is built once,
    // so a later device with another rate is converted by WASAPI (AUTOCONVERTPCM) to the rate the mixer runs at.
    if let Some(b) = builder.take() {
        let mut r = consts::SAMPLE_RATE;
        let mut mix_format: *mut WAVEFORMATEX = std::ptr::null_mut();
        if (*audio_client).GetMixFormat(&mut mix_format) >= 0 && !mix_format.is_null() {
            let m = (*mix_format).nSamplesPerSec;
            if (8000..=384000).contains(&m) {
                r = m;
            }
            CoTaskMemFree(mix_format as _);
        }
        *rate = r;
        *mixer = Some(b.build_at(r));
    }
    let rate = *rate;
    let Some(mixer) = mixer.as_mut() else {
        (*audio_client).Release();
        return "mixer missing".into();
    };

    let format = WAVEFORMATEX {
        nChannels: consts::CHANNELS as _,
        nSamplesPerSec: rate as _,
        wFormatTag: WAVE_FORMAT_EXTENSIBLE,
        wBitsPerSample: 32,
        nBlockAlign: consts::CHANNELS as u16 * 4,
        nAvgBytesPerSec: consts::CHANNELS as u32 * rate * 4,
        cbSize: (std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>())
            as _,
    };

    const FRONT_LEFT: u32 = 0b0001;
    const FRONT_RIGHT: u32 = 0b0010;

    let format_extensible = WAVEFORMATEXTENSIBLE {
        Format: format,
        Samples: 4 * 8,
        dwChannelMask: FRONT_LEFT | FRONT_RIGHT,
        SubFormat: ksmedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
    };

    // https://docs.microsoft.com/en-us/windows/win32/coreaudio/audclnt-streamflags-xxx-constants
    const AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM: u32 = 0x80000000;
    const AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY: u32 = 0x08000000;

    let dur = consts::BUFFER_MS * 10000.0; // 100 ns units
    let hr = (*audio_client).Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        dur as _,
        0,
        &format_extensible as *const _ as _,
        std::ptr::null(),
    );
    if hr < 0 {
        (*audio_client).Release();
        return format!("audio_client.Initialize failed 0x{:x}", hr as u32);
    }

    let mut dst_buffer_frames = 0;
    let hr = (*audio_client).GetBufferSize(&mut dst_buffer_frames);
    if hr < 0 {
        (*audio_client).Release();
        return format!("GetBufferSize failed 0x{:x}", hr as u32);
    }

    let mut render_client: *mut IAudioRenderClient = std::ptr::null_mut();
    let hr = (*audio_client).GetService(&IID_IAudioRenderClient, &mut render_client as *mut _ as _);
    if hr < 0 || render_client.is_null() {
        (*audio_client).Release();
        return format!("GetService failed 0x{:x}", hr as u32);
    }

    let hr = (*audio_client).SetEventHandle(buffer_end_event);
    if hr < 0 {
        (*render_client).Release();
        (*audio_client).Release();
        return format!("SetEventHandle failed 0x{:x}", hr as u32);
    }

    (*audio_client).Start();
    let why = loop {
        WaitForSingleObject(buffer_end_event, WAIT_MS);

        let mut padding = 0;
        let hr = (*audio_client).GetCurrentPadding(&mut padding);
        if hr < 0 {
            break format!("GetCurrentPadding failed 0x{:x}", hr as u32);
        }
        let num_frames = dst_buffer_frames.saturating_sub(padding);

        if num_frames == 0 {
            continue;
        }

        let mut wasapi_buffer: *mut u8 = std::ptr::null_mut();
        let hr = (*render_client).GetBuffer(num_frames, &mut wasapi_buffer);
        if hr < 0 || wasapi_buffer.is_null() {
            break format!("GetBuffer failed 0x{:x}", hr as u32);
        }

        let buffer = std::slice::from_raw_parts_mut(
            wasapi_buffer as *mut f32,
            num_frames as usize * consts::CHANNELS as usize,
        );

        mixer.fill_audio_buffer(buffer, num_frames as _);

        (*render_client).ReleaseBuffer(num_frames, 0);
    };
    (*audio_client).Stop();
    (*render_client).Release();
    (*audio_client).Release();
    why
}

unsafe fn audio_thread(mixer_builder: crate::mixer::MixerBuilder) {
    CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);

    let buffer_end_event = CreateEventA(std::ptr::null_mut(), FALSE, FALSE, std::ptr::null());
    if buffer_end_event.is_null() {
        eprintln!("audio: CreateEvent failed, sound disabled");
        return;
    }

    let mut builder = Some(mixer_builder);
    let mut mixer = None;
    let mut rate = consts::SAMPLE_RATE;
    let mut attempt: u32 = 0;
    loop {
        let started = std::time::Instant::now();
        let why = run_session(buffer_end_event, &mut builder, &mut mixer, &mut rate);
        // a session that ran a while was healthy: restart the backoff
        if started.elapsed() > std::time::Duration::from_secs(10) {
            attempt = 0;
        }
        eprintln!("audio: device session ended ({why}), retrying");
        std::thread::sleep(std::time::Duration::from_millis(retry_delay_ms(attempt)));
        attempt = attempt.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::retry_delay_ms;

    #[test]
    fn retry_backs_off_and_caps() {
        assert!(retry_delay_ms(1) > retry_delay_ms(0));
        assert!(retry_delay_ms(100) <= 5000);
        assert_eq!(retry_delay_ms(100), retry_delay_ms(50));
    }
}

pub struct AudioContext {
    pub(crate) mixer_ctrl: crate::mixer::MixerControl,
}

impl AudioContext {
    pub fn new() -> AudioContext {
        use crate::mixer::Mixer;

        let (mixer_builder, mixer_ctrl) = Mixer::new();
        std::thread::spawn(move || unsafe {
            audio_thread(mixer_builder);
        });

        AudioContext { mixer_ctrl }
    }
}

pub struct Sound {
    sound_id: u32,
}

impl Sound {
    pub fn load(ctx: &AudioContext, data: &[u8]) -> Sound {
        let sound_id = ctx.mixer_ctrl.load(data);

        Sound { sound_id }
    }

    pub fn play(&self, ctx: &AudioContext, params: PlaySoundParams) -> Playback {
        ctx.mixer_ctrl.play(self.sound_id, params)
    }

    pub fn stop(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.stop_all(self.sound_id);
    }

    pub fn set_volume(&self, ctx: &AudioContext, volume: f32) {
        ctx.mixer_ctrl.set_volume_all(self.sound_id, volume);
    }

    pub fn delete(&self, ctx: &AudioContext) {
        ctx.mixer_ctrl.delete(self.sound_id);
    }
}
