# LLM anonymous Trace-Replayer

**Trace-Replayer** is a Rust-based tool for replaying **anonymous traces** (e.g., https://github.com/alibaba-edu/qwen-bailian-usagetraces-anon) containing block hashes on backend serving systems (e.g., vLLM, a cluster of vLLM, etc), making it easier for developers to conduct **debugging and performance benchmarking** of LLM serving systems.

At a high-level, it reconstructs **prompts** based on **prompt length + block hashes** recorded in the trace (preserving the same KVCache hit patterns) on-the-fly,
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


## Features

- Replay anonymous traces and construct prompts based on block hashes
- End-to-end request replay using OpenAI and TGI compatible APIs
- Exports results in **JSONL** format for post-processing
- High throughput with low resource usage:
  - ~30 CPU threads can saturate a 16-instance model cluster
  - Request timestamp drift < 5 ms
- Highly extensible:
  - Easy to add new APIs, trace formats, and metrics


## Supported backend APIs

As long as the backend supports these APIs, we can use the trace replayer to replay the traces. The current supported APIs are: 

- **OpenAI API**: `http://endpoint:port/v1/chat/completion` (non-streaming)
- **TGI (Text Generation Inference)**: `http://endpoint:port/generate` (non-streaming)
- **AIBrix**


## Getting Started

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


### 3. Init backend and trace-replayer

```bash
# Example
# Init vLLM as target
vllm serve /path/to/Qwen2.5-7B-Instruct --port 8080

# Now init trace-replayer
path/to/your/repo/target/release/client \
  --tokenizer /path/to/Qwen2.5-7B-Instruct/tokenizer.json \
  --tokenizer-config /path/to/Qwen2.5-7B-Instruct/tokenizer_config.json \
  --endpoint http://localhost:8080/v1/chat/completions \
  --api openai \
  --dataset bailian \
  --dataset-path /path/to/qwen_traceA_blksz_16.jsonl \
  --scale-factor 1.5 \
  --time-in-secs 1200 \
  --num-producer 32 \
  --channel-capacity 40960 \
  --output-path /path/to/client.jsonl \
  --model-name <MODEL_NAME IN VLLM>
```

For a complete list of command-line arguments, please refer to  
👉 [`docs/arguments.md`](docs/arguments.md)

### 4.Check Results

After execution:

* All results are written to the specified output file
* Each request produces one line in the **`.jsonl`** file

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


## Contributing
We welcome and value any contributions and feedback, please check
👉 [`docs/extending.md`](docs/extending.md) for how to extend API


## Sponsor

This project is sponsored by **Alibaba Tongyi Lab**.

## Contact
For technical questions, bug reports, and feature requests, please use
GitHub [Issues](https://github.com/blitz-serving/trace-replayer/issues).

For other inquiries, you may contact the maintainers via GitHub.([Healthcliff-Ding](https://github.com/Healthcliff-Ding))