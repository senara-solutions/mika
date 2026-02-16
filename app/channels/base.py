"""Channel adapter interface for multi-platform messaging."""

from abc import ABC, abstractmethod
from dataclasses import dataclass, field


@dataclass
class IncomingMessage:
    user_id: str  # Internal Mika user_id (UUID)
    channel_type: str  # "telegram" | "whatsapp"
    channel_user_id: str  # Platform-specific user ID
    text: str
    message_type: str = "text"  # "text" | "photo" | "voice" | etc.
    raw_data: dict = field(default_factory=dict)


@dataclass
class OutgoingMessage:
    user_id: str
    channel_type: str
    channel_user_id: str
    text: str
    parse_mode: str = "Markdown"


class ChannelAdapter(ABC):
    @abstractmethod
    async def send_message(self, msg: OutgoingMessage) -> None: ...

    @abstractmethod
    async def send_typing_indicator(
        self, channel_user_id: str
    ) -> None: ...
