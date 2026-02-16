"""Maintenance tasks: summarization, backups (Phase 2d)."""

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)


@celery_app.task
def summarize_old_conversations() -> dict:
    """Summarize conversations older than 7 days into Neo4j Fact nodes.

    Stub for Phase 2d -- full implementation coming.
    """
    logger.info("Conversation summarization triggered (stub)")
    return {"users_processed": 0, "summaries_created": 0}
