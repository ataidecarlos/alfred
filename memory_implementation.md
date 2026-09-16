# Memory System Implementation Plan

## Context

Alfred needs a structured memory system inspired by Obsidian vaults. Memories are stored as `.md` files in `C:\Users\ataid\vaults\Ataide\Alfred`, organized by topic with wiki links, frontmatter properties, and MOCs. A multi-stage distillation pipeline (sessions → complexity scoring → intermediate store → LLM memory creation) ensures only relevant, distilled insights become memories. The existing flat `memories` SQLite table is retired after migration.

### Processing Tiers

The system supports three processing tiers, from cheapest to most powerful:

1. **Cloud LLM (default)** — User selects a cloud model. Memory is enabled by default. Small/fast models recommended for cost control (~$3-8/month).
2. **Local NPU/GPU pre-processing (optional)** — User enables "use local NPU/GPU" for complexity scoring and basic extraction. Runs on-device at $0. Requires Copilot+ PC (Snapdragon NPU), Intel NPU, or AMD iGPU.
3. **Full local LLM (optional)** — User enables "run all locally" for the entire pipeline. No cloud calls at all. Requires Ollama or similar with a capable GPU or 16GB+ RAM.

Tiers are not mutually exclusive: Tier 2 replaces only the initial scoring/extraction step (still needs Tier 1 or 3 for deep distillation and memory creation). Tier 3 replaces everything.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Alfred Memory System                              │
│                                                                             │
│  ┌──────────────┐    ┌──────────────┐    ┌───────────────────────────────┐  │
│  │   Sessions   │───▶│   Backlog    │───▶│  Complexity Scoring           │  │
│  │  (SQLite)    │    │  (pending    │    │                               │  │
│  │              │    │   distill)   │    │  ┌─────────┐  ┌────────────┐ │  │
│  └──────────────┘    └──────────────┘    │  │ Tier 1: │  │ Tier 2:    │ │  │
│                                          │  │ Cloud   │  │ Local NPU/ │ │  │
│                                          │  │ LLM     │  │ GPU ($0)   │ │  │
│                                          │  └─────────┘  └────────────┘ │  │
│                                          └───────────────┬───────────────┘  │
│                                                          │                  │
│                              ┌───────────────────────────┼────────────┐     │
│                              │                           │            │     │
│                              ▼                           ▼            ▼     │
│                    ┌──────────────┐            ┌──────────────┐            │
│                    │ Easy (70-80%)│            │ Med/Complex  │            │
│                    │ Extracts     │            │ (20-30%)     │            │
│                    │ basic info   │            │              │            │
│                    └──────┬───────┘            └──────┬───────┘            │
│                           │                           │                    │
│                           │              ┌────────────┴────────────┐       │
│                           │              │                         │       │
│                           │              ▼                         ▼       │
│                           │    ┌──────────────────┐    ┌────────────────┐  │
│                           │    │ Tier 1 or 3:     │    │ Tier 1 only:   │  │
│                           │    │ LLM distills     │    │ Basic info     │  │
│                           │    │ deep insights    │    │ only (no deep  │  │
│                           │    │                  │    │ distillation)  │  │
│                           │    └────────┬─────────┘    └────────┬───────┘  │
│                           │             │                       │          │
│                           └─────────────┼───────────────────────┘          │
│                                         │                                  │
│                                         ▼                                  │
│                    ┌──────────────────────────────────────────────────┐    │
│                    │           Intermediate Store (SQLite)            │    │
│                    │  - Extracted insights (not yet memories)         │    │
│                    │  - Complexity scores                             │    │
│                    │  - Pending review/creation                       │    │
│                    └────────────────────┬─────────────────────────────┘    │
│                                         │                                  │
│                                         ▼                                  │
│                    ┌──────────────────────────────────────────────────┐    │
│                    │    Memory Creation (Tier 1 or Tier 3)            │    │
│                    │  - Is this relevant enough for a memory?         │    │
│                    │  - Does a similar memory already exist?          │    │
│                    │  - If yes: create new .md in Obsidian vault      │    │
│                    │  - If exists: enhance/update existing memory     │    │
│                    └────────────────────┬─────────────────────────────┘    │
│                                         │                                  │
│                                         ▼                                  │
│                    ┌──────────────────────────────────────────────────┐    │
│                    │         Obsidian Vault (final destination)       │    │
│                    │  C:\Users\ataid\vaults\Ataide\Alfred\            │    │
│                    │  - .md files with frontmatter                    │    │
│                    │  - Wikilinks, MOCs, Bases                        │    │
│                    │  - retrieval_count tracking                      │    │
│                    └──────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Vault Structure

