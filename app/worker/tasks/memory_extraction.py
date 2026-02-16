"""Celery task for async memory extraction."""

import asyncio

from app.common.logging import get_logger
from app.worker.celery_app import celery_app

logger = get_logger(__name__)


@celery_app.task(
    bind=True,
    max_retries=3,
    default_retry_delay=30,
    acks_late=True,
    reject_on_worker_lost=True,
)
def extract_and_store_memory(
    self, user_id: str, conversation_text: str
) -> dict:
    """Extract entities from conversation and store in Neo4j.

    Runs asynchronously as a Celery task with retry policy.
    Retries: 30s, 120s, 480s (exponential backoff).
    """
    try:
        loop = asyncio.new_event_loop()
        result = loop.run_until_complete(
            _extract_and_store(user_id, conversation_text)
        )
        loop.close()
        return result
    except Exception as exc:
        logger.exception(
            "Memory extraction failed",
            extra={"user_id": user_id},
        )
        retry_delay = 30 * (4 ** self.request.retries)
        raise self.retry(exc=exc, countdown=retry_delay)


async def _extract_and_store(
    user_id: str, conversation_text: str
) -> dict:
    """Internal async implementation."""
    from app.memory.extractor import extract_entities
    from app.memory.repository import write_entities

    entities = await extract_entities(conversation_text)

    if not entities:
        logger.info(
            "No entities extracted",
            extra={"user_id": user_id},
        )
        return {"extracted": 0}

    await write_entities(user_id, entities)

    logger.info(
        f"Extracted {len(entities)} entities",
        extra={"user_id": user_id},
    )
    return {"extracted": len(entities)}
