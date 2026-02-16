"""Maintenance tasks: conversation summarization."""

import asyncio
import uuid
from datetime import UTC, datetime, timedelta

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)

SUMMARIZATION_PROMPT = """Summarize the following conversation between a user and \
their AI assistant Mika. Extract the key points in a concise format.

Focus on:
- Decisions made
- Action items or commitments
- Important facts or preferences mentioned
- Key topics discussed

Conversation:
{conversation_text}

Write a concise summary (2-4 sentences). Return ONLY the summary text."""

CONVERSATION_AGE_DAYS = 7


def _get_users_with_old_conversations() -> list[str]:
    """Find users with conversations older than threshold."""
    from sqlalchemy import select
    from sqlalchemy.orm import Session

    from app.common.db import sync_engine
    from app.models.conversation import Conversation

    cutoff = datetime.now(UTC) - timedelta(days=CONVERSATION_AGE_DAYS)

    try:
        with Session(sync_engine()) as session:
            result = session.execute(
                select(Conversation.user_id)
                .where(Conversation.created_at < cutoff)
                .distinct()
            )
            return [str(row[0]) for row in result.all()]
    except Exception:
        logger.exception("Failed to query users for summarization")
        return []


def _get_old_conversations(user_id: str) -> list[dict]:
    """Get conversations older than threshold for a user."""
    from sqlalchemy import select
    from sqlalchemy.orm import Session

    from app.common.db import sync_engine
    from app.models.conversation import Conversation

    cutoff = datetime.now(UTC) - timedelta(days=CONVERSATION_AGE_DAYS)

    try:
        with Session(sync_engine()) as session:
            result = session.execute(
                select(
                    Conversation.role,
                    Conversation.content,
                    Conversation.created_at,
                )
                .where(
                    Conversation.user_id == uuid.UUID(user_id),
                    Conversation.created_at < cutoff,
                )
                .order_by(Conversation.created_at.asc())
                .limit(100)
            )
            return [
                {"role": row.role, "content": row.content}
                for row in result.all()
            ]
    except Exception:
        logger.exception("Failed to get old conversations")
        return []


def _format_conversations(conversations: list[dict]) -> str:
    """Format conversations for the summarization prompt."""
    lines = []
    for conv in conversations:
        role = "User" if conv["role"] == "user" else "Mika"
        lines.append(f"{role}: {conv['content']}")
    return "\n".join(lines)


def _summarize_text(text: str) -> str:
    """Summarize conversation text using Claude Sonnet."""
    from app.common.llm import get_sonnet

    async def _invoke():
        llm = get_sonnet()
        prompt = SUMMARIZATION_PROMPT.format(conversation_text=text)
        response = await llm.ainvoke(prompt)
        return response.content.strip()

    try:
        return asyncio.get_event_loop().run_until_complete(_invoke())
    except Exception:
        logger.exception("Failed to summarize conversations")
        return ""


def _store_summary(user_id: str, summary: str, week_key: str) -> None:
    """Store summary as a Fact node in Neo4j."""
    from app.memory.repository import ensure_user_node, write_entities

    async def _write():
        await ensure_user_node(user_id)
        await write_entities(user_id, [{
            "label": "Fact",
            "properties": {
                "key": week_key,
                "value": summary,
                "source": "summarization",
            },
        }])

    asyncio.get_event_loop().run_until_complete(_write())


@celery_app.task
def summarize_old_conversations() -> dict:
    """Summarize conversations older than 7 days into Neo4j Fact nodes.

    Runs weekly via Celery Beat (Sunday 3 AM UTC).
    For each user with old conversations:
    - Load conversation text
    - Summarize via Claude Sonnet
    - Store as Fact node with key weekly_summary_YYYY_WW
    """
    logger.info("Conversation summarization started")

    users = _get_users_with_old_conversations()
    users_processed = 0
    summaries_created = 0

    now = datetime.now(UTC)
    week_key = f"weekly_summary_{now.strftime('%Y_%W')}"

    for user_id in users:
        conversations = _get_old_conversations(user_id)
        if not conversations:
            continue

        text = _format_conversations(conversations)
        if len(text) < 50:
            continue

        summary = _summarize_text(text)
        if not summary:
            continue

        try:
            _store_summary(user_id, summary, week_key)
            summaries_created += 1
        except Exception:
            logger.exception(
                "Failed to store summary",
                extra={"user_id": user_id},
            )

        users_processed += 1

    logger.info(
        "Conversation summarization completed",
        extra={
            "users_processed": users_processed,
            "summaries_created": summaries_created,
        },
    )
    return {
        "users_processed": users_processed,
        "summaries_created": summaries_created,
    }
