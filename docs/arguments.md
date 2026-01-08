
# Command-line Arguments

This document describes all command-line arguments supported by **Trace-Replayer**.


## Required Arguments

| Argument             | Type             | Description                                                                                              |
| -------------------- | ---------------- | -------------------------------------------------------------------------------------------------------- |
| `--tokenizer`        | `String`         | Path to `tokenizer.json` used for tokenization.                                                          |
| `--tokenizer-config` | `String`         | Path to `tokenizer_config.json`.                                                                         |
| `--endpoint`         | `String`         | Target HTTP endpoint. See **Supported APIs** for examples (e.g., TGI: `http://localhost:8000/generate`). |
| `--api`, `-a`        | `String`         | LLM API type: `tgi`, `openai`, or `aibrix`.                                                              |
| `--dataset`, `-d`    | `String`         | Dataset type: `bailian`, `mooncake`, `azure`.                                                            |
| `--dataset-path`     | `Option<String>` | Path to the dataset file.                                                                                |


## Request Rate Control

| Argument         | Type          | Default | Description                                                                                                                                                                                  |
| ---------------- | ------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--scale-factor` | `Option<f64>` | None    | Request rate scaling factor. For example, `2.0` maps **logical time** in the trace to **physical (wall-clock) time** at 2× speed, issuing more requests within the same wall-clock duration. |


## Concurrency & Runtime

| Argument             | Type            | Default   | Description                                                              |
| -------------------- | --------------- | --------- | ------------------------------------------------------------------------ |
| `--num-producer`     | `Option<usize>` | None      | Number of producer threads in `TokenSampler` (recommended: 16).          |
| `--channel-capacity` | `Option<usize>` | None      | Channel capacity between producers and consumers (recommended: 10240).   |
| `--threads`          | `Option<usize>` | CPU cores | Number of Tokio runtime worker threads. ~30 threads can achieve 100 QPS. |


## Output & Logging

| Argument              | Type     | Default              | Description       |
| --------------------- | -------- | -------------------- | ----------------- |
| `--output-path`, `-o` | `String` | `./log/output.jsonl` | Output file path. |
| `--summary-path` | `Option<String>` | `<output-path>.summary.json` | Summary output file path (JSON). |
| `--metric-percentile` | `Vec<u32>` | `90,95,99` | Percentiles (comma-separated) to report for latency metrics. |


## Runtime Duration

| Argument               | Type  | Default | Description                           |
| ---------------------- | ----- | ------- | ------------------------------------- |
| `--time-in-secs`, `-t` | `u64` | `60`    | Replayer runtime duration in seconds. |


## Platform-specific Arguments

| Argument         | Type             | Description                                        |
| ---------------- | ---------------- | -------------------------------------------------- |
| `--model-name`   | `Option<String>` | Model name used by the target inference framework. |
| `--aibrix-route` | `Option<String>` | AIBrix routing strategy name.                      |


## SLO Parameters

| Argument     | Type  | Default | Description          |
| ------------ | ----- | ------- | -------------------- |
| `--ttft-slo` | `f32` | `5.0`   | TTFT SLO in seconds. |
| `--tpot-slo` | `f32` | `0.06`  | TPOT SLO in seconds. |

If a request does not complete within:

```

max(15, TTFT_SLO + TPOT_SLO * output_length)

```

the connection will be aborted and a timeout will be recorded.

## Streaming

| Argument     |  Type   | Description          |
| ------------ |  ------ | -------------------- |
| `--stream` |  `bool` | If send streaming request |
