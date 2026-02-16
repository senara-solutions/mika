"""Channel-agnostic message router. Routes incoming messages to the agent."""

from app.channels.base import ChannelAdapter, IncomingMessage, OutgoingMessage
from app.common.logging import get_logger

logger = get_logger(__name__)

# Registry of channel adapters
_adapters: dict[str, ChannelAdapter] = {}


def register_adapter(channel_type: str, adapter: ChannelAdapter) -> None:
    _adapters[channel_type] = adapter


def get_adapter(channel_type: str) -> ChannelAdapter:
    adapter = _adapters.get(channel_type)
    if adapter is None:
        raise ValueError(f"No adapter registered for channel: {channel_type}")
    return adapter


async def route_message(
    message: IncomingMessage,
    agent_handler: object,
) -> None:
    """Route an incoming message through the agent and send the response."""
    from app.agent.graph import invoke_agent

    adapter = get_adapter(message.channel_type)

    # Send typing indicator
    await adapter.send_typing_indicator(message.channel_user_id)

    try:
        response_text = await invoke_agent(
            user_id=message.user_id,
            message_text=message.text,
            channel_type=message.channel_type,
        )
    except Exception:
        logger.exception("Agent invocation failed", extra={"user_id": message.user_id})
        response_text = (
            "I'm having trouble right now. I'll follow up shortly."
        )

    # Split and send response
    for chunk in split_message(response_text):
        outgoing = OutgoingMessage(
            user_id=message.user_id,
            channel_type=message.channel_type,
            channel_user_id=message.channel_user_id,
            text=chunk,
        )
        await adapter.send_message(outgoing)


def split_message(
    text: str, max_length: int = 4096
) -> list[str]:
    """Split long messages at paragraph boundaries."""
    if len(text) <= max_length:
        return [text]

    chunks: list[str] = []
    remaining = text

    while remaining:
        if len(remaining) <= max_length:
            chunks.append(remaining)
            break

        # Find last paragraph break within limit
        split_pos = remaining[:max_length].rfind("\n\n")
        if split_pos == -1:
            # Try single newline
            split_pos = remaining[:max_length].rfind("\n")
        if split_pos == -1:
            # Try space
            split_pos = remaining[:max_length].rfind(" ")
        if split_pos == -1:
            # Hard cut
            split_pos = max_length

        chunks.append(remaining[:split_pos].rstrip())
        remaining = remaining[split_pos:].lstrip()

    return chunks
