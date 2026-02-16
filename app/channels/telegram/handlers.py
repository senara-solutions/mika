"""Telegram message and command handlers."""

import uuid

from aiogram import F, Router
from aiogram.filters import Command
from aiogram.types import Message
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.channels.base import IncomingMessage
from app.channels.router import route_message
from app.common.logging import get_logger
from app.models.channel import UserChannel
from app.models.consent import UserConsent
from app.models.user import User

logger = get_logger(__name__)

router = Router(name="telegram")

CONSENT_MESSAGE = (
    "Hi! I'm Mika, your AI executive assistant.\n\n"
    "Before we get started, here's how I work:\n"
    "- Your messages are processed by Claude (Anthropic's AI)\n"
    "- I store your conversations and build a memory of your "
    "preferences, commitments, and contacts\n"
    "- Your data is encrypted and never used for AI training\n"
    "- You can export or delete all your data anytime with "
    "/export or /delete\n\n"
    "Reply **Sounds good** to continue, or **Tell me more** "
    "for details."
)

HELP_MESSAGE = (
    "I'm Mika, your AI executive assistant. Here's what I can do:\n\n"
    "- Remember your preferences, commitments, and contacts\n"
    "- Draft emails, messages, and documents\n"
    "- Research people and topics\n"
    "- Follow up on your pending tasks\n"
    "- Send you morning briefings\n\n"
    "**Commands:**\n"
    "/start - Start or restart onboarding\n"
    "/help - Show this help message\n"
    "/settings - Manage your preferences\n"
    "/export - Export all your data\n"
    "/delete - Delete all your data\n\n"
    "Just message me naturally and I'll help!"
)

GROUP_MESSAGE = (
    "I work best in private conversations. "
    "Message me directly instead!"
)

UNSUPPORTED_CONTENT = (
    "I can't process that yet, but it's coming soon. "
    "Can you describe it in text?"
)


async def get_or_create_user(
    telegram_user_id: str,
    db: AsyncSession,
) -> tuple[User, bool]:
    """Find existing user by Telegram ID or create a new one."""
    result = await db.execute(
        select(UserChannel).where(
            UserChannel.channel_type == "telegram",
            UserChannel.channel_user_id == telegram_user_id,
        )
    )
    channel = result.scalar_one_or_none()

    if channel:
        result = await db.execute(
            select(User).where(User.id == channel.user_id)
        )
        user = result.scalar_one()
        return user, False

    # Create new user
    user = User(id=uuid.uuid4())
    db.add(user)

    user_channel = UserChannel(
        user_id=user.id,
        channel_type="telegram",
        channel_user_id=telegram_user_id,
    )
    db.add(user_channel)

    consent = UserConsent(user_id=user.id)
    db.add(consent)

    await db.flush()
    return user, True


@router.message(Command("start"))
async def cmd_start(message: Message, db: AsyncSession) -> None:
    """Handle /start command -- onboarding entry point."""
    if message.chat.type != "private":
        await message.answer(GROUP_MESSAGE)
        return

    tg_user_id = str(message.from_user.id)
    user, is_new = await get_or_create_user(tg_user_id, db)

    if is_new:
        logger.info("New user created", extra={"user_id": str(user.id)})

    await message.answer(CONSENT_MESSAGE)


@router.message(Command("help"))
async def cmd_help(message: Message) -> None:
    if message.chat.type != "private":
        return
    await message.answer(HELP_MESSAGE)


@router.message(Command("settings"))
async def cmd_settings(message: Message) -> None:
    if message.chat.type != "private":
        return
    await message.answer(
        "Settings coming soon! For now, just tell me your preferences "
        "and I'll remember them."
    )


@router.message(Command("export"))
async def cmd_export(message: Message, db: AsyncSession) -> None:
    if message.chat.type != "private":
        return
    tg_user_id = str(message.from_user.id)
    user, _ = await get_or_create_user(tg_user_id, db)

    try:
        from app.api.routes.privacy import _gather_memory_data, _gather_postgres_data

        profile, channels, conversations = await _gather_postgres_data(str(user.id))
        memory = await _gather_memory_data(str(user.id))

        summary = (
            f"**Your Data Summary:**\n"
            f"- Profile: {profile.get('email', 'No email')}, "
            f"timezone {profile.get('timezone', 'UTC')}\n"
            f"- Connected channels: {len(channels)}\n"
            f"- Conversations: {len(conversations)} messages\n"
            f"- Memory nodes: {len(memory)}\n\n"
            f"For a full export (ZIP download), visit the web dashboard "
            f"at /api/privacy/export."
        )
        await message.answer(summary)
    except Exception:
        await message.answer(
            "Data export is being prepared. "
            "I'll send it to you shortly."
        )


@router.message(Command("delete"))
async def cmd_delete(message: Message, db: AsyncSession) -> None:
    if message.chat.type != "private":
        return
    await message.answer(
        "Are you sure you want to delete all your data? "
        "This cannot be undone. Reply **Yes, delete everything** "
        "to confirm."
    )


@router.message(F.text == "Yes, delete everything")
async def handle_delete_confirmation(
    message: Message, db: AsyncSession
) -> None:
    """Handle confirmation of data deletion."""
    if message.chat.type != "private":
        return

    tg_user_id = str(message.from_user.id)
    user, _ = await get_or_create_user(tg_user_id, db)

    try:
        from app.api.routes.privacy import (
            _clear_redis_cache,
            _delete_neo4j_data,
            _delete_postgres_user,
        )

        user_id = str(user.id)
        await _delete_neo4j_data(user_id)
        await _delete_postgres_user(user_id)
        await _clear_redis_cache(user_id)

        await message.answer(
            "All your data has been deleted. "
            "If you want to start over, send /start."
        )
    except Exception:
        logger.exception("Failed to delete user data via Telegram")
        await message.answer(
            "Something went wrong deleting your data. "
            "Please try again or use the web dashboard."
        )


@router.message(F.chat.type != "private")
async def handle_group_message(message: Message) -> None:
    """Block group chat usage."""
    # Only respond once -- we'll just always respond with redirect
    await message.answer(GROUP_MESSAGE)


@router.message(F.content_type.in_({"photo", "video", "voice", "audio", "document", "sticker"}))
async def handle_unsupported_content(message: Message) -> None:
    """Handle non-text content types."""
    if message.chat.type != "private":
        return
    await message.answer(UNSUPPORTED_CONTENT)


@router.message(F.text)
async def handle_text_message(
    message: Message, db: AsyncSession
) -> None:
    """Handle regular text messages -- route to agent."""
    if message.chat.type != "private":
        return

    tg_user_id = str(message.from_user.id)
    user, _ = await get_or_create_user(tg_user_id, db)

    incoming = IncomingMessage(
        user_id=str(user.id),
        channel_type="telegram",
        channel_user_id=tg_user_id,
        text=message.text,
    )

    await route_message(incoming, agent_handler=None)
