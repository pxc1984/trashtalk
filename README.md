# Telegram Chat Tokenizer & Markov Text Generator (Rust + gRPC)

## 1. Project Overview

This project implements a **simple statistical text generation system** (n-gram / Markov-chain–based) trained on Telegram chat exports.  
The system ingests Telegram chat logs, tokenizes messages (including custom emojis), builds token transition statistics, and exposes a **gRPC API** for text generation.

The project is written in **Rust** and uses **gRPC (tonic)** for external interaction.

Key goals:
- Deterministic, reproducible tokenization of Telegram data
- Clear separation between ingestion/training and text generation
- Strong typing and correctness guarantees
- Easy retraining by reprocessing exported chats

---

## 2. High-Level Architecture

The application runs as a **single Rust binary** with **two long-lived workers**:

1. **Ingestion & Tokenization Worker**
   - Runs in a dedicated background thread
   - Watches and processes Telegram export directories
   - Parses `result.json`
   - Normalizes messages into tokens
   - Updates token and n-gram statistics in PostgreSQL

2. **gRPC Server Worker**
   - Runs in the main async runtime
   - Exposes a gRPC API for:
     - Text generation
     - Introspection / health checks
   - Reads token statistics from PostgreSQL (and optional in-memory cache)

---

## 3. Directory Layout

```text
.
├── Cargo.toml
├── Cargo.lock
├── README.md
├── .env.example <- Example environment config
├── build.rs <- makes protobuf code
├── Dockerfile
├── docker-compose.yml
├── exports/
│   └── ChatExport_*/
│       ├── result.json
│       ├── stickers/
│       └── video_files/
├── proto/
│   └── generator.proto
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── db/
│   │   ├── mod.rs
│   │   └── schema.rs
│   ├── tokenizer/
│   │   ├── mod.rs
│   │   ├── telegram.rs
│   │   └── tokens.rs
│   ├── trainer/
│   │   └── ngrams.rs
│   ├── grpc/
│   │   ├── mod.rs
│   │   └── generator.rs
│   └── workers/
│       ├── ingestion.rs
│       └── grpc_server.rs
```

---

## 4. Telegram Export Processing

### 4.1 Input Format

Each export directory:

```text
exports/ChatExport_*/
├── result.json
├── stickers/
└── video_files/
```

The system **must only depend on `result.json`** for tokenization.
Media files may be referenced but are not required for generation.

---

### 4.2 Message Parsing Rules

* Messages are parsed from `result.json`
* Only objects with `"type": "message"` are processed
* System messages may be ignored or handled separately

---

### 4.3 Token Types

All text must be normalized into a **token stream**.

Supported token types:

* `Word`
* `Punctuation`
* `Whitespace` (optional, implementation-defined)
* `Newline`
* `CustomEmoji`

Custom emojis:

* Identified by `document_id`
* Stored as first-class entities (not raw text)

Tokenization must be:

* Deterministic
* Locale-aware (Unicode-safe)
* Reproducible across runs

---

## 5. Storage Requirements (PostgreSQL)

The system must persist:

* Raw messages (optional but recommended)
* Normalized tokens
* Custom emojis
* Token vocabulary
* N-gram statistics

All training data must be **rebuildable** by reprocessing exports.

---

## 6. Background Worker: Ingestion & Tokenization

### Responsibilities

* Scan `./exports/` directory at startup
* Detect new or unprocessed export directories
* For each export:

  1. Parse `result.json`
  2. Tokenize all messages
  3. Insert/update:

     * Emoji records
     * Token vocabulary
     * N-gram statistics

### Execution Model

* Runs in a **dedicated OS thread**
* Communicates with the database only
* Must not block gRPC request handling
* Errors must be logged but not crash the server

---

## 7. Text Generation Model

* N-gram–based (configurable N, default = 3)
* Uses token IDs internally
* Generation is probabilistic (weighted by frequency)

Generation loop:

1. Normalize input prefix into tokens
2. Select the last `N-1` tokens as context
3. Query possible next tokens
4. Sample next token
5. Repeat until:

   * Token limit reached
   * End-of-sequence condition met

---

## 8. gRPC API (tonic)

### 8.1 Service Definition

The project must expose a gRPC service named:

```
TextGenerator
```

### 8.2 Required RPC Methods

#### `GenerateText`

Generates text based on a given prefix.

* **Input**

  * `prefix: string`
  * `max_tokens: uint32`
* **Output**

  * `text: string`
  * `tokens_generated: uint32`

#### `TokenizePreview`

Returns how input text would be tokenized.

* **Input**

  * `text: string`
* **Output**

  * List of tokens (type + value)

#### `HealthCheck`

Returns service status.

* **Input**

  * Empty
* **Output**

  * Status enum (`OK`, `DEGRADED`, `ERROR`)

#### `ReloadTrainingData` (optional)

Triggers reloading or retraining without restart.

---

## 9. Concurrency Model

* gRPC server runs on **Tokio async runtime**
* Ingestion worker runs in a **separate thread**
* Database connections are pooled
* Shared in-memory caches (if any) must be thread-safe

---

## 10. Configuration

Configuration is loaded from:

* Environment variables
* Optional `.env` file

Required settings:

* PostgreSQL connection URL
* gRPC bind address
* N-gram size
* Maximum generation length

---

## 11. Rust & Dependency Management Rules

### 11.1 Adding Dependencies

**Do NOT manually edit `Cargo.toml`.**

All dependencies must be added using:

```bash
cargo add <crate>@latest
```

Example:

```bash
cargo add tonic@latest
cargo add sqlx@latest
cargo add serde@latest
```

This rule applies to **all AI agents and contributors**.

---

## 12. Code Quality & Linting

Before submitting or merging code, the following must pass:

### Formatting

```bash
cargo fmt --all
```

### Linting

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Tests (if present)

```bash
cargo test
```

Code that fails **Clippy with warnings denied** must not be merged.

---

## 13. Non-Goals (Explicitly Out of Scope)

* Neural networks or embeddings
* Vector databases
* Online Telegram scraping
* Real-time chat ingestion
* Media analysis (audio/video)

---

## 14. Future Extensions (Not Required)

* Redis caching for hot prefixes
* Per-user or per-chat models
* Emoji rendering pipelines
* Web UI or HTTP gateway

---

## 15. Summary

This project is a **deterministic, Rust-based text generation system** built on Telegram chat history, emphasizing:

* Correct tokenization
* Explicit emoji handling
* Clear separation of concerns
* Safe concurrency
* Strong operational discipline

All implementation decisions must prioritize **reproducibility, clarity, and correctness** over premature optimization.
