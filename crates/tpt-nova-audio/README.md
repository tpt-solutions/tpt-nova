# tpt-nova-audio

> 2D and 3D audio for TPT Nova: SFX one-shots, looping music, bus-based volume mixing, and positional (spatial) sources

2D and 3D audio: one-shot SFX, looping music, bus-based mixing, and positional sources, built on rodio. Degrades to a no-op on headless machines.

Part of [TPT Nova](../../README.md), the AI-native, ECS-based real-time engine.

## Decoding

| Format | Decoder |
|--------|---------|
| WAV | In-house (`wav.rs`). rodio's WAV path needs `hound`, which is Apache-2.0 only. |
| FLAC | `tpt-av-cadence-flac` (`flac.rs`), from the tpt-cadence suite, pinned to a commit. |
| Ogg Vorbis | `lewton` (rodio feature). |
| MP3 | `minimp3` (rodio feature). |

rodio runs with `default-features = false`, enabling only `playback`, `lewton`,
and `minimp3`. The `symphonia` decoder stack is not used.

## Testing

```sh
cargo test -p tpt-nova-audio
```

## License

Dual-licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