```
C:\Users\ataid\vaults\Ataide\Alfred/
├── _templates/          # Obsidian note templates
│   ├── memory.md        # Generic memory template
│   ├── decision.md      # Decision with alternatives
│   └── moc.md           # Map of Content template
├── _attachments/        # Non-content files (images, etc.)
├── _config/
│   └── topics.md        # Controlled vocabulary (seed list + extensible)
│
├── inbox/               # New/unprocessed memories
├── preferences/         # ⚙️ User preferences + _index.md MOC
├── facts/               # 📋 Facts & knowledge + _index.md MOC
├── decisions/           # 🔒 Decision log + _index.md MOC
├── lessons/             # 💡 Lessons learned + _index.md MOC
├── action-items/        # ✅ Follow-ups + _index.md MOC
├── daily/               # Daily logs (only significant days)
│
├── index.md             # Home note — vault entry point
└── alfred-index.base    # Base file for querying all memories
```

### Memory Note Schema (frontmatter)

```yaml
---
title: "User prefers Portuguese"
type: preference              # preference | fact | decision | lesson | action-item
topics:
  - language
  - communication
status: active                # active | archived | superseded
created: 2026-09-16
updated: 2026-09-16
source: conversation          # conversation | observation | explicit
aliases:
  - Portuguese preference
tags:
  - memory
  - preference
retrieval_count: 0
last_retrieved: null
importance: high              # low | medium | high
distilled_from: "conversation-2026-09-16-001"
---
```

---

## Implementation Steps

### Group 1: Configuration & Schema

**Step 1.1 — Add `[memory]` config section**
- Target: `src/config.rs`
- Add `MemoryConfig` struct with fields:
  - Core: `vault_path`, `mode` (auto/cli/files), `cli_check_interval_secs`, `retrieval_review_threshold_days`
  - Tier 1 (Cloud): `cloud_provider`, `cloud_model`, `cloud_monthly_limit`
  - Tier 2 (Local NPU/GPU): `local_preprocessing`, `local_backend` (auto/foundry_local/nexa/llama_cpp/openvino), `local_model`
  - Tier 3 (Full Local): `full_local`, `local_llm_provider` (ollama/foundry_local/custom), `local_llm_base_url`, `local_llm_model`
  - Pipeline: `distillation_interval_secs`
- Add `pub memory: MemoryConfig` to `AppConfig`
- Add `default_vault_path()` returning `"C:\\Users\\ataid\\vaults\\Ataide\\Alfred"`
- Error handling: if `vault_path` is missing, use default; if path doesn't exist, log warning (vault scaffolding step creates it)

**Step 1.2 — Add distillation tables to SQLite**
- Target: `src/store/mod.rs`, inside `Store::new()`
- Add three new tables to the `execute_batch` call:
  - `distillation_backlog(session_id TEXT PK, status TEXT, complexity_score TEXT, complexity_confidence REAL, npu_distilled_at INTEGER, llm_distilled_at INTEGER, created_at INTEGER)`
  - `distilled_insights(id TEXT PK, session_id TEXT, complexity TEXT, tools TEXT, files TEXT, outcome TEXT, summary TEXT, basic_lesson TEXT, detailed_lessons TEXT, patterns TEXT, principles TEXT, status TEXT DEFAULT 'pending', memory_id TEXT, created_at INTEGER)`
  - `memory_index(id TEXT PK, path TEXT, title TEXT, type TEXT, status TEXT, topics TEXT, created_at INTEGER, updated_at INTEGER, retrieval_count INTEGER DEFAULT 0, last_retrieved INTEGER)`
- Add `MemoryRecord` struct: `{ id, path, title, mem_type, status, topics, created_at, updated_at, retrieval_count, last_retrieved }`
- Error handling: `CREATE TABLE IF NOT EXISTS` — idempotent, no error if tables exist

