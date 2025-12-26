
# LLM anonymous Trace-Replayer

**Trace-Replayer** is a Rust-based tool for replaying **anonymous traces** (e.g., https://github.com/alibaba-edu/qwen-bailian-usagetraces-anon) containing block hashes, making it easier for developers to conduct **debugging and performance benchmarking** of LLM serving systems.

At a high-level, it reconstructs **prompts** based on **prompt length + block hashes** recorded in the trace (preserving the same KVCache hit patterns),
sends requests to specified API endpoints, and records responses for further analysis
and evaluation.

**Trace-Replayer** can achieve **100+ QPS and 500,000+ tokens/s**
while using only **~30 CPU threads**, which is sufficient for stress testing a
**16/32-instance Qwen3-30B-A3B deployment**.

> **Note**:  
> The generated prompts are semantically meaningless. Since the original prompt text
> or token list is not available (for privacy protection), there is insufficient
> information to reconstruct the original prompt content.
> Therefore, the constructed prompts are **only suitable for performance testing**,
> not for semantic correctness or model capability evaluation.

> **Strongly recommended**:  
> Use the same `tokenizers` library version as the inference framework to ensure
> identical token counts after encoding.  
> You may align versions by updating the `transformers` dependency in `Cargo.toml`
> to match the inference framework.

---

## Features

- Replay anonymous traces and construct prompts based on block hashes
- End-to-end request replay using OpenAI and TGI compatible APIs
- Exports results in **JSONL** format for post-processing
- High throughput with low resource usage:
  - ~30 CPU threads can saturate a 16-instance model cluster
  - Request timestamp drift < 5 ms
- Highly extensible:
  - Easy to add new APIs, trace formats, and metrics

---

## Supported APIs

- **OpenAI API**: `http://endpoint:port/chat/completion` (non-streaming)
- **TGI (Text Generation Inference)**: `http://endpoint:port/generate` (non-streaming)
- **AIBrix**

---

## Usage

### 1. Install Rust

Ensure Rust is installed (recommended via `rustup`):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
````

Verify installation:

```bash
rustc --version
cargo --version
```

---

### 2. Build

Build in release mode from the repository root:

```bash
cargo build \
  -p request-sim \
  --bin client \
  --release \
  -j64
```

The executable will be generated at:

```
path/to/your/repo/target/release/client
```

---

### 3. Run

```bash
# Example
path/to/your/repo/target/release/client \
  --replay-mode \
  --tokenizer /path/to/Qwen2.5-7B-Instruct/tokenizer.json \
  --tokenizer-config /path/to/Qwen2.5-7B-Instruct/tokenizer_config.json \
  --endpoint http://localhost:58009/generate \
  --api tgi \
  --dataset bailian \
  --dataset-path /path/to/qwen_traceA_blksz_16.jsonl \
  --scale-factor 1.5 \
  --time-in-secs 1200 \
  --num-producer 32 \
  --channel-capacity 40960 \
  --output-path /path/to/client.jsonl
```

---

## Command-line Arguments

### Required Arguments

| Argument             | Type             | Description                                                                                              |
| -------------------- | ---------------- | -------------------------------------------------------------------------------------------------------- |
| `--tokenizer`        | `String`         | Path to `tokenizer.json` used for tokenization.                                                          |
| `--tokenizer-config` | `String`         | Path to `tokenizer_config.json`.                                                                         |
| `--endpoint`         | `String`         | Target HTTP endpoint. See **Supported APIs** for examples (e.g., TGI: `http://localhost:8000/generate`). |
| `--api`, `-a`        | `String`         | LLM API type: `tgi`, `openai`, or `aibrix`.                                                              |
| `--dataset`, `-d`    | `String`         | Dataset type: `bailian`, `mooncake`, `azure`.                                                            |
| `--dataset-path`     | `Option<String>` | Path to the dataset file.                                                                                |

---

### Request Rate Control

| Argument         | Type          | Default | Description                                                                                                                                                                                  |
| ---------------- | ------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--scale-factor` | `Option<f64>` | None    | Request rate scaling factor. For example, `2.0` maps **logical time** in the trace to **physical (wall-clock) time** at 2× speed, issuing more requests within the same wall-clock duration. |

---

### Concurrency & Runtime

| Argument             | Type            | Default   | Description                                                              |
| -------------------- | --------------- | --------- | ------------------------------------------------------------------------ |
| `--num-producer`     | `Option<usize>` | None      | Number of producer threads in `TokenSampler` (recommended: 16).          |
| `--channel-capacity` | `Option<usize>` | None      | Channel capacity between producers and consumers (recommended: 10240).   |
| `--threads`          | `Option<usize>` | CPU cores | Number of Tokio runtime worker threads. ~30 threads can achieve 100 QPS. |

---

### Output & Logging

| Argument              | Type     | Default              | Description       |
| --------------------- | -------- | -------------------- | ----------------- |
| `--output-path`, `-o` | `String` | `./log/output.jsonl` | Output file path. |

---

### Runtime Duration

| Argument               | Type  | Default | Description                           |
| ---------------------- | ----- | ------- | ------------------------------------- |
| `--time-in-secs`, `-t` | `u64` | `60`    | Replayer runtime duration in seconds. |

---

### Platform-specific Arguments

| Argument         | Type             | Description                                        |
| ---------------- | ---------------- | -------------------------------------------------- |
| `--model-name`   | `Option<String>` | Model name used by the target inference framework. |
| `--aibrix-route` | `Option<String>` | AIBrix routing strategy name.                      |

---

### SLO Parameters

| Argument     | Type  | Default | Description          |
| ------------ | ----- | ------- | -------------------- |
| `--ttft-slo` | `f32` | `5.0`   | TTFT SLO in seconds. |
| `--tpot-slo` | `f32` | `0.06`  | TPOT SLO in seconds. |

If a request does not complete within:

```
max(15, TTFT_SLO + TPOT_SLO * output_length)
```

the connection will be aborted and a timeout will be recorded.

---

## Viewing Results

After execution:

* All results are written to the specified output file
* Each request produces one line in the **`.jsonl`** file

Example output (OpenAI API):

```text
{
    "e_time": "2157",
    "input_length": "358",
    "output_length": "54",
    "s_time": "129",
    "s_time_drift": "1",
    "span_time": "2028",
    "status": "200"
}
{...}
{...}
...
```

---

## Output Fields

Each line in the `.jsonl` file corresponds to one request and may include:

* `status`: HTTP status code
* `s_time_drift`: Deviation between actual and scheduled send time (ms, expected < 5 ms)

  * If too large, consider increasing `num_threads`
* `s_time`: Request send timestamp
* `e_time`: Request end timestamp
* `input_length`, `output_length`: Input and output token lengths
* Additional metrics:

  * OpenAI API: `span_time` (end-to-end latency in ms)
  * TGI API: TTFT, TPOT, etc.

(Field availability depends on the API implementation.)

---

## Extending API Support

1. Create a new file under:

```text
src/apis/xxx_api.rs
```

2. Define a struct for the new API.
3. Implement the `LLMApi` trait:

   * **`request_json_body`**: Construct the HTTP request body
   * **`parse_response`**: Parse the HTTP response and extract metrics
     (refer to `TGIApi` for examples)
4. In `client.rs`, instantiate
   `spawn_request_loop_with_timestamp<T>` using the new API struct as the
   generic parameter.

---

