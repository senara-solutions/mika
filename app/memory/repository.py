"""Neo4j memory repository -- stub for Phase 1b, full implementation in Phase 1c."""

from app.common.logging import get_logger

logger = get_logger(__name__)


async def get_user_context(user_id: str, limit: int = 30) -> str:
    """Retrieve formatted memory context for a user from Neo4j.

    Stub implementation -- returns empty string until Phase 1c.
    """
    return ""
