"""Database engine and session factories."""

from sqlalchemy import create_engine
from sqlalchemy.ext.asyncio import (
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)
from sqlalchemy.orm import sessionmaker

from app.config import settings

engine = create_async_engine(settings.database_url, echo=settings.debug)

async_session_factory = async_sessionmaker(
    engine, class_=AsyncSession, expire_on_commit=False
)


def _sync_url() -> str:
    """Convert async database URL to sync for Celery workers."""
    return settings.database_url.replace(
        "postgresql+asyncpg", "postgresql+psycopg2"
    )


_sync_engine = None


def sync_engine():
    """Get or create sync engine (lazy, for Celery workers)."""
    global _sync_engine  # noqa: PLW0603
    if _sync_engine is None:
        _sync_engine = create_engine(_sync_url(), echo=settings.debug)
    return _sync_engine


def sync_session_factory():
    """Create a sync session factory for Celery workers."""
    return sessionmaker(bind=sync_engine())
