"""WhatsApp webhook handler for incoming messages."""

import uuid
from datetime import UTC, datetime

from fastapi import APIRouter, Request
from fastapi.responses import JSONResponse, PlainTextResponse
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.channels.base import IncomingMessage
from app.channels.router import route_message
from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.config import settings
from app.models.channel import UserChannel
from app.models.consent import UserConsent
from app.models.user import User

logger = get_logger(__name__)

router = APIRouter(tags=["whatsapp"])


@router.get("/webhook/whatsapp")
async def verify_webhook(request: Request):
    """Handle WhatsApp webhook verification challenge."""
    mode = request.query_params.get("hub.mode")
    token = request.query_params.get("hub.verify_token")
    challenge = request.query_params.get("hub.challenge")

    if mode == "subscribe" and token == settings.whatsapp_verify_token:
        return PlainTextResponse(challenge)

    return PlainTextResponse("Forbidden", status_code=403)


@router.post("/webhook/whatsapp")
async def whatsapp_webhook(request: Request):
    """Receive WhatsApp messages via webhook."""
    try:
        data = await request.json()
    except Exception:
        return JSONResponse({"ok": False}, status_code=400)

    # Process each entry
    for entry in data.get("entry", []):
        for change in entry.get("changes", []):
            value = change.get("value", {})
            messages = value.get("messages", [])

            for message in messages:
                await _handle_message(message)

    return JSONResponse({"ok": True})


async def _handle_message(message: dict) -> None:
    """Process a single incoming WhatsApp message."""
    msg_type = message.get("type")
    wa_id = message.get("from", "")

    if msg_type != "text":
        # TODO: Handle other message types
        logger.debug("Ignoring non-text WhatsApp message", extra={"type": msg_type})
        return

    text = message.get("text", {}).get("body", "")
    if not text:
        return

    async with async_session_factory() as session:
        user, _ = await _get_or_create_whatsapp_user(wa_id, session)

        # Update last message time for 24h window tracking
        channel_result = await session.execute(
            select(UserChannel).where(
                UserChannel.channel_type == "whatsapp",
                UserChannel.channel_user_id == wa_id,
            )
        )
        channel = channel_result.scalar_one_or_none()
        if channel:
            channel.last_message_at = datetime.now(UTC)
            await session.commit()

    incoming = IncomingMessage(
        user_id=str(user.id),
        channel_type="whatsapp",
        channel_user_id=wa_id,
        text=text,
    )

    await route_message(incoming, agent_handler=None)


async def _get_or_create_whatsapp_user(
    wa_id: str, session: AsyncSession
) -> tuple[User, bool]:
    """Find or create a user by WhatsApp phone number."""
    result = await session.execute(
        select(UserChannel).where(
            UserChannel.channel_type == "whatsapp",
            UserChannel.channel_user_id == wa_id,
        )
    )
    channel = result.scalar_one_or_none()

    if channel:
        user_result = await session.execute(
            select(User).where(User.id == channel.user_id)
        )
        user = user_result.scalar_one()
        return user, False

    # Create new user
    user = User(
        id=uuid.uuid4(),
        preferred_channel="whatsapp",
    )
    session.add(user)

    user_channel = UserChannel(
        user_id=user.id,
        channel_type="whatsapp",
        channel_user_id=wa_id,
    )
    session.add(user_channel)

    consent = UserConsent(user_id=user.id)
    session.add(consent)

    await session.flush()
    return user, True