**Step 1.3 — Add memory CRUD methods to Store**
- Target: `src/store/mod.rs`
- Add methods:
  - `upsert_memory_index(record: &MemoryRecord)` — INSERT OR REPLACE into memory_index
  - `get_memory_index() -> Vec<MemoryRecord>` — SELECT all from memory_index
  - `update_memory_retrieval(id: &str)` — increment retrieval_count, set last_retrieved
  - `get_stale_memories(threshold_days: i64) -> Vec<MemoryRecord>` — WHERE last_retrieved IS NULL OR last_retrieved < threshold
  - `add_to_backlog(session_id: &str)` — INSERT into distillation_backlog
  - `get_pending_backlog() -> Vec<String>` — SELECT session_id WHERE status = 'pending'
  - `update_backlog_status(session_id: &str, status: &str)` — UPDATE status
  - `add_distilled_insight(insight: &DistilledInsight)` — INSERT into distilled_insights
  - `get_pending_insights() -> Vec<DistilledInsight>` — SELECT WHERE status = 'pending'
  - `update_insight_status(id: &str, status: &str, memory_id: Option<&str>)` — UPDATE status + memory_id
- Add `DistilledInsight` struct matching the table columns
- Error handling: all methods return `Result<_, AlfredError>`

### Group 2: Vault Scaffolding

**Step 2.1 — Create vault directory structure**
- Target: `src/memory/vault.rs` (new module)
- Create function `scaffold_vault(vault_path: &Path) -> Result<(), AlfredError>`
- Create directories: `_templates/`, `_attachments/`, `_config/`, `inbox/`, `preferences/`, `facts/`, `decisions/`, `lessons/`, `action-items/`, `daily/`
- Create `_config/topics.md` with seed vocabulary (Communication, Work, Technical, Personal, People, Projects categories)
- Create `_templates/memory.md` with frontmatter template (title, type, topics, status, created, updated, source, aliases, tags, retrieval_count, last_retrieved, importance)
- Create `_templates/decision.md` with decision-specific frontmatter (adds alternatives_rejected, rationale)
- Create `_templates/moc.md` with MOC template
- Create `index.md` (home note) with links to all category MOCs
- Create `alfred-index.base` with table views for All Memories and Recent Memories
- Create `_index.md` in each category folder with MOC content
- Error handling: `create_dir_all` is idempotent; file creation uses `OpenOptions::create_new(true)` to avoid overwriting existing files; log warning if file already exists

**Step 2.2 — Add vault module to project**
- Target: `src/memory/mod.rs` (new module)
- `pub mod vault;`
- Add `mod memory;` to `src/main.rs`
- Call `vault::scaffold_vault(&vault_path)` in `ensure_directories()` in `src/main.rs`

### Group 3: Obsidian Markdown Engine

**Step 3.1 — Frontmatter parser/serializer**
- Target: `src/memory/frontmatter.rs` (new file)
- Create `MemoryFrontmatter` struct with all fields (title, type, topics, status, created, updated, source, aliases, tags, retrieval_count, last_retrieved, importance, distilled_from)
- Implement `parse(content: &str) -> Result<(MemoryFrontmatter, String), AlfredError>` — split on `---` delimiters, parse YAML with `serde_yaml`
- Implement `serialize(fm: &MemoryFrontmatter, body: &str) -> String` — format frontmatter as YAML, combine with body
- Derive `Serialize, Deserialize` for `MemoryFrontmatter`
- Error handling: if frontmatter is malformed, return `AlfredError::Config` with line number; if no frontmatter found, return empty frontmatter + full content as body

**Step 3.2 — Wikilink parser**
- Target: `src/memory/frontmatter.rs` (same file)
- Create function `extract_wikilinks(content: &str) -> Vec<String>` — regex `\[\[([^\]|]+)(?:\|[^\]]+)?\]\]` to extract link targets
- Create function `add_wikilink(content: &str, target: &str) -> String` — append `[[target]]` to a "Related" section at the bottom, or create the section if missing
- Create function `remove_wikilink(content: &str, target: &str) -> String` — remove `[[target]]` from content
- Error handling: regex compilation is done once at module level with `lazy_static!` or `std::sync::LazyLock`; no runtime errors expected

**Step 3.3 — Memory file reader/writer**
- Target: `src/memory/vault.rs`
- Create function `read_memory(path: &Path) -> Result<(MemoryFrontmatter, String), AlfredError>` — read file, parse frontmatter + body
- Create function `write_memory(path: &Path, fm: &MemoryFrontmatter, body: &str) -> Result<(), AlfredError>` — serialize frontmatter + body, write atomically (write to `.tmp`, rename)
- Create function `scan_vault(vault_path: &Path) -> Result<Vec<(PathBuf, MemoryFrontmatter)>, AlfredError>` — walk directory, parse each `.md` file's frontmatter, return list
- Create function `update_moc(vault_path: &Path, category: &str) -> Result<(), AlfredError>` — scan category folder, collect all memory titles, rewrite `_index.md` with wikilinks
- Error handling: file-not-found returns error; parse errors include file path; write uses atomic rename to prevent corruption

