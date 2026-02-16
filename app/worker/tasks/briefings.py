"""Morning briefing dispatch and composition."""

import asyncio
import uuid

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)

BRIEFING_WINDOW_START_HOUR = 7
BRIEFING_WINDOW_START_MINUTE = 0
BRIEFING_WINDOW_END_HOUR = 7
BRIEFING_WINDOW_END_MINUTE = 15

BRIEFING_PROMPT = """You are Mika, a warm AI executive assistant.
Compose a concise morning briefing for your user.

Here is the user's context from their memory graph:

{memory_context}

{calendar_section}

Write a briefing that:
- Greets them warmly
- Summarizes today's calendar if available
- Lists pending commitments (with due dates if known)
- Mentions any recent topics they were interested in
- Offers to help with one specific thing (e.g., meeting prep, follow-up)
- Is under 200 words

Return ONLY the briefing text, no JSON or markdown formatting."""


def _get_briefing_eligible_users() -> list[dict]:
    """Find users in the morning briefing window.

    Returns list of dicts with user_id, timezone, preferred_channel.
    """
    from sqlalchemy import select
    from sqlalchemy.orm import Session

    from app.common.db import sync_engine
    from app.common.timezone import is_in_time_window
    from app.models.user import User

    try:
        with Session(sync_engine()) as session:
            result = session.execute(
                select(
                    User.id, User.timezone, User.preferred_channel,
                    User.onboarding_completed,
                )
                .where(User.onboarding_completed.is_(True))
            )
            rows = result.all()

        eligible = []
        for row in rows:
            tz = row.timezone or "UTC"
            if is_in_time_window(
                tz,
                BRIEFING_WINDOW_START_HOUR,
                BRIEFING_WINDOW_START_MINUTE,
                BRIEFING_WINDOW_END_HOUR,
                BRIEFING_WINDOW_END_MINUTE,
            ):
                eligible.append({
                    "user_id": str(row.id),
                    "timezone": tz,
                    "preferred_channel": row.preferred_channel or "telegram",
                })
        return eligible
    except Exception:
        logger.exception("Failed to get briefing eligible users")
        return []


def _get_memory_context(user_id: str) -> str:
    """Get user's memory context from Neo4j."""
    from app.memory.repository import get_user_context

    async def _fetch():
        return await get_user_context(user_id, limit=20)

    return asyncio.get_event_loop().run_until_complete(_fetch())


def _get_calendar_context(user_id: str) -> str:
    """Get today's calendar events for the user."""

    async def _fetch():
        from app.integrations.google_calendar import get_user_calendar_data

        return await get_user_calendar_data(user_id)

    try:
        result = asyncio.get_event_loop().run_until_complete(_fetch())
        return result or ""
    except Exception:
        return ""


def _compose_briefing(memory_context: str, calendar_context: str = "") -> str:
    """Compose briefing using Claude Sonnet."""
    from app.common.llm import get_sonnet

    calendar_section = ""
    if calendar_context:
        calendar_section = f"Today's calendar:\n\n{calendar_context}"

    async def _invoke():
        llm = get_sonnet()
        prompt = BRIEFING_PROMPT.format(
            memory_context=memory_context,
            calendar_section=calendar_section,
        )
        response = await llm.ainvoke(prompt)
        return response.content.strip()

    try:
        return asyncio.get_event_loop().run_until_complete(_invoke())
    except Exception:
        logger.exception("Failed to compose briefing")
        return ""


def _send_briefing(
    user_id: str, channel_type: str, text: str
) -> bool:
    """Send briefing message to user."""
    from app.channels.base import OutgoingMessage
    from app.channels.router import get_adapter, split_message

    async def _send():
        adapter = get_adapter(channel_type)
        for chunk in split_message(text):
            outgoing = OutgoingMessage(
                user_id=user_id,
                channel_type=channel_type,
                channel_user_id="",
                text=chunk,
            )
            await adapter.send_message(outgoing)
        return True

    try:
        return asyncio.get_event_loop().run_until_complete(_send())
    except Exception:
        logger.exception(
            "Failed to send briefing",
            extra={"user_id": user_id},
        )
        return False


def _save_briefing_conversation(
    user_id: str, channel_type: str, briefing_text: str
) -> None:
    """Save the briefing as a conversation turn in Postgres."""
    from datetime import UTC, datetime

    from sqlalchemy.orm import Session

    from app.common.db import sync_engine
    from app.models.conversation import Conversation

    try:
        with Session(sync_engine()) as session:
            session.add(
                Conversation(
                    user_id=uuid.UUID(user_id),
                    channel_type=channel_type,
                    role="assistant",
                    content=briefing_text,
                    created_at=datetime.now(UTC),
                )
            )
            session.commit()
    except Exception:
        logger.debug("Could not save briefing conversation")


@celery_app.task
def morning_briefing_dispatcher() -> dict:
    """Dispatch morning briefings to eligible users.

    Runs every 15 minutes via Celery Beat. For each user whose local
    time falls in the briefing window (7:00-7:15 AM by default):
    - Query their memory context from Neo4j
    - Compose a briefing via Claude Sonnet
    - Send via their preferred channel
    """
    from app.common.rate_limiter import can_send_proactive, record_proactive_send

    logger.info("Morning briefing dispatcher started")

    eligible = _get_briefing_eligible_users()
    briefings_sent = 0

    for user_info in eligible:
        user_id = user_info["user_id"]
        channel = user_info["preferred_channel"]

        # Check rate limit
        can_send = asyncio.get_event_loop().run_until_complete(
            can_send_proactive(user_id)
        )
        if not can_send:
            continue

        # Get memory context
        memory_context = _get_memory_context(user_id)
        if not memory_context:
            continue

        # Get calendar context (optional)
        calendar_context = _get_calendar_context(user_id)

        # Compose briefing
        briefing = _compose_briefing(memory_context, calendar_context)
        if not briefing:
            continue

        # Send it
        if _send_briefing(user_id, channel, briefing):
            asyncio.get_event_loop().run_until_complete(
                record_proactive_send(user_id)
            )
            _save_briefing_conversation(user_id, channel, briefing)
            briefings_sent += 1

    logger.info(
        "Morning briefing dispatcher completed",
        extra={"briefings_sent": briefings_sent},
    )
    return {"briefings_sent": briefings_sent}
