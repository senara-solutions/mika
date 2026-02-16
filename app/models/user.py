import uuid
from datetime import datetime

from sqlalchemy import Boolean, DateTime, String, func
from sqlalchemy.dialects.postgresql import UUID
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.models import Base


class User(Base):
    __tablename__ = "users"

    id: Mapped[uuid.UUID] = mapped_column(
        UUID(as_uuid=True), primary_key=True, default=uuid.uuid4
    )
    email: Mapped[str | None] = mapped_column(String(255), unique=True, nullable=True)
    encrypted_name: Mapped[bytes | None] = mapped_column(nullable=True)
    timezone: Mapped[str] = mapped_column(String(50), default="UTC")
    preferred_channel: Mapped[str] = mapped_column(String(20), default="telegram")
    onboarding_completed: Mapped[bool] = mapped_column(Boolean, default=False)
    onboarding_state: Mapped[str] = mapped_column(
        String(50), default="awaiting_consent"
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), server_default=func.now()
    )
    last_active_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True
    )

    channels: Mapped[list["UserChannel"]] = relationship(  # noqa: F821
        back_populates="user", cascade="all, delete-orphan"
    )
    conversations: Mapped[list["Conversation"]] = relationship(  # noqa: F821
        back_populates="user", cascade="all, delete-orphan"
    )
    consent: Mapped["UserConsent | None"] = relationship(  # noqa: F821
        back_populates="user", cascade="all, delete-orphan", uselist=False
    )
