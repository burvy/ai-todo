# Jev (TypeSafe System One) HTTP API Spike

> TypeSafe Jev provides fast, typed, calibrated probability decisions over unstructured text and state, eliminating generative hallucination in the understanding pipeline.

---

## 1. Authentication & Endpoints

- **Environment Variable**: `TYPESAFE_API_KEY`
- **Request Header**: `Authorization: Bearer <API_KEY>`
- **Content-Type**: `application/json`
- **Model**: `"jev-latest"` (alias for current release `"jev-1.13.0"`)

| Operation | Method & URL | Description |
| :--- | :--- | :--- |
| **Evaluate** | `POST https://api.typesafe.ai/v1/systemone` | Core evaluation endpoint; evaluates `state` against typed `questions`. |
| **List Models** | `GET https://api.typesafe.ai/v1/models` | Lists available models and aliases (`jev-latest`, `jev-preview`). |

---

## 2. Request & Response Architecture

### Top-Level Request
```json
{
  "state": "Raw transcript, document text, or structured JSON context",
  "model": "jev-latest",
  "questions": {
    "<question_id>": { "type": "noul | choice | score", "instructions": "...", "criteria": ... }
  }
}
```
*Notes*: `state` can be a raw string or structured JSON object/array. Dot-and-index syntax (e.g. `` `span.text` ``) in `instructions` can reference nested fields. Question IDs are client-defined and returned verbatim.

### Top-Level Response
```json
{
  "model": "jev-1.13.0",
  "answers": {
    "<question_id>": { "type": "...", ... }
  },
  "usage": { "input_tokens": 312, "output_tokens": 42 }
}
```

---

## 3. Core Question Primitives

### Noul (Binary Probability)
Answers yes/no questions with a calibrated probability $p \in [0.0, 1.0]$.
- **Question**: `type`: `"noul"`, `instructions`: string/object, optional `criteria`: `{ "true": "...", "false": "..." }`.
- **Answer**: `type`: `"noul"`, `noul`: float (probability of `true`). *No separate confidence field is returned; uncertainty is measured by distance from 0.5 ($|2p - 1|$)*.

```json
// Question
"is_actionable": {
  "type": "noul",
  "instructions": "Does this text express a concrete task or commitment?",
  "criteria": { "true": "Clear task or action item", "false": "Observation, fact, or chit-chat" }
}
// Answer
"is_actionable": { "type": "noul", "noul": 0.94 }
```

### Choice (Categorical Selection)
Selects one option from a discrete set (up to 255 options) and returns full probability distribution plus confidence.
- **Question**: `type`: `"choice"`, `instructions`: string/object, `criteria`: map of option key to description.
- **Answer**: `type`: `"choice"`, `choice`: string (highest probability option), `probabilities`: map of option to float (sum to 1.0), `confidence`: float ($[0.0, 1.0]$, derived from separation over uniform distribution $\frac{p_{\max} - 1/n}{1 - 1/n}$).

```json
// Question
"deadline_pick": {
  "type": "choice",
  "instructions": "Which parsed date represents the intended deadline for the task?",
  "criteria": {
    "opt_friday": "Friday by 5pm",
    "opt_next_week": "Sometime next week",
    "none": "No deadline specified"
  }
}
// Answer
"deadline_pick": {
  "type": "choice",
  "choice": "opt_friday",
  "probabilities": { "opt_friday": 0.88, "opt_next_week": 0.08, "none": 0.04 },
  "confidence": 0.82
}
```

### Score (Continuous Ordered Rubric)
Rates state along 2 to 10 ordered levels. Returns probability-weighted expectation score ($[0, n-1]$) and dispersion confidence.
- **Question**: `type`: `"score"`, `instructions`: string/object, `criteria`: array of ordered level descriptions.
- **Answer**: `type`: `"score"`, `score`: float (expected value across levels), `legend`: map of level indices to descriptions, `probabilities`: map of index string to float, `confidence`: float ($[0.0, 1.0]$, lower if probability is dispersed across distant levels).

```json
// Question
"priority": {
  "type": "score",
  "instructions": "What is the priority level of this task?",
  "criteria": [
    "P3 - Low: Nice to have, no urgency",
    "P2 - Normal: Routine task",
    "P1 - High: Time-sensitive or significant impact",
    "P0 - Critical: Blocker or emergency"
  ]
}
// Answer
"priority": {
  "type": "score",
  "score": 2.15,
  "legend": { "0": "P3 - Low...", "1": "P2 - Normal...", "2": "P1 - High...", "3": "P0 - Critical..." },
  "probabilities": { "0": 0.02, "1": 0.11, "2": 0.57, "3": 0.30 },
  "confidence": 0.74
}
```

