"""Follow-up scanning and nudge tasks (Phase 2b)."""

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)


@celery_app.task
def scan_pending_commitments() -> dict:
    """Scan all users' pending commitments and send follow-ups.

    Stub for Phase 2b -- full implementation coming.
    """
    logger.info("Follow-up scan triggered (stub)")
    return {"scanned": 0, "nudges_sent": 0}
