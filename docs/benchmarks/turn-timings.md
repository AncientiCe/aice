# Turn Timing Benchmarks (desktop-runner)

`desktop-runner` emits a `turn_timing` log entry at the end of each turn so you can track full journey timing:
`mic -> STT -> LLM -> TTS`.

Important: these timings are **completion times**, not just model latency. They include end-of-speech detection, synthesis/flush, and audible speech output time.

## Real-world scenario logs (weather skill turns)

```json
{"timestamp":"2026-03-18T20:48:17.630419Z","level":"INFO","fields":{"message":"turn","user_text":"what's the weather?"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-18T20:48:18.741975Z","level":"INFO","fields":{"message":"skill_executed","skill":"weather"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-18T20:48:20.352816Z","level":"INFO","fields":{"message":"turn_timing","path":"skill_weather","outcome":"Complete","mic_to_stt_ms":"3756","stt_ms":"1644","llm_ms":"1610","tts_ms":"858","tts_flush_ms":"857","journey_ms":"6478"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-18T20:53:09.396430Z","level":"INFO","fields":{"message":"turn","user_text":"what's the weather in Rome?"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-18T20:53:10.721231Z","level":"INFO","fields":{"message":"skill_executed","skill":"weather"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-18T20:53:12.246062Z","level":"INFO","fields":{"message":"turn_timing","path":"skill_weather","outcome":"Complete","mic_to_stt_ms":"4468","stt_ms":"1652","llm_ms":"1524","tts_ms":"758","tts_flush_ms":"758","journey_ms":"7318"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:17:00.490171Z","level":"INFO","fields":{"message":"turn","user_text":"what's the weather?"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:17:01.562260Z","level":"INFO","fields":{"message":"skill_executed","skill":"weather"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:17:03.268484Z","level":"INFO","fields":{"message":"turn_timing","path":"skill_weather","outcome":"Complete","mic_to_stt_ms":"1957","stt_ms":"144","llm_ms":"1706","tts_ms":"810","tts_flush_ms":"810","journey_ms":"4735"},"target":"desktop_runner::runtime"}
```

## Recent updates (by date)

2026-03-18

| Query | mic_to_stt_ms | stt_ms | llm_ms | tts_ms | journey_ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| what's the weather? | 3756 | 1644 | 1610 | 858 | 6478 |
| what's the weather in Rome? | 4468 | 1652 | 1524 | 758 | 7318 |

2026-03-19

| Query | mic_to_stt_ms | stt_ms | llm_ms | tts_ms | journey_ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| what's the weather? | 1957 | 144 | 1706 | 810 | 4735 |

Latest run shows STT now in the sub-200 ms range; LLM is the dominant stage.
