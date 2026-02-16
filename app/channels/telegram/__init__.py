"""Telegram channel adapter."""

from aiogram.enums import ChatAction

from app.channels.base import ChannelAdapter, OutgoingMessage
from app.channels.router import split_message
from app.channels.telegram.bot import get_bot


class TelegramAdapter(ChannelAdapter):
    async def send_message(self, msg: OutgoingMessage) -> None:
        bot = get_bot()
        for chunk in split_message(msg.text, max_length=4096):
            await bot.send_message(
                chat_id=int(msg.channel_user_id),
                text=chunk,
                parse_mode=msg.parse_mode,
            )

    async def send_typing_indicator(
        self, channel_user_id: str
    ) -> None:
        bot = get_bot()
        try:
            await bot.send_chat_action(
                chat_id=int(channel_user_id),
                action=ChatAction.TYPING,
            )
        except Exception:
            pass