### Group 4: Memory Tool

**Step 4.1 — Memory tool implementation**
- Target: `src/tools/memory.rs` (new file)
- Create `MemoryTool` struct holding `Arc<Store>` and `vault_path: PathBuf`
- Create `MemoryArgs` struct with `action: String`, plus optional fields: `query`, `id`, `title`, `content`, `mem_type`, `topics` (Vec<String>), `source`, `target_id`, `importance`
- Implement `Tool` trait:
  - `definition()` — name "memory", description, JSON schema from `MemoryArgs`
  - `execute(args)` — match on action:
    - `"store"` — validate title+content+mem_type, generate kebab-case filename from title, write `.md` to `{category}/` folder, update MOC, upsert memory_index, return success with path
    - `"recall"` — search memory_index by query (match against title, topics, type), read matching files, return formatted results with snippets; call `update_memory_retrieval` for each hit
    - `"link"` — read source file, add wikilink to target, write back
    - `"update"` — read file, modify frontmatter/body, write back, update MOC
    - `"archive"` — read file, set `status: archived` in frontmatter, write back
    - `"review"` — call `store.get_stale_memories(90)`, return formatted list
    - `"topics"` — read `_config/topics.md`, return content
    - `"recent"` — scan memory_index sorted by updated_at desc, limit 20
    - `"rebuild"` — for each category folder, call `update_moc`
- Error handling: each action returns `ToolOutput::success` or `ToolOutput::error`; file-not-found during recall returns "no memories found"; invalid action returns error

**Step 4.2 — Register memory tool**
- Target: `src/tools/mod.rs`
- Add `pub mod memory;`
- In `register_builtins()`, add `registry.register(Arc::new(memory::MemoryTool::new(store, vault_path)))`
- Pass `vault_path` from config through `AppState` to `register_builtins`

**Step 4.3 — Wire vault_path through AppState**
- Target: `src/server/mod.rs` (or wherever `AppState` is defined)
- Add `vault_path: PathBuf` to `AppState`
- In `initialize_state()` in `src/main.rs`, read `config.memory.vault_path` and pass to `AppState`
- Pass `vault_path` to `register_builtins()`

### Group 5: Distillation Pipeline

**Step 5.1 — Local pre-processor (NPU/GPU, Tier 2)**
- Target: `src/memory/local_preprocessor.rs` (new file)
- This step is **optional** — only runs if `memory.local_preprocessing = true`
- Create `LocalPreprocessor` trait with method `score_complexity(&self, transcript: &str) -> Result<ComplexityResult, AlfredError>`
- Implement backend-specific scorers:
  - `FoundryLocalScorer` — calls Foundry Local API at `http://localhost:5273/v1/chat/completions` (OpenAI-compatible)
  - `NexaSdkScorer` — shells out to `nexa` CLI for NPU-optimized Qwen3 models
  - `LlamaCppScorer` — shells out to `llama-cli` with QNN backend for Snapdragon NPU
  - `OpenvinoScorer` — shells out to `optimum-cli` for Intel NPU
  - `AutoScorer` — detects available backend (Foundry Local → Nexa → llama.cpp → OpenVINO → fail)
- Create `ComplexityResult` struct: `{ complexity: String, confidence: f64, reasoning: String, tools: Vec<String>, files: Vec<String>, outcome: String, summary: String, basic_lesson: String }`
- Error handling: if backend unavailable, return sentinel error; pipeline catches and falls back to cloud LLM (Tier 1) for scoring
- If `local_preprocessing = false`, the pipeline skips this step entirely and uses cloud LLM for scoring

