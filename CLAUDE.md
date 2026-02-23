# Mika - AI Executive Assistant

## Project Overview

Mika is a conversation-first AI executive assistant that lives in Telegram (then WhatsApp), with persistent memory via a Neo4j knowledge graph, proactive follow-ups via Celery, and a web dashboard for settings and memory management.

## Stack

- **Language:** Python 3.12+
- **Agent framework:** LangGraph + LangChain
- **LLM:** Claude (Sonnet 4.5 default, Opus 4.6 for complex tasks)
- **Knowledge graph:** Neo4j 5 Community
- **Relational DB:** PostgreSQL 16 + SQLAlchemy 2 (async) + Alembic
- **Task queue:** Celery + Redis
- **Telegram:** aiogram 3.x
- **WhatsApp:** PyWa
- **Web API:** FastAPI
- **Package manager:** uv

## Directory Structure

- `app/` - Main application package
  - `agent/` - LangGraph agent (graph, state, prompts, nodes)
  - `tools/` - LangChain tool definitions
  - `memory/` - Neo4j memory layer (repository, extractor, schema, driver)
  - `worker/` - Celery tasks and beat schedule
  - `channels/` - Message router + channel adapters (telegram, whatsapp)
  - `api/` - FastAPI app + routes (webhooks, auth, dashboard, privacy)
  - `models/` - SQLAlchemy models
  - `common/` - Shared utilities (LLM factory, encryption, logging)
- `alembic/` - Database migrations
- `tests/` - Test suite (mirrors app/ structure)
- `scripts/` - Utility scripts

## Conventions

- **Async by default:** All DB, Neo4j, and HTTP operations are async
- **Type hints:** Required on all function signatures
- **Naming:** snake_case for functions/variables, PascalCase for classes
- **Imports:** Use absolute imports (`from app.config import settings`)
- **Testing:** pytest + pytest-asyncio. Tests mirror app/ structure.
- **Linting:** ruff (select: E, F, I, N, W, UP)
- **Models:** SQLAlchemy 2.0 mapped_column style
- **Settings:** pydantic-settings via `app.config.settings` singleton

## Commands

- `uv run pytest` - Run tests
- `uv run ruff check .` - Lint
- `uv run ruff format .` - Format
- `uv run alembic upgrade head` - Run migrations
- `uv run uvicorn app.api.main:app --reload` - Dev server
- `uv run celery -A app.worker.celery_app worker -l info` - Celery worker
- `uv run celery -A app.worker.celery_app beat -l info` - Celery beat
- `docker compose up -d` - Start local infrastructure (Neo4j, Redis, Postgres)

## Neo4j Conventions

- All memory nodes have a `user_id` property for data isolation
- Use MERGE (not CREATE) for idempotent writes
- All queries scoped by user_id (defense in depth)
- Node labels: User, Person, Commitment, Topic, Pattern, Preference, Fact

## Reference Repositories

Local clones of agent platforms to study for patterns and inspiration. Read freely when designing Mika features.

- **OpenClaw** — `/home/samidarko/workspace/senara-solutions/openclaw/`
  TypeScript monorepo. Study for: channel adapter architecture (hub-and-spoke gateway), skills system (Markdown/YAML definitions), multi-channel UX patterns, community marketplace model.

- **LettaBot** — `/home/samidarko/workspace/senara-solutions/lettabot/`
  TypeScript. Study for: memory hierarchy patterns (core/archival/recall from MemGPT), autonomous memory self-editing via tool calls, agent state persistence, channel integrations built on top of Letta's memory API.
