"""Follow-up scanning and nudge tasks."""

import asyncio
from datetime import UTC, datetime

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)


def _get_approaching_commitments() -> list[dict]:
    """Query all users' pending commitments approaching due date.

    Returns list of dicts with user_id, description, due_date, etc.
    """
    from app.memory.driver import get_neo4j_driver

    async def _query():
        driver = get_neo4j_driver()
        async with driver.session() as session:
            result = await session.run(
                """
                MATCH (u:User)-[:COMMITTED_TO]->(c:Commitment)
                WHERE (c.status = 'pending' OR c.status IS NULL)
                RETURN u.user_id AS user_id,
                       c.description AS description,
                       c.due_date AS due_date,
                       c.priority AS priority
                ORDER BY c.due_date ASC
                """
            )
            return await result.data()

    return asyncio.get_event_loop().run_until_complete(_query())


def _get_user_timezone(user_id: str) -> str:
    """Get user's timezone from Postgres."""
    from sqlalchemy import select

    from app.common.db import sync_engine
    from app.models.user import User

    try:
        from sqlalchemy.orm import Session

        with Session(sync_engine()) as session:
            result = session.execute(
                select(User.timezone).where(
                    User.id == __import__("uuid").UUID(user_id)
                )
            )
            tz = result.scalar_one_or_none()
            return tz or "UTC"
    except Exception:
        return "UTC"


def _get_user_preferred_channel(user_id: str) -> str:
    """Get user's preferred channel."""
    from sqlalchemy import select

    from app.common.db import sync_engine
    from app.models.user import User

    try:
        from sqlalchemy.orm import Session

        with Session(sync_engine()) as session:
            result = session.execute(
                select(User.preferred_channel).where(
                    User.id == __import__("uuid").UUID(user_id)
                )
            )
            channel = result.scalar_one_or_none()
            return channel or "telegram"
    except Exception:
        return "telegram"


def _send_nudge(
    user_id: str, channel_type: str, message: str
) -> bool:
    """Send a nudge message to a user via their preferred channel."""
    from app.channels.base import OutgoingMessage
    from app.channels.router import get_adapter, split_message

    async def _send():
        try:
            adapter = get_adapter(channel_type)
            for chunk in split_message(message):
                outgoing = OutgoingMessage(
                    user_id=user_id,
                    channel_type=channel_type,
                    channel_user_id="",  # Will be resolved by adapter
                    text=chunk,
                )
                await adapter.send_message(outgoing)
            return True
        except Exception:
            logger.exception(
                "Failed to send nudge",
                extra={"user_id": user_id},
            )
            return False

    return asyncio.get_event_loop().run_until_complete(_send())


def _compose_nudge(
    description: str, due_date: str | None
) -> str:
    """Compose a friendly nudge message for a commitment."""
    now = datetime.now(UTC)

    if due_date:
        try:
            due = datetime.fromisoformat(due_date)
            if due.tzinfo is None:
                due = due.replace(tzinfo=UTC)
            days_until = (due.date() - now.date()).days

            if days_until < 0:
                return (
                    f"Hey! Just a heads up -- this is overdue: "
                    f"**{description}** (was due {abs(days_until)} "
                    f"day{'s' if abs(days_until) != 1 else ''} ago). "
                    f"Want me to help you knock it out?"
                )
            elif days_until == 0:
                return (
                    f"Reminder: **{description}** is due today! "
                    f"Need any help with it?"
                )
            elif days_until <= 2:
                return (
                    f"Quick heads up -- **{description}** is coming "
                    f"up in {days_until} "
                    f"day{'s' if days_until != 1 else ''}. "
                    f"Anything I can help with?"
                )
        except (ValueError, TypeError):
            pass

    return (
        f"Just checking in on: **{description}**. "
        f"How's that going? Let me know if I can help."
    )


@celery_app.task
def scan_pending_commitments() -> dict:
    """Scan all users' pending commitments and send follow-ups.

    Runs every 4 hours via Celery Beat. Checks each commitment:
    - Is it overdue or approaching due date?
    - Is the user in active hours?
    - Has rate limit been hit?
    """
    from app.common.rate_limiter import can_send_proactive, record_proactive_send
    from app.common.timezone import is_active_hours

    logger.info("Follow-up scan started")

    commitments = _get_approaching_commitments()
    scanned = len(commitments)
    nudges_sent = 0

    # Group by user to respect rate limits
    user_commitments: dict[str, list[dict]] = {}
    for c in commitments:
        uid = c.get("user_id", "")
        if uid:
            user_commitments.setdefault(uid, []).append(c)

    for user_id, user_comms in user_commitments.items():
        # Check timezone
        tz = _get_user_timezone(user_id)
        if not is_active_hours(tz):
            continue

        # Check rate limit
        can_send = asyncio.get_event_loop().run_until_complete(
            can_send_proactive(user_id)
        )
        if not can_send:
            continue

        # Pick the most urgent commitment
        commitment = user_comms[0]  # Already sorted by due_date ASC
        description = commitment.get("description", "")
        due_date = commitment.get("due_date")

        nudge = _compose_nudge(description, due_date)
        channel = _get_user_preferred_channel(user_id)

        if _send_nudge(user_id, channel, nudge):
            asyncio.get_event_loop().run_until_complete(
                record_proactive_send(user_id)
            )
            nudges_sent += 1

    logger.info(
        "Follow-up scan completed",
        extra={"scanned": scanned, "nudges_sent": nudges_sent},
    )
    return {"scanned": scanned, "nudges_sent": nudges_sent}
