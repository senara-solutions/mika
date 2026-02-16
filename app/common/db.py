"""Async database engine and session factory."""

from sqlalchemy.ext.asyncio import (
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from app.config import settings

engine = create_async_engine(settings.database_url, echo=settings.debug)

async_session_factory = async_sessionmaker(
    engine, class_=AsyncSession, expire_on_commit=False
)
