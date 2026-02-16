"""Tests for Telegram handlers."""

from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from aiogram.types import Chat, Message, User


def make_message(
    text: str = "hello",
    chat_type: str = "private",
    user_id: int = 12345,
) -> MagicMock:
    """Create a mock Telegram message."""
    msg = MagicMock(spec=Message)
    msg.text = text
    msg.answer = AsyncMock()
    msg.answer_chat_action = AsyncMock()

    chat = MagicMock(spec=Chat)
    chat.type = chat_type
    chat.id = user_id
    msg.chat = chat

    user = MagicMock(spec=User)
    user.id = user_id
    user.full_name = "Test User"
    msg.from_user = user

    msg.content_type = "text"

    return msg


@pytest.fixture
def mock_db():
    """Mock async database session."""
    db = AsyncMock()
    result = MagicMock()
    result.scalar_one_or_none = MagicMock(return_value=None)
    db.execute = AsyncMock(return_value=result)
    db.flush = AsyncMock()
    db.add = MagicMock()
    return db


class TestStartCommand:
    @pytest.mark.asyncio
    async def test_start_private_chat(self, mock_db):
        from app.channels.telegram.handlers import cmd_start

        msg = make_message(text="/start")
        await cmd_start(msg, mock_db)
        msg.answer.assert_called_once()
        call_text = msg.answer.call_args[0][0]
        assert "Mika" in call_text
        assert "Sounds good" in call_text

    @pytest.mark.asyncio
    async def test_start_group_chat(self, mock_db):
        from app.channels.telegram.handlers import cmd_start

        msg = make_message(text="/start", chat_type="group")
        await cmd_start(msg, mock_db)
        call_text = msg.answer.call_args[0][0]
        assert "private" in call_text.lower() or "directly" in call_text.lower()


class TestHelpCommand:
    @pytest.mark.asyncio
    async def test_help_private(self):
        from app.channels.telegram.handlers import cmd_help

        msg = make_message(text="/help")
        await cmd_help(msg)
        msg.answer.assert_called_once()
        call_text = msg.answer.call_args[0][0]
        assert "/start" in call_text


class TestGroupMessages:
    @pytest.mark.asyncio
    async def test_group_message_blocked(self):
        from app.channels.telegram.handlers import (
            handle_group_message,
        )

        msg = make_message(chat_type="group")
        await handle_group_message(msg)
        msg.answer.assert_called_once()
        call_text = msg.answer.call_args[0][0]
        assert "private" in call_text.lower() or "directly" in call_text.lower()


class TestUnsupportedContent:
    @pytest.mark.asyncio
    async def test_photo_rejected(self):
        from app.channels.telegram.handlers import (
            handle_unsupported_content,
        )

        msg = make_message()
        msg.content_type = "photo"
        await handle_unsupported_content(msg)
        msg.answer.assert_called_once()
        assert "text" in msg.answer.call_args[0][0].lower()


class TestTextMessage:
    @pytest.mark.asyncio
    async def test_text_routes_to_agent(self, mock_db):
        from app.channels.telegram.handlers import handle_text_message

        msg = make_message(text="Draft an email to Sarah")

        with patch("app.channels.telegram.handlers.route_message") as mock_route:
            mock_route.return_value = None
            await handle_text_message(msg, mock_db)
            mock_route.assert_called_once()
            incoming = mock_route.call_args[0][0]
            assert incoming.text == "Draft an email to Sarah"
            assert incoming.channel_type == "telegram"
