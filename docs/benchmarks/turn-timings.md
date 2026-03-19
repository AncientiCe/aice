# Turn Timing Benchmarks (desktop-runner)

`desktop-runner` emits a `turn_timing` log entry at the end of each turn so you can track full journey timing:
`mic -> STT -> LLM -> TTS`.

Important: these timings are **completion times**, not just model latency. They include end-of-speech detection, synthesis/flush, and audible speech output time.

## Recent updates (by date)

2026-03-18

| Query | mic_to_stt_ms | stt_ms | llm_ms | tts_ms | journey_ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| what's the weather? | 3756 | 1644 | 1610 | 858 | 6478 |
| what's the weather in Rome? | 4468 | 1652 | 1524 | 758 | 7318 |

2026-03-19

| Timestamp (UTC) | Query | mic_to_stt_ms | speech_voiced_ms | stt_ms | endpointing_wait_ms | llm_first_token_ms | llm_ms | llm_stream_tail_ms | time_to_first_audio_ms | tts_ms | journey_ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2026-03-19T07:17:03.268484Z | what's the weather? | 1957 | - | 144 | - | - | 1706 | - | - | 810 | 4735 |
| 2026-03-19T07:45:35.308592Z | what's the weather? | 2133 | 865 | 170 | 1098 | 341 | 1891 | 1550 | 2474 | 957 | 5208 |
| 2026-03-19T07:45:54.466796Z | what's the weather in Rome? | 2630 | 1261 | 144 | 1225 | 322 | 1555 | 1233 | 2952 | 783 | 5429 |
| 2026-03-19T07:49:54.681457Z | what's the weather? | 2639 | 716 | 155 | 1768 | 344 | 2023 | 1679 | 2983 | 876 | 5896 |
| 2026-03-19T07:50:14.847728Z | what's the weather in Bucharest? | 2573 | 993 | 150 | 1430 | 326 | 1516 | 1190 | 2899 | 761 | 5595 |
| 2026-03-19T07:52:53.581790Z | what's the weather in Los Angeles? | 3592 | 1464 | 147 | 1981 | 342 | 1642 | 1300 | 3934 | 824 | 5925 |
| 2026-03-19T07:53:23.395709Z | what's the weather? | 2027 | 972 | 150 | 905 | 337 | 1851 | 1514 | 2364 | 855 | 4970 |
| 2026-03-19T07:57:50.341257Z | what's the weather? | 2030 | 844 | 152 | 1034 | 339 | 1851 | 1512 | 2369 | 970 | 5092 |
| 2026-03-19T07:58:42.873159Z | what's the weather in Berlin? | 3025 | 1742 | 145 | 1138 | 339 | 1861 | 1522 | 3364 | 857 | 6115 |
| 2026-03-19T08:06:58.240448Z | how far is Rome? | 2796 | 1346 | 257 | 1193 | 428 | 1761 | 1333 | 3224 | 861 | 6057 |
| 2026-03-19T08:07:23.955921Z | what's the weather? | 1930 | 822 | 202 | 906 | 469 | 1837 | 1368 | 2399 | 825 | 5181 |
| 2026-03-19T08:07:54.060732Z | what's the weather in LA? | 2891 | 1442 | 192 | 1257 | 380 | 1451 | 1071 | 3271 | 727 | 6147 |

Latest run shows STT now in the sub-200 ms range; LLM is the dominant stage.

## Benchmark protocol (repeatable)

Run 10 turns per utterance and report median + p90 for:

- `speech_voiced_ms`
- `stt_ms`
- `llm_first_token_ms`
- `llm_ms`
- `time_to_first_audio_ms`
- `journey_ms`

Fixed utterance set:

1. `what's the weather?`
2. `what's the weather in Rome?`
3. `tell me a joke`

When publishing updates, append a new date section with before/after tables.
Do not append new raw logs unless debugging requires it.

## Raw logs (historical sample)

<details>
<summary>Expand raw benchmark log examples</summary>

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
{"timestamp":"2026-03-19T07:45:32.233547Z","level":"INFO","fields":{"message":"turn","user_text":"what's the weather?"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:45:33.417238Z","level":"INFO","fields":{"message":"skill_executed","skill":"weather"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:45:35.308592Z","level":"INFO","fields":{"message":"turn_timing","path":"skill_weather","outcome":"Complete","mic_to_stt_ms":"2133","speech_voiced_ms":"865","stt_ms":"170","endpointing_wait_ms":"1098","llm_first_token_ms":"341","llm_ms":"1891","llm_network_and_prefill_ms":"341","llm_stream_tail_ms":"1550","tts_first_audio_ms":"0","time_to_first_audio_ms":"2474","tts_ms":"957","tts_flush_ms":"957","journey_ms":"5208"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:45:51.667079Z","level":"INFO","fields":{"message":"turn","user_text":"what's the weather in Rome?"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:45:52.910468Z","level":"INFO","fields":{"message":"skill_executed","skill":"weather"},"target":"desktop_runner::runtime"}
{"timestamp":"2026-03-19T07:45:54.466796Z","level":"INFO","fields":{"message":"turn_timing","path":"skill_weather","outcome":"Complete","mic_to_stt_ms":"2630","speech_voiced_ms":"1261","stt_ms":"144","endpointing_wait_ms":"1225","llm_first_token_ms":"322","llm_ms":"1555","llm_network_and_prefill_ms":"322","llm_stream_tail_ms":"1233","tts_first_audio_ms":"0","time_to_first_audio_ms":"2952","tts_ms":"783","tts_flush_ms":"783","journey_ms":"5429"},"target":"desktop_runner::runtime"}
```

</details>
