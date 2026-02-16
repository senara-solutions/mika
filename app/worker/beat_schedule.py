"""Celery Beat periodic task configuration."""

from celery.schedules import crontab

from app.worker.celery_app import celery_app

celery_app.conf.beat_schedule = {
    "scan-follow-ups-every-4h": {
        "task": "app.worker.tasks.follow_ups.scan_pending_commitments",
        "schedule": crontab(minute=0, hour="*/4"),
    },
    "morning-briefing-dispatcher": {
        "task": "app.worker.tasks.briefings.morning_briefing_dispatcher",
        "schedule": crontab(minute="*/15"),
    },
    "weekly-conversation-summary": {
        "task": "app.worker.tasks.maintenance.summarize_old_conversations",
        "schedule": crontab(minute=0, hour=3, day_of_week=0),  # Sunday 3 AM
    },
}
