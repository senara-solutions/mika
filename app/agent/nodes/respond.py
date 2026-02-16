"""Extract final response from agent state."""

from app.agent.state import MikaState
from app.common.logging import get_logger

logger = get_logger(__name__)


async def respond(state: MikaState) -> dict:
    """Extract the final text response from the last message."""
    messages = state.get("messages", [])
    if not messages:
        return {"error": "No messages to respond with"}

    last_message = messages[-1]
    content = getattr(last_message, "content", "")

    if not content:
        content = "I'm not sure how to respond to that. Could you rephrase?"

    return {"messages": []}  # No new messages to add, response already in state