**Step 5.2 — LLM distiller (Tier 1 or Tier 3)**
- Target: `src/memory/distiller.rs` (new file)
- Create `LlmDistiller` struct that can use either a cloud provider (Tier 1) or a local LLM (Tier 3)
- Constructor takes a `LlmBackend` enum: `Cloud { provider, model }` or `Local { base_url, model }`
- `score_complexity(&self, transcript: &str) -> Result<ComplexityResult, AlfredError>` — used when Tier 2 is disabled; calls LLM for complexity scoring
- `distill_deep(&self, transcript: &str, pre_result: &ComplexityResult) -> Result<DeepDistillation, AlfredError>` — call LLM with distillation prompt, parse JSON output (detailed_lessons, patterns, principles, cross_session_insights)
- `assess_memory_relevance(&self, insight: &DistilledInsight, existing_memories: &[MemoryRecord]) -> Result<MemoryDecision, AlfredError>` — call LLM with assessment prompt + existing memory titles, parse decision (create_new / enhance_existing / reject) with reasoning
- Create `DeepDistillation` struct and `MemoryDecision` enum
- Error handling: LLM errors propagate; JSON parse errors include raw output for debugging; cost tracking: increment a counter, check against monthly limit before each call (Tier 1 only; Tier 3 has no cost)
- For Tier 3 (full local), the `base_url` points to Ollama (`http://localhost:11434/v1`) or Foundry Local (`http://localhost:5273/v1`)

**Step 5.3 — Distillation pipeline orchestrator**
- Target: `src/memory/pipeline.rs` (new file)
- Create `DistillationPipeline` struct holding `Arc<Store>`, `Option<Box<dyn LocalPreprocessor>>` (Tier 2), `Arc<LlmDistiller>` (Tier 1 or 3), `vault_path: PathBuf`, config flags
- `run_cycle(&self) -> Result<PipelineResult, AlfredError>` — main cycle:
  1. Get pending backlog items from store
  2. For each: read session transcript from conversations table
  3. **Complexity scoring**: If Tier 2 enabled, use local preprocessor; otherwise use LLM distiller (Tier 1 or 3)
  4. If easy: extract basic metadata → add to distilled_insights
  5. If medium/complex: LLM deep distill (Tier 1 or 3) → add to distilled_insights
  6. For each pending insight: LLM assesses relevance (Tier 1 or 3) → create/enhance memory or reject
  7. Update MOCs for any new/modified memories
  8. Return summary (processed count, created count, rejected count)
- Create `PipelineResult` struct with counts
- Error handling: individual session failures don't stop the cycle (log error, continue); local preprocessor failure falls back to LLM scoring; LLM cost limit exceeded stops further LLM calls for this cycle

**Step 5.4 — Integrate pipeline with scheduler**
- Target: `src/scheduler/mod.rs` (or wherever scheduled tasks are managed)
- Add a built-in scheduled task for distillation: runs every `distillation_interval_secs`
- Alternatively: create a dedicated background task in `src/main.rs` that spawns a tokio interval timer
- The task calls `pipeline.run_cycle()` and logs the result
- Error handling: pipeline errors are logged, don't crash the server

### Group 6: Migration

**Step 6.1 — Migrate existing SQLite memories**
- Target: `src/memory/migration.rs` (new file)
- Create function `migrate_sqlite_memories(store: &Arc<Store>, vault_path: &Path) -> Result<MigrationResult, AlfredError>`
- Read all memories from `store.list_memories()`
- For each memory:
  - Infer type from content heuristics: contains "prefer" → preference; contains "decided"/"chose" → decision; contains "learned"/"mistake" → lesson; contains "todo"/"remember to" → action-item; default → fact
  - Generate kebab-case filename from first 50 chars of content (slugify)
  - Create frontmatter with inferred type, today's date, status: active
  - Write `.md` file to appropriate category folder
- Rebuild all MOCs
- Log migration count
- Optionally: drop the `memories` table (or rename to `memories_backup`)
- Error handling: if vault doesn't exist, scaffold it first; if a file with the same name exists, append a numeric suffix; migration is idempotent (check if already migrated via a flag in the DB)

**Step 6.2 — Run migration on startup**
- Target: `src/main.rs`, in `initialize_state()`
- After `Store::new()`, check if migration has already run (a flag in the DB or check if memory_index is populated)
- If not migrated, call `migrate_sqlite_memories()`
- Log result

### Group 7: Prompt Integration

**Step 7.1 — Replace flat memory loading with vault-based loading**
- Target: `src/prompt/mod.rs`
- Replace `load_memories(store)` with `load_memories_from_vault(vault_path)`
- Scan vault for active memories (status != archived)
- Format as context: group by type, include title + first paragraph
- Inject into system prompt under "## User Memories"
- Error handling: if vault doesn't exist or is empty, return empty string (no memories section)

