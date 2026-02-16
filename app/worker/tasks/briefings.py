"""Morning briefing tasks (Phase 2c)."""

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)


@celery_app.task
def morning_briefing_dispatcher() -> dict:
    """Dispatch morning briefings to users in their local morning window.

    Stub for Phase 2c -- full implementation coming.
    """
    logger.info("Morning briefing dispatcher triggered (stub)")
    return {"briefings_sent": 0}
