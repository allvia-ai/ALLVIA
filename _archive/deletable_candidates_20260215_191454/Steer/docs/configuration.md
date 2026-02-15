# Configuration Reference

This document lists all environment variables used by the AI-Summary / InfoPilot system.

## Core Runtime
| Variable | Default | Description |
|----------|---------|-------------|
| `INFOPILOT_LOG_LEVEL` | INFO | Logging verbosity (DEBUG, INFO, WARNING, ERROR). |
| `INFOPILOT_RUNTIME_LOG_DIR` | (derived) | Directory for runtime logs. |
| `INFOPILOT_DATA_DIR` | (derived) | Base directory for data storage (smart folders, indices). |
| `INFOPILOT_CACHE_MAX_ENTRIES` | 1000 | Max entries in chunk cache before eviction. |

## Chat Engine (LNP Chat)
| Variable | Default | Description |
|----------|---------|-------------|
| `LNPCHAT_RERANK_MODEL` | BAAI/bge-reranker-large | Model ID for Cross-Encoder reranking. |
| `LNPCHAT_LLM_BACKEND` | | Backend to use (ollama, llamacpp, openai). |
| `LNPCHAT_LLM_MODEL` | llama3 | Model name to request from backend. |
| `LNPCHAT_LLM_HOST` | | Host URL for LLM backend. |
| `LNPCHAT_LLM_TIMEOUT` | 30.0 | Timeout for generation requests in seconds. |
| `LNPCHAT_OLLAMA_NUM_PREDICT` | | Context size/predict length for Ollama. |

## API Server (FastAPI)
| Variable | Default | Description |
|----------|---------|-------------|
| `APP_TESTING` | 0 | Enable testing mode (in-memory DBs). |
| `APP_STARTUP_LOAD` | 1 | Load resources on startup (0 to disable). |
| `LLM_PROVIDER` | none | Provider for API LLM (openai, etc). |
| `LLM_API_KEY` | | API Key for provider. |
| `LLM_BASE_URL` | | Base URL for provider. |
| `LLM_MODEL` | | Model name for API. |

## Agents
### Document Agent
| Variable | Default | Description |
|----------|---------|-------------|
| `DOCUMENT_LLM_MODEL` | | Model for document summarization. |
| `DOCUMENT_SUPERVISOR_MODE` | manual | Supervisor mode (manual, auto). |

### Meeting Agent
| Variable | Default | Description |
|----------|---------|-------------|
| `MEETING_STT_MODEL` | small | Whisper model size (tiny, base, small, medium, large). |
| `MEETING_STT_DEVICE` | | Device for inference (cpu, cuda, mps). |
| `MEETING_STT_COMPUTE` | int8 | Compute precision (int8, float16). |

### Supervisor & Common
| Variable | Default | Description |
|----------|---------|-------------|
| `SUMMARY_SUPERVISOR_BACKEND` | | distinct backend for supervisor (if different). |
| `SUMMARY_SUPERVISOR_MODEL` | | distinct model for supervisor. |