**Step 7.2 — Add memory instructions to system prompt**
- Target: `prompts/system.md`
- Add instructions about the memory tool: when to use it, how to recall memories
- Add instructions about memory creation criteria (explicit request, decision, lesson, preference, fact, action item, repeated pattern)
- Add instructions about NOT creating memories for trivial interactions

### Group 8: Obsidian Skills

**Step 8.1 — Install Obsidian skills**
- Target: `~/.opencode/skills/obsidian-skills/`
- Clone `https://github.com/kepano/obsidian-skills.git` to `~/.opencode/skills/obsidian-skills/`
- This is a one-time setup step, done outside the Rust code
- Document in README or AGENTS.md

**Step 8.2 — Reference skills in Alfred's prompt**
- Target: `prompts/system.md` or a new `prompts/memory.md`
- Add reference to Obsidian skills location for syntax guidance
- Note that skills are checked periodically for updates

### Group 9: Dependencies

**Step 9.1 — Add new Cargo dependencies**
- Target: `Cargo.toml`
- Add: `walkdir = "2"`, `serde_yaml = "0.9"`, `regex = "1"`, `slug = "0.1"` (for kebab-case filenames)
- Note: No ONNX Runtime crate needed — Tier 2 uses external processes (Foundry Local API, Nexa CLI, llama.cpp CLI, OpenVINO CLI) rather than in-process inference

---

## Model Recommendations

### Tier 1: Cloud LLMs (Default — User-Selected)

Memory is enabled by default. The user picks a cloud model for full control over cost. These are the best small/fast models for memory tasks (classification, extraction, summarization, relevance assessment):

| Model | Input $/M | Output $/M | Context | Best For | Notes |
|---|---|---|---|---|---|
| **Gemini 2.5 Flash-Lite** | $0.04–0.10 | $0.15–0.40 | 1M | Absolute cheapest | ~257 tok/s, great for high-volume classification |
| **GPT-4o-mini** | $0.15 | $0.60 | 128K | Reliable all-rounder | Best JSON accuracy, mature ecosystem |
| **DeepSeek V4-Flash** | $0.14 | $0.28 | 128K | Best price/quality | 90% cache discount available (~$0.014/M cached) |
| **GPT-5.6 Luna** | $0.20 | $1.20 | — | OpenAI budget tier | Newer, good instruction following |
| **Claude Haiku 4.5** | $1.00 | $5.00 | 200K | Best structured output | 90% cache discount; expensive at base rate |
| **Mistral Small 4** | $0.15 | $0.60 | — | Budget alternative | Competitive with GPT-4o-mini |

**Default recommendation**: `gpt-4o-mini` (most reliable JSON output for extraction tasks). For cost-conscious users: `gemini-2.5-flash-lite` or `deepseek-v4-flash`.

**Estimated monthly cost** (assuming ~100 sessions/day, ~2K tokens each for distillation):
- Gemini 2.5 Flash-Lite: ~$2–4/month
- GPT-4o-mini: ~$5–8/month
- DeepSeek V4-Flash: ~$3–5/month

### Tier 2: NPU/GPU Local Pre-Processing (Optional, $0)

User enables `local_preprocessing = true` for the initial extraction and complexity classification. Runs entirely on-device.

#### For Snapdragon X Elite / Copilot+ PCs (45 TOPS Hexagon NPU):

| Model | Size | Runtime | NPU Speed | RAM | Best For |
|---|---|---|---|---|---|
| **Phi-4-mini-instruct 3.8B** (INT4 QDQ) | ~2 GB | Foundry Local | **32 tok/s** | 3.4 GB | Zero-friction, best NPU speed |
| **Qwen3-8B Hybrid** (INT4) | ~5 GB | Nexa SDK | **26 tok/s** | 5.9 GB | Best quality on NPU |
| **DeepSeek-R1-Distill-Qwen-7B** (INT4 QDQ) | ~5 GB | Foundry Local | **23 tok/s** | 5.2 GB | Reasoning tasks |
| **Llama 3.1 8B** (q4_0) | ~5 GB | llama.cpp QNN | **24 tok/s** | ~5 GB | Flexible, well-supported |

**Key runtimes**:
- **Foundry Local** (Microsoft) — easiest setup, OpenAI-compatible API at `localhost:5273`, one-click NPU acceleration
- **Nexa SDK** — best NPU throughput for Qwen3 models
- **llama.cpp with QNN backend** — most flexible, supports any GGUF model

