"""Celery application configuration."""

from celery import Celery

from app.config import settings

celery_app = Celery(
    "mika",
    broker=settings.celery_broker_url,
    backend=settings.celery_result_backend,
)

celery_app.conf.update(
    task_serializer="json",
    accept_content=["json"],
    result_serializer="json",
    timezone="UTC",
    enable_utc=True,
    task_track_started=True,
    task_acks_late=True,
    worker_prefetch_multiplier=1,
    task_routes={
        "app.worker.tasks.memory_extraction.*": {"queue": "memory"},
        "app.worker.tasks.follow_ups.*": {"queue": "scheduled"},
        "app.worker.tasks.briefings.*": {"queue": "scheduled"},
        "app.worker.tasks.maintenance.*": {"queue": "scheduled"},
    },
)

celery_app.autodiscover_tasks(
    [
        "app.worker.tasks.memory_extraction",
        "app.worker.tasks.follow_ups",
        "app.worker.tasks.briefings",
        "app.worker.tasks.maintenance",
    ]
)
