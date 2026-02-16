"""Aiogram bot and dispatcher setup."""

from aiogram import Bot, Dispatcher
from aiogram.client.default import DefaultBotProperties
from aiogram.enums import ParseMode

from app.config import settings

_bot: Bot | None = None
dp = Dispatcher()

# Fake token format that passes aiogram validation (for tests/startup without env)
_PLACEHOLDER_TOKEN = "0000000000:AAFakeTokenForTestsOnly_placeholder"


def get_bot() -> Bot:
    global _bot
    if _bot is None:
        token = settings.telegram_bot_token or _PLACEHOLDER_TOKEN
        _bot = Bot(
            token=token,
            default=DefaultBotProperties(parse_mode=ParseMode.MARKDOWN),
        )
    return _bot


def get_dispatcher() -> Dispatcher:
    return dp