**Recommendation**: `phi-4-mini-instruct` via Foundry Local — smallest, fastest, zero configuration.

#### For Intel Core Ultra (OpenVINO NPU, ~10-15 TOPS):

| Model | Runtime | NPU Speed | Notes |
|---|---|---|---|
| Phi-3.5-mini (INT4) | OpenVINO | ~14 tok/s | Validated by Microsoft |
| Qwen 2.5 1.5B (INT4) | OpenVINO | ~18 tok/s | Smaller, faster |

#### For AMD Ryzen AI (XDNA NPU):
- NPU support in llama.cpp is not yet wired up for XDNA
- Fall back to CPU or iGPU (ROCm) paths
- AMD iGPU (Radeon 890M): ~19 tok/s for 8B models via ROCm

### Tier 3: Full Local LLM (Optional, $0 — Powerful GPUs)

For users who want to run everything locally with Ollama or similar. No cloud costs, full privacy.

#### For machines with dedicated GPUs (RTX 3060+, 12GB+ VRAM):

| Model | Size | VRAM | Speed (GPU) | Speed (CPU) | Best For |
|---|---|---|---|---|---|
| **Qwen3 8B** (Q4_K_M) | ~5 GB | 6 GB | ~60+ tok/s | ~18 tok/s | Best all-rounder |
| **Phi-4-mini 3.8B** (Q4_K_M) | ~2.5 GB | 3 GB | ~80+ tok/s | ~19 tok/s | Smallest good option |
| **Gemma 3 4B** (Q4_K_M) | ~3 GB | 3.5 GB | ~70+ tok/s | ~15 tok/s | General tasks |
| **Qwen2.5-Coder 7B** (Q4_K_M) | ~4.5 GB | 5 GB | ~55+ tok/s | ~16 tok/s | Code-heavy tasks |
| **DeepSeek-R1-Distill-7B** (Q4_K_M) | ~5 GB | 5.5 GB | ~50+ tok/s | ~14 tok/s | Reasoning |

#### For machines without dedicated GPU (CPU only, 16GB+ RAM):

| Model | Size | RAM | Speed | Notes |
|---|---|---|---|---|
| **Phi-4-mini 3.8B** (Q4_K_M) | ~2.5 GB | 3 GB | ~19 tok/s | Best CPU speed/quality |
| **Qwen3 8B** (Q3_K_M) | ~3.5 GB | 4 GB | ~12 tok/s | Good quality at lower quant |
| **Llama 3.2 3B** (Q4_K_M) | ~2 GB | 2.5 GB | ~44 tok/s | Fastest small option |

