"""LangGraph agent -- stub for Phase 1a, full implementation in Phase 1b."""

from app.common.logging import get_logger

logger = get_logger(__name__)


async def invoke_agent(
    user_id: str,
    message_text: str,
    channel_type: str,
) -> str:
    """Invoke the Mika agent. Returns response text.

    This is a stub that will be replaced with the full LangGraph
    implementation in Phase 1b.
    """
    logger.info(
        "Agent invoked (stub)",
        extra={"user_id": user_id, "channel_type": channel_type},
    )
    return (
        "I'm Mika, your AI executive assistant. "
        "I'm still setting up, but I'll be fully operational soon!"
    )
