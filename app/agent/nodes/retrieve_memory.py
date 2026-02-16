"""Retrieve user memory context from Neo4j."""

from app.agent.state import MikaState
from app.common.logging import get_logger

logger = get_logger(__name__)


async def retrieve_memory(state: MikaState) -> dict:
    """Query Neo4j for user's memory context."""
    user_id = state["user_id"]

    try:
        from app.memory.repository import get_user_context

        memory_context = await get_user_context(user_id)
    except Exception:
        logger.warning(
            "Neo4j unreachable, proceeding without memory",
            extra={"user_id": user_id},
        )
        memory_context = ""

    return {"memory_context": memory_context}