**Recommendation**: `qwen3:8b` for GPU users (best quality/speed balance), `phi4-mini:3.8b` for CPU-only users (smallest that's still good at JSON extraction).

---

## Configuration

```toml
[memory]
# Core
enabled = true
vault_path = "C:\\Users\\ataid\\vaults\\Ataide\\Alfred"
mode = "auto"                         # auto | cli | files
cli_check_interval_secs = 300
retrieval_review_threshold_days = 90  # Flag for review after 90 days of no retrieval

# Pipeline
distillation_interval_secs = 7200     # 2 hours

# ─── Tier 1: Cloud LLM (default processing) ───
cloud_provider = "openai"             # openai | anthropic | google | deepseek | mistral
cloud_model = "gpt-4o-mini"           # Recommended: gpt-4o-mini, gemini-2.5-flash-lite, deepseek-v4-flash
cloud_monthly_limit = 5.0             # Max $ per month

# ─── Tier 2: Local NPU/GPU pre-processing (optional, $0) ───
# Replaces cloud for complexity scoring + basic extraction only.
# Still needs Tier 1 or Tier 3 for deep distillation + memory creation.
local_preprocessing = false
local_backend = "auto"                # auto | foundry_local | nexa | llama_cpp | openvino
local_model = "phi-4-mini-instruct"   # For NPU: phi-4-mini, qwen3-8b-hybrid, deepseek-r1-distill-7b

# ─── Tier 3: Full local LLM (optional, $0, replaces cloud entirely) ───
# Runs the entire pipeline locally. No cloud calls at all.
full_local = false
local_llm_provider = "ollama"         # ollama | foundry_local | custom
local_llm_base_url = "http://localhost:11434/v1"
local_llm_model = "qwen3:8b"          # For GPU: qwen3:8b. For CPU: phi4-mini:3.8b
```

---

## Critical Files & Anchors

| File | Anchor | Reason |
|---|---|---|
| `src/store/mod.rs:34` | `execute_batch()` in `Store::new()` | Add 3 new tables here; all distillation state lives in SQLite |
| `src/tools/mod.rs:10` | `register_builtins()` | Register new memory tool alongside todo/webhook/shell |
| `src/config.rs:10` | `AppConfig` struct | Add `MemoryConfig` field; all memory settings flow from here |
| `src/prompt/mod.rs:39` | `load_memories()` | Replace flat SQLite loading with vault-based scanning |
| `src/main.rs:255` | `initialize_state()` | Wire vault_path, run migration, start distillation pipeline |

---

## Verification

1. **Vault scaffolding**: Start Alfred with `memory.vault_path` set. Verify `C:\Users\ataid\vaults\Ataide\Alfred\` is created with all subdirectories, templates, `_config/topics.md`, `index.md`, `alfred-index.base`, and `_index.md` in each category folder.

2. **Memory tool — store**: Send message "Remember that I prefer Portuguese for all responses." Verify agent calls `memory action=store title="User prefers Portuguese" mem_type="preference" content="..."`. Verify file `preferences/user-prefers-portuguese.md` is created with correct frontmatter. Verify `preferences/_index.md` MOC is updated with `[[user-prefers-portuguese]]`.

3. **Memory tool — recall**: Send message "What do you know about my language preferences?" Verify agent calls `memory action=recall query="language preferences"`. Verify it returns the Portuguese preference memory with content snippet.

4. **Memory tool — link**: Send message "Link the Portuguese preference to the email templates note." Verify agent calls `memory action=link`. Verify wikilink is added.

5. **Migration**: Start Alfred with existing memories in SQLite. Verify memories are converted to `.md` files in appropriate category folders. Verify MOCs are updated. Verify `memory_index` table is populated.

6. **Distillation pipeline**: Create a test conversation in SQLite. Wait for distillation cycle (or trigger manually). Verify entry appears in `distillation_backlog`, then `distilled_insights`. If LLM distillation is enabled, verify memory is created in vault.

7. **Obsidian integration**: Open vault in Obsidian. Verify all notes render correctly with frontmatter, wikilinks resolve, MOCs show linked notes, `alfred-index.base` shows table views.

8. **Prompt integration**: Start a conversation. Verify system prompt includes "## User Memories" section with memories from vault (not SQLite).

---

## Assumptions & Contingencies

1. **Assumption**: Cloud LLM API is available and user has configured API keys.
   - **Contingency**: If API key is missing, memory system is disabled with a clear error message. User must configure `cloud_provider` and `cloud_model` or enable Tier 3 (full local).

2. **Assumption**: Local NPU/GPU backend (Foundry Local, Nexa SDK, llama.cpp, OpenVINO) is installed when `local_preprocessing = true`.
   - **Contingency**: If backend is unavailable, preprocessor returns a sentinel error. Pipeline falls back to cloud LLM for complexity scoring. Log warning with installation instructions.

3. **Assumption**: Ollama or Foundry Local is running when `full_local = true`.
   - **Contingency**: If local LLM endpoint is unreachable, return error. Log warning with startup instructions.

4. **Assumption**: Obsidian CLI is available when Obsidian is running.
   - **Contingency**: If CLI is unavailable, fall back to direct file I/O. Detection runs every 5 minutes.

5. **Assumption**: The vault folder is writable by the Alfred process.
   - **Contingency**: If write fails, log error and continue. Memory tool returns error to agent.

6. **Assumption**: Existing SQLite memories can be classified by simple heuristics (keyword matching).
   - **Contingency**: If heuristics produce poor results, default all to `fact` type. User can reclassify in Obsidian.

7. **Assumption**: Cloud LLM cost is acceptable at ~$3-8/month for typical usage.
   - **Contingency**: `cloud_monthly_limit` caps spending. If limit is reached, pipeline stops making cloud calls until next month. User can increase limit or switch to Tier 3 (full local).

8. **Assumption**: Local LLM (Tier 3) has sufficient quality for memory extraction tasks.
   - **Contingency**: Smaller models (3B) may produce less accurate JSON extraction than cloud models. If extraction fails repeatedly, log warning suggesting user upgrade to a larger model or switch to cloud.
