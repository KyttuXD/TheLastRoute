# Voice chat (groundwork)

Status: the codec, framing, jitter buffer and the range and mute rules exist in `src/bin/last_route/net/voice.rs`
and are tested. There is no capture, playback, UI or wiring yet, and no new dependency was added.

## Design

| Piece | Choice |
| --- | --- |
| Format | 16-bit mono PCM, 16 kHz, 20 ms frames (320 samples) |
| Codec | IMA ADPCM, 4 bits per sample: a frame is 163 bytes (3 header + 160), about 65 kbit/s |
| Frame header | predictor (i16) and step index (u8), so every frame decodes alone and a lost datagram never breaks the next |
| Packet | tag `V`, sequence (u32, counts frames), speaker peer (u32), frame count (u8), then 1 to 4 frames: at most 662 bytes |
| Size limit | one relay message (`session::MAX_MESSAGE_BYTES`, 1452); the cap of 4 frames also bounds one loss to 80 ms |
| Reliability | unreliable channel only. A lost voice packet is never resent: the jitter buffer reports the gap and the player conceals it |
| Jitter buffer | one per speaker. Waits for 3 frames (60 ms), then plays out in order. Late and duplicate frames are dropped, gaps give `Lost`, a depth cap (15) skips the oldest, an empty buffer re-primes |
| Range | full volume inside `full_radius`, a straight fall to zero at `radius`, silent on another floor (same rule as nearby chat and TTS) |
| Mute | a local per-peer set (`VoiceMutes`); muting changes only what this player hears |

Tunables (radius, full radius, jitter depths) are read from `data/voice.json` by `VoiceRules::load`; missing keys
fall back to the Standard defaults (30 and 6 world units, 3 and 15 frames).

ADPCM was chosen because it is about 40 lines, dependency-free, runs on the Airs at no measurable cost, and a frame
needs no history. Opus would be 3 to 4 times smaller but needs a crate or a port; the packet tag leaves room for a
later codec byte.

## Capture and playback: the open choice per platform

Nothing below is built. The codec works on plain `i16` slices so any of these can feed it.

* macOS: CoreAudio (an input audio unit) through the same path quad-snd uses for output, or a small `cpal`-style
  binding if a dependency is later allowed.
* Windows: WASAPI capture. quad-snd already uses WASAPI for playback, so the device stack is known to work.
* Browser: `getUserMedia` in the page (`dist/web/index.html`), an AudioWorklet resampling to 16 kHz and handing 320
  sample blocks to wasm through the existing web bridge. Playback through the same page, since quad-snd on the web is
  not a mixer for streams. The browser asks the player for microphone permission, which must be explained in the UI.
* Resampling: devices run at 44.1 or 48 kHz, so a cheap low-pass and decimate (3:1 from 48 kHz) belongs in the
  capture layer.

## Push-to-talk or voice activity

Start with push-to-talk (a held key): no false triggers from game sound, no echo gate to tune, and nothing is sent
while the key is up. Voice activity needs a noise floor, a hangover of about 200 ms and echo handling for speakers;
add it later as an option. Both feed the same encoder, so this is a capture-side decision only.

## Server and relay load at 8 players

One speaker sends 50 frames per second. At 4 frames per packet that is 12.5 packets per second of about 660 bytes
(8.4 KB/s, 66 kbit/s, plus the 14-byte relay header and IP/UDP).

The relay forwards, it does not mix. With 8 players in one room:

* Worst case, all 8 talk: 8 uploads of 66 kbit/s, and the host relays each to 7 others: 8 x 7 x 66 = about 3.7 Mbit/s
  out of the relay, and each client receives 7 x 66 = about 460 kbit/s.
* Normal case, 2 talking at once: 2 x 7 x 66 = about 0.9 Mbit/s out, 130 kbit/s per listener.
* Range culling on the sender or host (only forward to listeners inside `radius` and on the same floor) cuts this
  further and is cheap, since positions are already synced. This is the main lever.
* Packet rate is 12.5 per speaker per second, small next to the world sync, but the voice must use its own send budget
  so it can never starve gameplay messages. Voice is dropped first under congestion.
* The Node relay is store-and-echo with no game logic, so the cost is bandwidth and one copy per listener, not CPU.
  The web build runs over WebSocket (TCP), where one stalled packet delays the rest; the jitter buffer then plays
  late frames as `Lost`, so the browser build will sound worse on bad links than the native UDP one.

## Integration notes (not done)

* A new message kind (or a raw relay payload with the `V` tag) on the unreliable channel; the host forwards it to
  listeners in range.
* `VoiceRules::volume_for` gives the gain per speaker, `VoiceMutes::gain` applies the mute.
* A speaking indicator in the player list; a mute toggle per player. Player-facing text stays upper case.
