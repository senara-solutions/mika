"""Telegram middleware for typing indicators and DB session injection."""

from collections.abc import Awaitable, Callable
from typing import Any

from aiogram import BaseMiddleware
from aiogram.types import Message
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from app.common.logging import get_logger

logger = get_logger(__name__)


class TypingMiddleware(BaseMiddleware):
    """Send typing indicator on every incoming message."""

    async def __call__(
        self,
        handler: Callable[[Message, dict[str, Any]], Awaitable[Any]],
        event: Message,
        data: dict[str, Any],
    ) -> Any:
        if hasattr(event, "chat") and event.chat:
            try:
                await event.answer_chat_action("typing")
            except Exception:
                pass  # Non-critical, don't block
        return await handler(event, data)


class DbSessionMiddleware(BaseMiddleware):
    """Inject async DB session into handler data."""

    def __init__(self, session_factory: async_sessionmaker[AsyncSession]):
        self.session_factory = session_factory

    async def __call__(
        self,
        handler: Callable[[Message, dict[str, Any]], Awaitable[Any]],
        event: Message,
        data: dict[str, Any],
    ) -> Any:
        async with self.session_factory() as session:
            data["db"] = session
            try:
                result = await handler(event, data)
                await session.commit()
                return result
            except Exception:
                await session.rollback()
                raise