---

## 4. Batching & Performance Patterns

1. **Single-Pass Evaluation**: All questions in the `questions` map run concurrently over `state`.
2. **Cost & Latency Efficiency**: `state` tokens are billed and ingested once. Batching 10 questions is ~10x faster and ~12x cheaper than 10 individual calls.
3. **Output Tokens Free**: Output tokens incur \$0.00 cost.
4. **Speculative Fan-Out**: Send all candidate checks (taskness, deadline, priority, effort, category) in a single request. If `is_a_task.noul < 0.50`, discard the downstream answers in Rust code with zero wasted round-trips.

---

## 5. Limits, Pricing & Errors

| Metric / Error | Value / Specification |
| :--- | :--- |
| **Pricing** | **\$42 / Btok (\$0.042 / Mtok)** on input tokens. **\$0.00** on output tokens. |
| **Rate Limits** | **100,000 tokens/sec** and **80 requests/sec**. Dynamic scaling. |
| **Context Length** | **64k tokens** total request budget; **32k tokens** max for `state` + longest question. |
| **Input Format** | Text only (plain string, JSON object, or string array). |
| `401 Unauthorized` | Missing/invalid `TYPESAFE_API_KEY`. |
| `422 Unprocessable` | Malformed request body (missing required fields, >255 Choice options, <2 or >10 Score levels). |
| `429 Rate Limit` | Token or RPS exceeded. Honor `retry-after` header and back off exponentially. |
| `529 Overloaded` | Server temporarily saturated; retry with jittered backoff. |

---

## 6. Pipeline Question Specs (ai-todo)

In `crates/capture` and `crates/judge`, speech/text is segmented into candidate spans and dates are extracted via regex/duckling. Jev performs typed evaluation on each candidate in one batched call.

### Draft Batched Request
```json
{
  "state": {
    "transcript": "Hey, need to file quarterly tax report before Friday 5pm. It's super urgent because penalties apply. Will probably take 2 to 3 hours to pull all receipts together.",
    "candidate_span": "file quarterly tax report before Friday 5pm",
    "parsed_date_candidates": [
      { "id": "d1", "text": "Friday 5pm", "resolved_utc": "2026-10-16T21:00:00Z" }
    ]
  },
  "model": "jev-latest",
  "questions": {
    "is_a_task": {
      "type": "noul",
      "instructions": "Does `candidate_span` from `transcript` represent an actionable task commitment?",
      "criteria": {
        "true": "Concrete todo, chore, or deliverable",
        "false": "Background context, thought, or reference note"
      }
    },
    "deadline_choice": {
      "type": "choice",
      "instructions": "Which parsed date candidate is the intended task deadline in `transcript`?",
      "criteria": {
        "d1": "Friday 5pm (`parsed_date_candidates[0]`)",
        "none": "No deadline intended or strictly an informational mention"
      }
    },
    "priority_score": {
      "type": "score",
      "instructions": "Given urgency and penalties in `transcript`, what is the task priority?",
      "criteria": [
        "P3: Low / Backlog (Someday, optional)",
        "P2: Medium / Routine (Normal workflow)",
        "P1: High / Important (Explicit deadline or notable impact)",
        "P0: Urgent / Critical (Immediate emergency or severe penalty)"
      ]
    },
    "effort_duration_score": {
      "type": "score",
      "instructions": "Estimate required task duration based on effort stated in `transcript`.",
      "criteria": [
        "Quick (<15 min)",
        "Short (15–45 min)",
        "Medium (1–2 hours)",
        "Half-Day (2–4 hours)",
        "Full-Day or Multi-Day (>4 hours)"
      ]
    }
  }
}
```

### Rust Integration Strategy (`crates/judge`)
1. **Reqwest Client**: Thin HTTP client using `reqwest` and `serde`/`serde_json`.
2. **Confidence Routing**:
   - `is_a_task.noul >= 0.70`: Accept as task.
   - `is_a_task.noul < 0.35`: Drop span as non-task.
   - $0.35 \le p < 0.70$: Mark task as `NeedsConfirmation` in SQLite.
3. **Duration Mapping**: Map continuous `effort_duration_score.score` (e.g. `2.8` $\to$ Level 3: ~3 hours) directly to time minutes for the deterministic scheduler slot-filler (`crates/scheduler`).
